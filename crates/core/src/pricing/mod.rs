//! Pricing pipeline (spec §7.2–§7.4): alias BFS → price lookup → USD math.
//!
//! Lookup order: `price_overrides` (user) → the price book → unpriced. The
//! book is not "whichever feed ranks highest": nine sources are polled and a
//! model's price is what most of them agree on (see [`consensus`]), so one
//! feed's mistake cannot become a wrong bill. Never guess, never spread
//! across a family — unpriced is flagged, counted 0, and routed to the
//! overrides UI.

pub mod consensus;
mod feeds;
mod seed;

use crate::model::{CostSource, UsageEvent};
use crate::store::{Store, now_ms};
use anyhow::{Context, Result};
#[cfg(test)]
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// USD per **1M** tokens (models.dev convention; LiteLLM rows are converted
/// from $/token at import).
#[derive(Debug, Clone, Copy, Default)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    /// LiteLLM tiers (already converted to $/1M): long-context input,
    /// 1-hour cache write, batch discount factor.
    pub tier_above_200k_input: Option<f64>,
    pub tier_1h_cache_write: Option<f64>,
    pub tier_batch: Option<f64>,
}

#[derive(Debug)]
pub enum Resolution {
    /// (pricing_model, via) — via: override|exact|prefix|seed
    Priced(String, &'static str, Price),
    Unpriced,
}

/// The consensus half of a [`PriceBook`]: derived from the `prices` table only,
/// so it is shared between loads while that table is unchanged.
struct Consensus {
    /// normalized model key → the consensus price of its model. Every spelling
    /// any source used for a model points at the same price.
    map: HashMap<String, Price>,
    /// Spelling-insensitive twin of `map` (`claude-opus-4.6` ≡ `…-4-6`), for a
    /// tool that writes the id in a form no source does.
    by_canon: HashMap<String, Price>,
}

pub struct PriceBook {
    consensus: std::sync::Arc<Consensus>,
    /// user overrides, keyed by the RAW model name (pre-normalization) too.
    overrides: HashMap<String, Price>,
}

/// `(ledger path, fingerprint of prices, consensus)` of the last load. The
/// engine reloads the book on every scan tick (~every 30s); recomputing the
/// vote over ~10k quotes each time cost ~15ms for a table that changes twice a
/// day. In-memory databases are never cached (each is its own world).
type CachedConsensus = (String, (i64, i64, f64), std::sync::Arc<Consensus>);
static CONSENSUS: std::sync::Mutex<Option<CachedConsensus>> = std::sync::Mutex::new(None);

fn shared_consensus(store: &Store) -> Result<std::sync::Arc<Consensus>> {
    let path = store.conn().path().unwrap_or_default().to_string();
    let fingerprint: (i64, i64, f64) = store.conn().query_row(
        "SELECT COUNT(*), COALESCE(MAX(fetched_at), 0), COALESCE(SUM(input + output), 0) FROM prices",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if !path.is_empty()
        && let Some((p, f, c)) = CONSENSUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *p == path
        && *f == fingerprint
    {
        return Ok(c.clone());
    }
    let mut map = HashMap::new();
    let mut by_canon = HashMap::new();
    for g in consensus::groups(store.conn())? {
        let price = g.verdict.price;
        for k in g.keys {
            map.insert(k, price);
        }
        by_canon.insert(g.canon, price);
    }
    let c = std::sync::Arc::new(Consensus { map, by_canon });
    if !path.is_empty() {
        *CONSENSUS.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, fingerprint, c.clone()));
    }
    Ok(c)
}

impl PriceBook {
    pub fn empty() -> Self {
        Self {
            consensus: std::sync::Arc::new(Consensus {
                map: HashMap::new(),
                by_canon: HashMap::new(),
            }),
            overrides: HashMap::new(),
        }
    }

    pub fn load(store: &Store) -> Result<Self> {
        seed::ensure_seeded(store)?;
        // One vote per source per model; see `consensus` for how they are
        // weighed. (The bundled seed only speaks for models no live source
        // knows.)
        let consensus = shared_consensus(store)?;

        let mut overrides = HashMap::new();
        let mut st = store.conn().prepare(
            "SELECT model_key, input, output, cache_read, cache_write
             FROM price_overrides WHERE deleted=0",
        )?;
        for r in st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Price {
                    input: r.get::<_, f64>(1).unwrap_or(0.0),
                    output: r.get::<_, f64>(2).unwrap_or(0.0),
                    cache_read: r.get::<_, f64>(3).unwrap_or(0.0),
                    cache_write: r.get::<_, f64>(4).unwrap_or(0.0),
                    ..Default::default()
                },
            ))
        })? {
            let (k, v) = r?;
            overrides.insert(k, v);
        }
        Ok(Self {
            consensus,
            overrides,
        })
    }

    /// Fill `pricing_model`/`cost_usd`/`cost_source` on an event. Adapter-set
    /// costs (official/provider_reported) are never overwritten — only the
    /// pricing_model key is resolved for them.
    pub fn apply(&self, ev: &mut UsageEvent) {
        // 1) Response-side real model wins (spec §7.3); fall back to the
        //    client-requested alias only when no response model exists.
        let raw = ev.model.clone().or_else(|| ev.request_model.clone());
        let Some(raw) = raw else {
            if ev.cost_usd.is_none() && ev.cost_source.is_none() {
                ev.cost_usd = Some(0.0);
                ev.cost_source = Some(CostSource::Unpriced);
            }
            return;
        };
        match self.resolve(&raw, ev.request_model.as_deref(), ev.provider_id.as_deref()) {
            Resolution::Priced(key, via, price) => {
                ev.pricing_model = Some(key);
                if ev.cost_usd.is_none() {
                    ev.cost_usd = Some(compute(ev, &price));
                    ev.cost_source =
                        Some(if via == "prefix" || ev.model.as_deref() == Some("auto") {
                            CostSource::Estimated
                        } else {
                            CostSource::Computed
                        });
                }
            }
            Resolution::Unpriced => {
                if ev.cost_usd.is_none() {
                    ev.cost_usd = Some(0.0);
                    ev.cost_source = Some(CostSource::Unpriced);
                }
            }
        }
    }

    pub fn resolve(&self, raw: &str, request: Option<&str>, provider: Option<&str>) -> Resolution {
        // Overrides match on raw AND normalized keys — user's word is final.
        for key in [raw, &normalize_key(raw)] {
            if let Some(p) = self.overrides.get(key) {
                return Resolution::Priced((*key).to_string(), "override", *p);
            }
        }
        if let Some(req) = request {
            for key in [req, &normalize_key(req)] {
                if let Some(p) = self.overrides.get(key) {
                    return Resolution::Priced((*key).to_string(), "override", *p);
                }
            }
        }

        // BFS candidate queue (spec §7.2): progressive peels of vendor noise.
        for cand in candidates(raw) {
            if let Some(p) = self.lookup(&cand, provider) {
                let via = if cand == normalize_key(raw) {
                    "exact"
                } else {
                    "prefix"
                };
                return Resolution::Priced(cand, via, p);
            }
        }
        // Same for the request alias (e.g. `gpt-reserve` → real model may be
        // absent; alias itself can still resolve, e.g. `sonnet` family).
        if let Some(req) = request.filter(|r| *r != raw) {
            for cand in candidates(req) {
                if let Some(p) = self.lookup(&cand, provider) {
                    return Resolution::Priced(cand, "prefix", p);
                }
            }
        }
        Resolution::Unpriced
    }

    fn lookup(&self, cand: &str, _provider: Option<&str>) -> Option<Price> {
        self.consensus
            .map
            .get(cand)
            .or_else(|| self.consensus.by_canon.get(&consensus::canon_key(cand)))
            .copied()
    }
}

// ── Live price refresh (network) ──────────────────────────────────────────

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
/// LLM Pricing (llmpricing.dev): static JSON, no key, CC BY 4.0.
/// Models carry `reference` (official list) and `cheapest` (best host) quotes
/// in $/1M — we book `reference`, falling back to `cheapest` when absent.
const LLMPRICING_URL: &str = "https://llmpricing.dev/api/models.json";
/// Auto-refresh cadence for the UI path.
pub const PRICE_TTL_SECS: i64 = 12 * 3600;
/// app_state key recording the last refresh ATTEMPT (success or failure) so a
/// broken network does not re-hit the CDN on every scan tick.
const LAST_ATTEMPT_KEY: &str = "prices_last_attempt";

pub struct RefreshReport {
    /// Rows written per source tag (a source can take several downloads).
    pub sources: Vec<(&'static str, usize)>,
    /// One line per download that failed (network, or a malformed document).
    pub failed: Vec<String>,
    /// Formerly-unpriced events that gained a price after the refresh.
    pub repriced: u64,
}

impl RefreshReport {
    /// `models.dev 1997, openrouter 376, …` for logs and the CLI.
    pub fn summary(&self) -> String {
        self.sources
            .iter()
            .map(|(s, n)| format!("{s} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Newest `fetched_at` among live (non-seed) sources; `None` = never synced.
pub fn last_live_sync(store: &Store) -> Result<Option<i64>> {
    let t: Option<i64> = store.conn().query_row(
        "SELECT MAX(fetched_at) FROM prices WHERE source != 'seed'",
        [],
        |r| r.get(0),
    )?;
    Ok(t)
}

/// Staleness follows the last refresh *attempt* (not last success) — repeated
/// failures throttle to the TTL instead of retrying every scan. DBs that
/// predate attempt tracking fall back to `fetched_at`.
pub fn prices_stale(store: &Store) -> Result<bool> {
    let last = store
        .get_state(LAST_ATTEMPT_KEY)?
        .and_then(|s| s.parse::<i64>().ok())
        .or(last_live_sync(store)?);
    Ok(last.is_none_or(|t| (now_ms() - t) / 1000 > PRICE_TTL_SECS))
}

fn http_get(url: &str) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let mut resp = agent
        .get(url)
        .header("User-Agent", "GlobalTokenTracker")
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.body_mut().read_to_string()?)
}

type Import = Box<dyn Fn(&rusqlite::Connection, &str, i64) -> Result<usize>>;

struct Feed {
    url: String,
    tag: &'static str,
    import: Import,
}

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/models";
const VERCEL_URL: &str = "https://ai-gateway.vercel.sh/v1/models";
const HELICONE_URL: &str = "https://www.helicone.ai/api/llm-costs";
const LANGFUSE_URL: &str = "https://raw.githubusercontent.com/langfuse/langfuse/main/worker/src/constants/default-model-prices.json";
const LLM_PRICES_URL: &str = "https://www.llm-prices.com/current-v1.json";
const PORTKEY_BASE: &str = "https://raw.githubusercontent.com/Portkey-AI/models/main/pricing";
/// Portkey publishes one file per *host*; these are the model makers' own
/// (first-party list prices, MIT-licensed). `qwen`/`qwq` narrows DashScope,
/// which also resells Kimi and GLM at its own prices. Z.ai's international
/// file is used, not the CNY-list `zhipu` one, so two files never fight over
/// one id.
const PORTKEY_FILES: &[(&str, &[&str])] = &[
    ("anthropic", &[]),
    ("openai", &[]),
    ("google", &[]),
    ("x-ai", &[]),
    ("mistral-ai", &[]),
    ("deepseek", &[]),
    ("moonshot", &[]),
    ("z-ai", &[]),
    ("minimax", &[]),
    ("cohere", &[]),
    ("perplexity-ai", &[]),
    ("dashscope", &["qwen", "qwq"]),
];

fn feeds() -> Vec<Feed> {
    let mut v = vec![
        Feed {
            url: MODELS_DEV_URL.into(),
            tag: "models.dev",
            import: Box::new(feeds::import_models_dev),
        },
        Feed {
            url: LITELLM_URL.into(),
            tag: "litellm",
            import: Box::new(feeds::import_litellm),
        },
        Feed {
            url: LLMPRICING_URL.into(),
            tag: "llmpricing",
            import: Box::new(feeds::import_llmpricing),
        },
        Feed {
            url: OPENROUTER_URL.into(),
            tag: "openrouter",
            import: Box::new(feeds::import_openrouter),
        },
        Feed {
            url: VERCEL_URL.into(),
            tag: "vercel",
            import: Box::new(feeds::import_vercel),
        },
        Feed {
            url: HELICONE_URL.into(),
            tag: "helicone",
            import: Box::new(feeds::import_helicone),
        },
        Feed {
            url: LANGFUSE_URL.into(),
            tag: "langfuse",
            import: Box::new(feeds::import_langfuse),
        },
        Feed {
            url: LLM_PRICES_URL.into(),
            tag: "llm-prices",
            import: Box::new(feeds::import_llm_prices),
        },
    ];
    for (provider, prefixes) in PORTKEY_FILES {
        v.push(Feed {
            url: format!("{PORTKEY_BASE}/{provider}.json"),
            tag: "portkey",
            import: Box::new(move |c, body, now| feeds::import_portkey(c, body, now, prefixes)),
        });
    }
    v
}

/// Give up on the rest of a refresh after this many downloads in a row failed
/// to *connect*: the network is down, and every remaining feed would burn its
/// full timeout.
const MAX_CONSECUTIVE_FETCH_FAILURES: usize = 4;

/// Fetch every price feed, upsert into `prices`, then reprice any event still
/// marked `unpriced` so newly-covered models gain estimates. Feeds are
/// independent — one outage or a malformed document costs that feed only, and
/// the book keeps that source's previous rows. All failing → Err, book
/// untouched (seed fallback). `prices_last_attempt` is stamped up front so a
/// persistent outage throttles to `PRICE_TTL_SECS` rather than every scan.
pub fn refresh(store: &Store) -> Result<RefreshReport> {
    let now = now_ms();
    store.set_state(LAST_ATTEMPT_KEY, &now.to_string())?;
    let mut report = RefreshReport {
        sources: Vec::new(),
        failed: Vec::new(),
        repriced: 0,
    };
    // Feeds are handled strictly one at a time — download, parse into the few
    // typed fields we need, write, drop — so the transient heap is one
    // document's worth instead of every full JSON DOM at once (was a 50–80MB
    // spike at every launch).
    let mut succeeded = 0;
    let mut fetch_failures = 0;
    for feed in feeds() {
        let body = match http_get(&feed.url) {
            Ok(b) => {
                fetch_failures = 0;
                b
            }
            Err(e) => {
                report.failed.push(format!("{} fetch: {e:#}", feed.tag));
                fetch_failures += 1;
                if fetch_failures >= MAX_CONSECUTIVE_FETCH_FAILURES {
                    report.failed.push("network unreachable — stopped".into());
                    break;
                }
                continue;
            }
        };
        // Its own transaction: nothing is written unless the whole document
        // parsed, and the write lock is never held across a download.
        let written = (|| -> Result<usize> {
            let tx = store.conn().unchecked_transaction()?;
            let n = (feed.import)(&tx, &body, now)?;
            tx.commit()?;
            Ok(n)
        })();
        drop(body);
        match written {
            Ok(n) => {
                succeeded += 1;
                match report.sources.iter_mut().find(|(t, _)| *t == feed.tag) {
                    Some((_, total)) => *total += n,
                    None => report.sources.push((feed.tag, n)),
                }
            }
            Err(e) => report.failed.push(format!("{} parse: {e:#}", feed.tag)),
        }
    }
    if succeeded == 0 {
        anyhow::bail!("all price sources failed: {}", report.failed.join("; "));
    }

    // Re-price events that were unpriced at ingest — a grown price book may
    // now cover them.
    let book = PriceBook::load(store)?;
    report.repriced = reprice_unpriced(store, &book)?;
    Ok(report)
}

/// llmpricing.dev importer: {models:[{id, reference:{provider,input,output,
/// cacheRead,official}, cheapest:{...}}]} — already $/1M, no conversion.
/// Books the official `reference` quote; `cheapest` only fills gaps so a
/// bargain host never understates the user's actual provider cost. The
/// source exposes no cache-write price → column stays NULL (honest absence,
/// not a guessed multiplier).
#[cfg(test)]
fn upsert_llmpricing(conn: &rusqlite::Connection, lp: &Value, now: i64) -> Result<usize> {
    let mut st = conn.prepare(
        "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
         VALUES ('llmpricing', ?1, ?2, ?3, ?4, NULL, 'llmpricing', ?5)",
    )?;
    let mut n = 0usize;
    for m in lp["models"].as_array().into_iter().flatten() {
        let Some(id) = m["id"].as_str() else { continue };
        let q = if m["reference"]["input"].is_number() {
            &m["reference"]
        } else {
            &m["cheapest"]
        };
        if !q["input"].is_number() && !q["output"].is_number() {
            continue; // neither quote usable
        }
        st.execute(rusqlite::params![
            normalize_key(id),
            q["input"].as_f64().unwrap_or(0.0),
            q["output"].as_f64().unwrap_or(0.0),
            q["cacheRead"].as_f64().unwrap_or(0.0),
            now
        ])?;
        n += 1;
    }
    Ok(n)
}

/// Re-resolve `unpriced` events against the current book. Bounded by the
/// unpriced count; one transaction.
pub fn reprice_unpriced(store: &Store, book: &PriceBook) -> Result<u64> {
    let mut st = store.conn().prepare(
        "SELECT rowid, model, request_model, provider_id,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens
         FROM usage_events WHERE cost_source='unpriced'",
    )?;
    let rows: Vec<(i64, UsageEvent)> = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                UsageEvent {
                    model: r.get(1)?,
                    request_model: r.get(2)?,
                    provider_id: r.get(3)?,
                    input_tokens: r.get::<_, i64>(4)? as u64,
                    output_tokens: r.get::<_, i64>(5)? as u64,
                    reasoning_tokens: r.get::<_, i64>(6)? as u64,
                    cache_read_tokens: r.get::<_, i64>(7)? as u64,
                    cache_write_5m_tokens: r.get::<_, i64>(8)? as u64,
                    cache_write_1h_tokens: r.get::<_, i64>(9)? as u64,
                    ..Default::default()
                },
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(st);
    let tx = store.conn().unchecked_transaction()?;
    let mut n = 0u64;
    {
        let mut up = tx.prepare(
            "UPDATE usage_events SET cost_usd=?1, cost_source=?2, pricing_model=?3 WHERE rowid=?4",
        )?;
        for (rowid, ev) in &rows {
            if let Resolution::Priced(key, via, p) = book.resolve(
                ev.model.as_deref().unwrap_or(""),
                ev.request_model.as_deref(),
                ev.provider_id.as_deref(),
            ) {
                up.execute(rusqlite::params![
                    compute(ev, &p),
                    if via == "prefix" || ev.model.as_deref() == Some("auto") {
                        "estimated"
                    } else {
                        "computed"
                    },
                    key,
                    rowid
                ])?;
                n += 1;
            }
        }
    }
    tx.commit()?;
    Ok(n)
}

/// Stage 1 normalization (spec §7.2.1): last `/` segment, drop `:` suffix,
/// `@`→`-`, lowercase, strip `[1m]` context-tag.
pub fn normalize_key(raw: &str) -> String {
    let tail = raw.rsplit_once('/').map(|(_, t)| t).unwrap_or(raw);
    let no_tag = tail.split(':').next().unwrap_or(tail);
    let s = no_tag.replace('@', "-").to_lowercase();
    s.strip_suffix("[1m]").map_or(s.clone(), |s| s.to_string())
}

/// Peel prefixes/suffixes progressively (spec §7.2.2). Yields most-specific
/// first; exact-match phase covers all of these before prefix matching.
pub fn candidates(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push = |s: String, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if !s.is_empty() && seen.insert(s.clone()) {
            out.push(s);
        }
    };
    let base = normalize_key(raw);
    push(base.clone(), &mut out, &mut seen);

    let mut queue = std::collections::VecDeque::from([base]);
    while let Some(c) = queue.pop_front() {
        // vendor prefixes: openai./anthropic./moonshot./bedrock./global.
        for p in ["openai.", "anthropic.", "moonshot.", "bedrock.", "global."] {
            if let Some(rest) = c.strip_prefix(p) {
                push(rest.to_string(), &mut out, &mut seen);
                queue.push_back(rest.to_string());
            }
        }
        // `rfind("claude-")` — bedrock-style `us.anthropic.claude-...` tails.
        if let Some(i) = c.rfind("claude-").filter(|&i| i > 0) {
            let rest = c[i..].to_string();
            push(rest.clone(), &mut out, &mut seen);
            queue.push_back(rest);
        }
        // `-v<digits>` suffix.
        if let Some(stripped) = strip_num_suffix(&c, "-v") {
            push(stripped.clone(), &mut out, &mut seen);
            queue.push_back(stripped);
        }
        // `-YYYYMMDD` date suffix.
        if let Some((head, date)) = c.rsplit_once('-')
            && date.len() == 8
            && date.bytes().all(|b| b.is_ascii_digit())
        {
            push(head.to_string(), &mut out, &mut seen);
            queue.push_back(head.to_string());
        }
        // reasoning-effort suffixes.
        for suf in ["-minimal", "-low", "-medium", "-high", "-xhigh"] {
            if let Some(head) = c.strip_suffix(suf) {
                push(head.to_string(), &mut out, &mut seen);
                queue.push_back(head.to_string());
            }
        }
    }
    out
}

fn strip_num_suffix(c: &str, marker: &str) -> Option<String> {
    let (head, tail) = c.rsplit_once(marker)?;
    (!tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())).then(|| head.to_string())
}

/// USD for one event (spec §7.4): 4 components; LiteLLM tiers trigger on real
/// context size; missing columns fall back to the fixed multipliers
/// (cache_read 0.1×input, 5m write 1.25×, 1h write 2×).
pub fn compute(ev: &UsageEvent, p: &Price) -> f64 {
    let context = ev.input_tokens + ev.cache_read_tokens + ev.cache_write_total();
    let in_price = if context > 200_000 {
        p.tier_above_200k_input.unwrap_or(p.input)
    } else {
        p.input
    };
    let cw_1h = p.tier_1h_cache_write.unwrap_or_else(|| {
        if p.cache_write > 0.0 {
            p.cache_write * 2.0 / 1.25
        } else {
            p.input * 2.0
        }
    });
    let cw_5m = if p.cache_write > 0.0 {
        p.cache_write
    } else {
        p.input * 1.25
    };
    let cr = if p.cache_read > 0.0 {
        p.cache_read
    } else {
        p.input * 0.1
    };

    let usd = (ev.input_tokens as f64 * in_price
        + ev.output_tokens as f64 * p.output
        + ev.cache_read_tokens as f64 * cr
        + ev.cache_write_5m_tokens as f64 * cw_5m
        + ev.cache_write_1h_tokens as f64 * cw_1h)
        / 1_000_000.0;
    (usd * 1e9).round() / 1e9 // nanodollar rounding, keeps sums stable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_examples() {
        assert_eq!(normalize_key("stealth/custom-alpha"), "custom-alpha");
        assert_eq!(
            normalize_key("anthropic/claude-opus-5:free"),
            "claude-opus-5"
        );
        assert_eq!(
            normalize_key("Claude-Opus-5@20260101"),
            "claude-opus-5-20260101"
        );
        assert_eq!(normalize_key("kimi-k3[1m]"), "kimi-k3");
    }

    #[test]
    fn candidates_peel() {
        let c = candidates("bedrock/global.anthropic.claude-opus-5-5-20260901-high");
        assert!(c.contains(&"claude-opus-5-5".to_string()), "{c:?}");
    }

    #[test]
    fn math_uses_per_1m() {
        let p = Price {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
            ..Default::default()
        };
        let ev = UsageEvent {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Default::default()
        };
        assert!((compute(&ev, &p) - 18.0).abs() < 1e-6);
    }

    fn unpriced_ev(key: &str, model: &str) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            app: crate::model::apps::CLAUDE.into(),
            model: Some(model.into()),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: Some(0.0),
            cost_source: Some(CostSource::Unpriced),
            ..Default::default()
        }
    }

    fn put_price(s: &Store, source: &str, model: &str, input: f64, output: f64) {
        s.conn()
            .execute(
                "INSERT OR REPLACE INTO prices(provider, model_id, input, output, source, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?1, ?5)",
                rusqlite::params![source, model, input, output, now_ms()],
            )
            .unwrap();
    }

    #[test]
    fn reprice_backfills_unpriced_events() {
        let s = Store::open_memory().unwrap();
        s.upsert_event(&unpriced_ev("u1", "brand-new-model"))
            .unwrap();
        s.upsert_event(&unpriced_ev("u2", "still-unknown")).unwrap();
        // A new live price arrives for u1 only.
        put_price(&s, "models.dev", "brand-new-model", 2.0, 10.0);
        let book = PriceBook::load(&s).unwrap();
        assert_eq!(reprice_unpriced(&s, &book).unwrap(), 1);
        let (cost, src, pm): (f64, String, String) = s
            .conn()
            .query_row(
                "SELECT cost_usd, cost_source, pricing_model FROM usage_events WHERE dedup_key='u1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!((cost - 0.007).abs() < 1e-9, "{cost}"); // 1000*2 + 500*10 (per 1M)
        assert_eq!(src, "computed");
        assert_eq!(pm, "brand-new-model");
        // u2 stays unpriced — never guess.
        let src2: String = s
            .conn()
            .query_row(
                "SELECT cost_source FROM usage_events WHERE dedup_key='u2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(src2, "unpriced");
    }

    #[test]
    fn override_beats_live_and_live_beats_seed() {
        let s = Store::open_memory().unwrap();
        put_price(&s, "seed", "m-x", 1.0, 1.0);
        put_price(&s, "litellm", "m-x", 3.0, 3.0);
        put_price(&s, "models.dev", "m-x", 5.0, 5.0);
        s.conn()
            .execute(
                "INSERT INTO price_overrides(model_key, input, output, cache_read, cache_write, updated_at)
                 VALUES ('m-y', 9.0, 9.0, 0, 0, 0)",
                [],
            )
            .unwrap();
        put_price(&s, "models.dev", "m-y", 1.0, 1.0);
        let book = PriceBook::load(&s).unwrap();
        match book.resolve("m-x", None, None) {
            Resolution::Priced(_, _, p) => assert_eq!(p.input, 5.0), // dev over seed+litellm
            _ => panic!("m-x should be priced"),
        }
        match book.resolve("m-y", None, None) {
            Resolution::Priced(_, via, p) => {
                assert_eq!(via, "override");
                assert_eq!(p.input, 9.0);
            }
            _ => panic!("m-y should hit override"),
        }
    }

    #[test]
    fn staleness_flags_seed_only_book() {
        let s = Store::open_memory().unwrap();
        assert!(prices_stale(&s).unwrap()); // no live rows
        put_price(&s, "models.dev", "m-z", 1.0, 1.0);
        assert!(!prices_stale(&s).unwrap());
        // Repricing is idempotent: second run finds nothing new.
        s.upsert_event(&unpriced_ev("i1", "nope-model")).unwrap();
        let book = PriceBook::load(&s).unwrap();
        assert_eq!(reprice_unpriced(&s, &book).unwrap(), 0);
    }

    fn llmpricing_fixture() -> Value {
        serde_json::json!({"meta": {"models": 3}, "models": [
            {"id": "acme/glm-9",
             "reference": {"provider": "acme", "input": 1.4, "output": 4.4,
                            "cacheRead": 0.26, "official": true},
             "cheapest": {"provider": "crof", "input": 0.3, "output": 1.05,
                           "cacheRead": 0.05}},
            {"id": "lab/no-ref",
             "cheapest": {"provider": "x", "input": 0.5, "output": 2.0,
                           "cacheRead": null}},
            {"id": "lab/empty",
             "reference": {"provider": "y"},
             "cheapest": {"provider": "z"}}
        ]})
    }

    #[test]
    fn llmpricing_books_reference_then_cheapest() {
        let s = Store::open_memory().unwrap();
        let n = upsert_llmpricing(s.conn(), &llmpricing_fixture(), now_ms()).unwrap();
        assert_eq!(n, 2); // lab/empty skipped — no usable quote
        // reference wins over cheapest (official list price).
        let (inp, outp, cr, cw): (f64, f64, f64, Option<f64>) = s
            .conn()
            .query_row(
                "SELECT input, output, cache_read, cache_write FROM prices
                 WHERE source='llmpricing' AND model_id='glm-9'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((inp, outp, cr), (1.4, 4.4, 0.26));
        assert_eq!(cw, None); // source has no cache-write field — honest NULL
        // cheapest fills where reference lacks a numeric input.
        let inp2: f64 = s
            .conn()
            .query_row(
                "SELECT input FROM prices WHERE source='llmpricing' AND model_id='no-ref'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inp2, 0.5);
    }

    #[test]
    fn the_book_bills_the_consensus_not_the_highest_ranked_feed() {
        let s = Store::open_memory().unwrap();
        // Two feeds say 1.4/4.4, the (formerly top-ranked) llmpricing row says
        // $9 — fixed precedence would have billed the outlier.
        put_price(&s, "models.dev", "glm-9", 1.4, 4.4);
        put_price(&s, "openrouter", "glm-9", 1.4, 4.4);
        put_price(&s, "llmpricing", "glm-9", 9.0, 9.0);
        let book = PriceBook::load(&s).unwrap();
        let Resolution::Priced(_, _, p) = book.resolve("glm-9", None, None) else {
            panic!("priced");
        };
        assert_eq!((p.input, p.output), (1.4, 4.4));
        // user override still outranks every source.
        s.conn()
            .execute(
                "INSERT INTO price_overrides(model_key, input, output, updated_at)
                 VALUES ('glm-9', 42.0, 42.0, 0)",
                [],
            )
            .unwrap();
        let book = PriceBook::load(&s).unwrap();
        let Resolution::Priced(_, via, p) = book.resolve("glm-9", None, None) else {
            panic!("priced");
        };
        assert_eq!((via, p.input), ("override", 42.0));
    }

    #[test]
    fn every_spelling_of_a_model_resolves_to_the_same_price() {
        let s = Store::open_memory().unwrap();
        put_price(&s, "openrouter", "claude-opus-4.6", 5.0, 25.0);
        put_price(&s, "vercel", "claude-opus-4.6", 5.0, 25.0);
        // The bundled seed once carried a zero-price hyphen twin of a dotted id.
        put_price(&s, "seed", "claude-opus-4-6", 0.0, 0.0);
        let book = PriceBook::load(&s).unwrap();
        for spelling in [
            "claude-opus-4.6",
            "claude-opus-4-6",
            "Claude-Opus-4.6",
            "anthropic/claude-opus-4-6",
        ] {
            match book.resolve(spelling, None, None) {
                Resolution::Priced(_, via, p) => {
                    assert_eq!((p.input, p.output), (5.0, 25.0), "{spelling}");
                    assert_ne!(via, "override");
                }
                Resolution::Unpriced => panic!("{spelling} unpriced"),
            }
        }
        // A spelling no source ever used still finds it by canonical form.
        assert!(matches!(
            book.resolve("claude-opus-4.6-20260101", None, None),
            Resolution::Priced(..)
        ));
    }

    #[test]
    fn price_rows_show_the_consensus_and_who_agreed() {
        let s = Store::open_memory().unwrap();
        put_price(&s, "portkey", "gpt-5", 1.25, 10.0);
        put_price(&s, "langfuse", "gpt-5", 1.25, 10.0);
        put_price(&s, "litellm", "gpt-5", 0.625, 5.0);
        let rows = s.price_rows(10).unwrap();
        let r = rows.iter().find(|r| r.model == "gpt-5").unwrap();
        assert_eq!((r.input, r.output), (1.25, 10.0));
        assert_eq!((r.agree, r.total), (2, 3));
        assert!(r.disputed());
        assert_eq!(r.source, "portkey");
        assert_eq!(r.quotes.len(), 3);
        // The page shows exactly what billing uses.
        let book = PriceBook::load(&s).unwrap();
        let Resolution::Priced(_, _, p) = book.resolve("gpt-5", None, None) else {
            panic!("priced");
        };
        assert_eq!((p.input, p.output), (r.input, r.output));
    }

    #[test]
    fn price_search_ignores_case_and_separators_and_ranks_prefixes_first() {
        use crate::store::filter_prices;
        let s = Store::open_memory().unwrap();
        for src in ["openrouter", "vercel"] {
            put_price(&s, src, "claude-opus-4.6", 5.0, 25.0);
        }
        put_price(&s, "litellm", "claude-opus-4-6", 5.0, 25.0);
        put_price(&s, "openrouter", "claude-sonnet-4.6", 3.0, 15.0);
        put_price(&s, "portkey", "gpt-5-mini", 0.25, 2.0);
        put_price(&s, "portkey", "mini-lm", 0.1, 0.1);
        // gpt-5: the sources disagree.
        put_price(&s, "portkey", "gpt-5", 1.25, 10.0);
        put_price(&s, "langfuse", "gpt-5", 1.25, 10.0);
        put_price(&s, "litellm", "gpt-5", 0.625, 5.0);
        let rows = s.price_rows(100).unwrap();
        let names = |q: &str, disputed: bool| -> Vec<String> {
            filter_prices(&rows, q, disputed)
                .into_iter()
                .map(|r| r.model)
                .collect()
        };
        // Any separator style, any case, either spelling of the version.
        for q in ["opus 4.6", "OPUS-4-6", "claude opus 4-6", "  opus,4.6 "] {
            assert_eq!(names(q, false).len(), 1, "{q}");
            assert!(names(q, false)[0].starts_with("claude-opus-4"), "{q}");
        }
        // Words are ANDed.
        assert_eq!(names("gpt mini", false), ["gpt-5-mini"]);
        assert!(names("gpt zzz", false).is_empty());
        // Ids that start with the query come before ids that merely contain it.
        assert_eq!(names("mini", false), ["mini-lm", "gpt-5-mini"]);
        // Nothing typed = everything, alphabetical.
        assert_eq!(names("", false).len(), rows.len());
        assert_eq!(names("   ", false).len(), rows.len());
        // The dispute filter composes with the search.
        assert_eq!(names("", true), ["gpt-5"]);
        assert!(names("opus", true).is_empty());
    }

    #[test]
    fn stale_gate_uses_attempt_stamp_12h_ttl() {
        let s = Store::open_memory().unwrap();
        // Fresh attempt (e.g. a failed fetch) → not stale for the next 12h.
        s.set_state(LAST_ATTEMPT_KEY, &now_ms().to_string())
            .unwrap();
        assert!(!prices_stale(&s).unwrap());
        // Older than 12h → stale again.
        let old = now_ms() - (PRICE_TTL_SECS + 60) * 1000;
        s.set_state(LAST_ATTEMPT_KEY, &old.to_string()).unwrap();
        assert!(prices_stale(&s).unwrap());
        // No attempt stamp → falls back to live fetched_at.
        s.conn()
            .execute("DELETE FROM app_state WHERE key=?1", [LAST_ATTEMPT_KEY])
            .unwrap();
        put_price(&s, "models.dev", "m-z", 1.0, 1.0);
        assert!(!prices_stale(&s).unwrap());
    }
}
