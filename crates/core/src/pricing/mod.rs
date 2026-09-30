//! Pricing pipeline (spec §7.2–§7.4): alias BFS → price lookup → USD math.
//!
//! Lookup order: `price_overrides` (user) → the price book → unpriced. The
//! book is not "whichever feed ranks highest": nine sources are polled and a
//! model's price is what most of them agree on (see [`consensus`]), so one
//! feed's mistake cannot become a wrong bill. Never guess, never spread
//! across a family — unpriced is flagged, counted 0, and routed to the
//! overrides UI. The knobs (feeds, trust, peels, routing placeholders,
//! aliases, long-context threshold) are data: see [`rules`].

pub mod consensus;
mod feeds;
pub mod rules;
mod seed;

use crate::model::{CostSource, UsageEvent};
use crate::store::{Store, now_ms};
use anyhow::{Context, Result};
use rules::Rules;
#[cfg(test)]
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Bump when the pricing function itself changes in a way that alters
/// already-booked costs; together with the rules fingerprint it decides
/// whether `reprice_if_rules_changed` has work to do.
const PRICING_LOGIC_VERSION: u32 = 2;
/// app_state key: `<logic version>:<rules fingerprint>` of the last full repass.
const APPLIED_KEY: &str = "pricing_applied";
/// Long-context tier threshold used by [`compute`] (rules carry the live one).
const DEFAULT_LONG_CONTEXT: u64 = 200_000;

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
    /// Long-context output and cache-read tiers.
    pub tier_above_200k_output: Option<f64>,
    pub tier_above_200k_cache_read: Option<f64>,
}

#[derive(Debug)]
pub enum Resolution {
    /// (pricing_model, via) — via: override|alias|exact|prefix|seed
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
    rules: Rules,
}

/// `(ledger path, fingerprint of prices + rules, consensus)` of the last load.
/// The engine reloads the book on every scan tick (~every 30s); recomputing
/// the vote over ~10k quotes each time cost ~15ms for a table that changes
/// twice a day. In-memory databases are never cached (each is its own world).
type CacheKey = ((i64, i64, f64), String);
type CachedConsensus = (String, CacheKey, std::sync::Arc<Consensus>);
static CONSENSUS: std::sync::Mutex<Option<CachedConsensus>> = std::sync::Mutex::new(None);

fn shared_consensus(store: &Store, rules: &Rules) -> Result<std::sync::Arc<Consensus>> {
    let path = store.conn().path().unwrap_or_default().to_string();
    let table: (i64, i64, f64) = store.conn().query_row(
        "SELECT COUNT(*), COALESCE(MAX(fetched_at), 0), COALESCE(SUM(input + output), 0) FROM prices",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let fingerprint: CacheKey = (table, rules.fingerprint());
    if !path.is_empty()
        && let Some((p, f, c)) = CONSENSUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *p == path
        && *f == fingerprint
    {
        return Ok(c.clone());
    }
    let mut map = HashMap::new();
    let mut by_canon = HashMap::new();
    for g in consensus::groups(store.conn(), rules)? {
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
            rules: Rules::bundled(),
        }
    }

    pub fn load(store: &Store) -> Result<Self> {
        seed::ensure_seeded(store)?;
        let rules = rules::current(store);
        // One vote per source per model; see `consensus` for how they are
        // weighed. (The bundled seed only speaks for models no live source
        // knows.)
        let consensus = shared_consensus(store, &rules)?;

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
            rules,
        })
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    /// The one pricing function: `(pricing_model, cost_usd, cost_source)` for
    /// an event as if it carried no cost yet. Ingest (`apply`) and every
    /// repricing path go through it, so they cannot diverge. Estimated iff the
    /// match needed a peel (`prefix`).
    pub fn price(&self, ev: &UsageEvent) -> (Option<String>, f64, CostSource) {
        // 1) Response-side real model wins (spec §7.3); fall back to the
        //    client-requested alias only when no response model exists.
        let Some(raw) = ev.model.as_deref().or(ev.request_model.as_deref()) else {
            return (None, 0.0, CostSource::Unpriced);
        };
        match self.resolve(raw, ev.request_model.as_deref(), ev.provider_id.as_deref()) {
            Resolution::Priced(key, via, price) => (
                Some(key),
                compute_at(ev, &price, self.rules.long_context_threshold),
                if via == "prefix" {
                    CostSource::Estimated
                } else {
                    CostSource::Computed
                },
            ),
            Resolution::Unpriced => (None, 0.0, CostSource::Unpriced),
        }
    }

    /// Fill `pricing_model`/`cost_usd`/`cost_source` on an event. Adapter-set
    /// costs (official/provider_reported) are never overwritten — only the
    /// pricing_model key is resolved for them.
    pub fn apply(&self, ev: &mut UsageEvent) {
        let (pricing_model, cost, source) = self.price(ev);
        if pricing_model.is_some() {
            ev.pricing_model = pricing_model;
        }
        if ev.cost_usd.is_none() {
            ev.cost_usd = Some(cost);
            ev.cost_source = Some(source);
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

        // Rule aliases: an explicit "this name is that model".
        for name in std::iter::once(raw).chain(request) {
            if let Some(target) = self.alias_target(name)
                && let Some(p) = self.lookup(&target, provider)
            {
                return Resolution::Priced(target, "alias", p);
            }
        }

        // A router placeholder names no model: price the request alias or
        // nothing — never a namesake row in some feed.
        if !self.is_routing(raw) {
            // BFS candidate queue (spec §7.2): progressive peels of vendor noise.
            let primary = primary_keys(raw);
            for cand in candidates_with(raw, &self.rules) {
                if let Some(p) = self.lookup(&cand, provider) {
                    let via = if primary.contains(&cand) {
                        "exact"
                    } else {
                        "prefix"
                    };
                    return Resolution::Priced(cand, via, p);
                }
            }
        }
        // Same for the request alias (e.g. `gpt-reserve` → real model may be
        // absent; alias itself can still resolve, e.g. `sonnet` family).
        if let Some(req) = request.filter(|r| *r != raw && !self.is_routing(r)) {
            for cand in candidates_with(req, &self.rules) {
                if let Some(p) = self.lookup(&cand, provider) {
                    return Resolution::Priced(cand, "prefix", p);
                }
            }
        }
        Resolution::Unpriced
    }

    fn is_routing(&self, model: &str) -> bool {
        let key = normalize_key(model);
        self.rules
            .routing_models
            .iter()
            .any(|r| normalize_key(r) == key)
    }

    fn alias_target(&self, model: &str) -> Option<String> {
        let key = normalize_key(model);
        self.rules
            .aliases
            .iter()
            .find(|(from, _)| normalize_key(from) == key)
            .map(|(_, to)| normalize_key(to))
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

/// Auto-refresh cadence for the UI path.
pub const PRICE_TTL_SECS: i64 = 12 * 3600;
/// app_state key recording the last refresh ATTEMPT (success or failure) so a
/// broken network does not re-hit the CDN on every scan tick.
const LAST_ATTEMPT_KEY: &str = "prices_last_attempt";

pub struct RefreshReport {
    /// Rows written per source tag (a source can take several downloads).
    pub sources: Vec<(String, usize)>,
    /// One line per download that failed (network, or a malformed document).
    pub failed: Vec<String>,
    /// Events whose booked price changed after the refresh (formerly
    /// unpriced ones that gained a price, plus a rules repass).
    pub repriced: u64,
    /// The pricing rules or logic changed and every book-priced row was
    /// re-priced (rollups rebuilt) — the UI must rebuild its aggregates.
    pub rules_repass: bool,
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

fn http_get_secs(url: &str, secs: u64) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .build()
        .new_agent();
    let mut resp = agent
        .get(url)
        .header("User-Agent", "GlobalTokenTracker")
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.body_mut().read_to_string()?)
}

fn http_get(url: &str) -> Result<String> {
    http_get_secs(url, 30)
}

type Import = Box<dyn Fn(&rusqlite::Connection, &str, i64) -> Result<usize>>;

/// The importer for a feed's `format`; `None` for a format this build does not
/// know (a newer rules document may list one).
fn importer(spec: &rules::FeedSpec) -> Option<Import> {
    Some(match spec.format.as_str() {
        "models_dev" => Box::new(feeds::import_models_dev),
        "litellm" => Box::new(feeds::import_litellm),
        "llmpricing" => Box::new(feeds::import_llmpricing),
        "openrouter" => Box::new(feeds::import_openrouter),
        "vercel" => Box::new(feeds::import_vercel),
        "helicone" => Box::new(feeds::import_helicone),
        "langfuse" => Box::new(feeds::import_langfuse),
        "llm_prices" => Box::new(feeds::import_llm_prices),
        "portkey" => {
            let prefixes = spec.prefixes.clone();
            Box::new(move |c, body, now| {
                let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
                feeds::import_portkey(c, body, now, &refs)
            })
        }
        _ => return None,
    })
}

/// Give up on the rest of a refresh after this many downloads in a row failed
/// to *connect*: the network is down, and every remaining feed would burn its
/// full timeout.
const MAX_CONSECUTIVE_FETCH_FAILURES: usize = 4;

/// Fetch the pricing rules and every price feed they list, upsert into
/// `prices`, then reprice events: still-`unpriced` ones (a grown book may now
/// cover them) and, when the rules or pricing logic changed, every
/// book-priced row. Feeds are independent — one outage or a malformed
/// document costs that feed only, and the book keeps that source's previous
/// rows. All feeds failing and no rules repass → Err, book untouched (seed
/// fallback). `prices_last_attempt` is stamped up front so a persistent
/// outage throttles to `PRICE_TTL_SECS` rather than every scan.
pub fn refresh(store: &Store) -> Result<RefreshReport> {
    let now = now_ms();
    store.set_state(LAST_ATTEMPT_KEY, &now.to_string())?;
    let mut report = RefreshReport {
        sources: Vec::new(),
        failed: Vec::new(),
        repriced: 0,
        rules_repass: false,
    };
    // Rules first: they say which feeds to read. A bad or unreachable
    // document keeps the previous rules.
    if let Some(url) = rules::remote_url()
        && let Err(e) = http_get_secs(&url, 10).and_then(|raw| rules::store_remote(store, &raw))
    {
        report.failed.push(format!("rules: {e:#}"));
    }
    let rules = rules::current(store);
    // Feeds are handled strictly one at a time — download, parse into the few
    // typed fields we need, write, drop — so the transient heap is one
    // document's worth instead of every full JSON DOM at once (was a 50–80MB
    // spike at every launch).
    let mut succeeded = 0;
    let mut fetch_failures = 0;
    for feed in &rules.feeds {
        let Some(import) = importer(feed) else {
            report.failed.push(format!("{} unknown format", feed.tag));
            continue;
        };
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
            let n = import(&tx, &body, now)?;
            tx.commit()?;
            Ok(n)
        })();
        drop(body);
        match written {
            Ok(n) => {
                succeeded += 1;
                match report.sources.iter_mut().find(|(t, _)| *t == feed.tag) {
                    Some((_, total)) => *total += n,
                    None => report.sources.push((feed.tag.clone(), n)),
                }
            }
            Err(e) => report.failed.push(format!("{} parse: {e:#}", feed.tag)),
        }
    }

    // Re-price events that were unpriced at ingest — a grown price book may
    // now cover them.
    if succeeded > 0 {
        let book = PriceBook::load(store)?;
        report.repriced += reprice_unpriced(store, &book)?;
    }
    if let Some(n) = reprice_if_rules_changed(store)? {
        report.repriced += n;
        report.rules_repass = true;
    }
    if succeeded == 0 && !report.rules_repass {
        anyhow::bail!("all price sources failed: {}", report.failed.join("; "));
    }
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

/// Re-price the rows selected by `filter` (a SQL predicate on `usage_events`)
/// with the book's one pricing function, writing only rows whose booked
/// price actually changes. Returns how many changed. One transaction.
fn reprice_rows(store: &Store, book: &PriceBook, filter: &str) -> Result<u64> {
    let mut st = store.conn().prepare(&format!(
        "SELECT rowid, model, request_model, provider_id,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                cost_usd, cost_source, pricing_model
         FROM usage_events WHERE {filter}"
    ))?;
    type Old = (Option<f64>, Option<String>, Option<String>);
    let rows: Vec<(i64, UsageEvent, Old)> = st
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
                (r.get(10)?, r.get(11)?, r.get(12)?),
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
        for (rowid, ev, (old_cost, old_src, old_pm)) in &rows {
            let (pm, cost, src) = book.price(ev);
            let same = old_src.as_deref() == Some(src.as_str())
                && *old_pm == pm
                && old_cost.is_some_and(|c| (c - cost).abs() < 5e-10);
            if !same {
                up.execute(rusqlite::params![cost, src.as_str(), pm, rowid])?;
                n += 1;
            }
        }
    }
    tx.commit()?;
    Ok(n)
}

/// Repricing only touches rows that carry tokens: metadata-tier adapters
/// (Cursor/Qoder) book zero-token rows as `unpriced` on purpose, and a
/// resolvable model name must not turn them into a "computed $0".
const HAS_TOKENS: &str = "(input_tokens + output_tokens + cache_read_tokens
     + cache_write_5m_tokens + cache_write_1h_tokens) > 0";

/// Derived data: a failed rebuild is logged, never fatal (the engine does the
/// same), so it cannot block the repricing it follows.
fn rebuild_rollups_best_effort(store: &Store) {
    if let Err(e) = store.rebuild_rollups(&crate::viewmodel::local_utc_offset()) {
        tracing::warn!("rollup rebuild failed: {e}");
    }
}

/// Re-resolve `unpriced` events against the current book. Bounded by the
/// unpriced count; one transaction. Rollups are rebuilt when anything moved
/// (the cube reads events, but the rollup table must not go stale).
pub fn reprice_unpriced(store: &Store, book: &PriceBook) -> Result<u64> {
    let n = reprice_rows(
        store,
        book,
        &format!("cost_source='unpriced' AND {HAS_TOKENS}"),
    )?;
    if n > 0 {
        rebuild_rollups_best_effort(store);
    }
    Ok(n)
}

/// One full repass when the pricing rules or the pricing logic differ from
/// what the ledger was last priced under: every book-priced row
/// (computed/estimated/unpriced) is re-priced by the shared function — a
/// priced row may become unpriced. Adapter-reported costs are never touched.
/// `None` = nothing to do; `Some(n)` = `n` rows changed.
pub fn reprice_if_rules_changed(store: &Store) -> Result<Option<u64>> {
    let rules = rules::current(store);
    let marker = format!("{PRICING_LOGIC_VERSION}:{}", rules.fingerprint());
    if store.get_state(APPLIED_KEY)?.as_deref() == Some(marker.as_str()) {
        return Ok(None);
    }
    let book = PriceBook::load(store)?;
    let n = reprice_rows(
        store,
        &book,
        &format!("cost_source IN ('computed','estimated','unpriced') AND {HAS_TOKENS}"),
    )?;
    rebuild_rollups_best_effort(store);
    store.set_state(APPLIED_KEY, &marker)?;
    Ok(Some(n))
}

/// Stage 1 normalization (spec §7.2.1): last `/` segment, drop `:` suffix,
/// `@`→`-`, lowercase, strip `[1m]` context-tag.
pub fn normalize_key(raw: &str) -> String {
    let tail = raw.rsplit_once('/').map(|(_, t)| t).unwrap_or(raw);
    let no_tag = tail.split(':').next().unwrap_or(tail);
    let s = no_tag.replace('@', "-").to_lowercase();
    s.strip_suffix("[1m]").map_or(s.clone(), |s| s.to_string())
}

/// The keys a raw id is looked up under before any peeling: its normalized
/// head, plus — for `provider:model` shapes such as `custom-local:glm-5.3` —
/// the model after the first `:`. A tail that does not look like a model id
/// (`:free`, `:0`, `:70b-instruct`) is a tag, not a name, and adds nothing.
/// (`normalize_key` itself stays head-only: it also keys feed imports.)
fn primary_keys(raw: &str) -> Vec<String> {
    let mut keys = vec![normalize_key(raw)];
    let tail = raw.rsplit_once('/').map_or(raw, |(_, t)| t);
    if let Some((_, after)) = tail.split_once(':')
        && after.starts_with(|c: char| c.is_ascii_alphabetic())
        && after.contains('-')
    {
        let k = normalize_key(after);
        if !k.is_empty() && !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

/// [`candidates_with`] under the bundled rules.
pub fn candidates(raw: &str) -> Vec<String> {
    candidates_with(raw, &Rules::bundled())
}

/// Peel prefixes/suffixes progressively (spec §7.2.2). Yields most-specific
/// first; exact-match phase covers all of these before prefix matching.
pub fn candidates_with(raw: &str, rules: &Rules) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push = |s: String, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if !s.is_empty() && seen.insert(s.clone()) {
            out.push(s);
        }
    };
    let seeds = primary_keys(raw);
    for s in &seeds {
        push(s.clone(), &mut out, &mut seen);
    }

    let mut queue = std::collections::VecDeque::from(seeds);
    while let Some(c) = queue.pop_front() {
        let mut peel = |rest: &str, out: &mut Vec<String>| {
            push(rest.to_string(), out, &mut seen);
            queue.push_back(rest.to_string());
        };
        // vendor prefixes: openai./anthropic./moonshot./bedrock./global.
        for p in &rules.strip_prefixes {
            if let Some(rest) = c.strip_prefix(p.as_str()) {
                peel(rest, &mut out);
            }
        }
        // `rfind("claude-")` — bedrock-style `us.anthropic.claude-...` tails.
        for m in &rules.anchor_markers {
            if let Some(i) = c.rfind(m.as_str()).filter(|&i| i > 0) {
                peel(&c[i..], &mut out);
            }
        }
        // `-v<digits>` suffix.
        if let Some(stripped) = strip_num_suffix(&c, "-v") {
            peel(&stripped, &mut out);
        }
        // `-<digits>` date suffix (YYYYMMDD, YYMMDD, MMDD…).
        if let Some((head, date)) = c.rsplit_once('-')
            && rules.date_suffix_lengths.contains(&date.len())
            && date.bytes().all(|b| b.is_ascii_digit())
        {
            peel(head, &mut out);
        }
        // reasoning-effort / vendor suffixes.
        for suf in &rules.strip_suffixes {
            if let Some(head) = c.strip_suffix(suf.as_str()) {
                peel(head, &mut out);
            }
        }
    }
    out
}

fn strip_num_suffix(c: &str, marker: &str) -> Option<String> {
    let (head, tail) = c.rsplit_once(marker)?;
    (!tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())).then(|| head.to_string())
}

/// USD for one event (spec §7.4) at the default long-context threshold.
pub fn compute(ev: &UsageEvent, p: &Price) -> f64 {
    compute_at(ev, p, DEFAULT_LONG_CONTEXT)
}

/// USD for one event (spec §7.4): 4 components; LiteLLM tiers trigger on real
/// context size above `threshold` (input, output and cache-read each use
/// their tier, else the base price); missing columns fall back to the fixed
/// multipliers (cache_read 0.1×input, 5m write 1.25×, 1h write 2×).
pub fn compute_at(ev: &UsageEvent, p: &Price, threshold: u64) -> f64 {
    let long = ev.input_tokens + ev.cache_read_tokens + ev.cache_write_total() > threshold;
    let in_price = if long {
        p.tier_above_200k_input.unwrap_or(p.input)
    } else {
        p.input
    };
    let out_price = if long {
        p.tier_above_200k_output.unwrap_or(p.output)
    } else {
        p.output
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
    let cr_base = if p.cache_read > 0.0 {
        p.cache_read
    } else {
        p.input * 0.1
    };
    let cr = if long {
        p.tier_above_200k_cache_read.unwrap_or(cr_base)
    } else {
        cr_base
    };

    let usd = (ev.input_tokens as f64 * in_price
        + ev.output_tokens as f64 * out_price
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

    /// Two agreeing sources per model, so the vote is unambiguous.
    fn put_model(s: &Store, model: &str, input: f64, output: f64) {
        put_price(s, "models.dev", model, input, output);
        put_price(s, "openrouter", model, input, output);
    }

    fn resolved(book: &PriceBook, raw: &str, req: Option<&str>) -> Option<(String, &'static str)> {
        match book.resolve(raw, req, None) {
            Resolution::Priced(k, via, _) => Some((k, via)),
            Resolution::Unpriced => None,
        }
    }

    fn sample_book() -> PriceBook {
        let s = Store::open_memory().unwrap();
        for (m, i, o) in [
            ("deepseek-v4-pro", 1.0, 2.0),
            ("glm-5.3", 3.0, 4.0),
            ("deepseek-v3-2", 5.0, 6.0),
            ("x-model", 7.0, 8.0),
            ("claude-opus-5", 5.0, 25.0),
            ("free", 9.0, 9.0),
            ("70b-instruct", 9.5, 9.5),
            ("auto", 1.5, 1.5),
        ] {
            put_model(&s, m, i, o);
        }
        PriceBook::load(&s).unwrap()
    }

    #[test]
    fn six_digit_date_and_vendor_suffix_are_peeled() {
        let b = sample_book();
        assert_eq!(
            resolved(&b, "deepseek-v4-pro-202606", None),
            Some(("deepseek-v4-pro".into(), "prefix"))
        );
        assert_eq!(
            resolved(&b, "deepseek-v3-2-volc", None),
            Some(("deepseek-v3-2".into(), "prefix"))
        );
    }

    #[test]
    fn provider_colon_model_uses_the_model_after_the_colon() {
        let b = sample_book();
        assert_eq!(
            resolved(&b, "custom-local:glm-5.3", None),
            Some(("glm-5.3".into(), "exact"))
        );
        assert_eq!(
            primary_keys("custom-local:deepseek-v4-flash"),
            ["custom-local", "deepseek-v4-flash"]
        );
        // A tag is not a model: `:free`, `:0`, `:70b-instruct` add nothing.
        assert_eq!(
            primary_keys("anthropic/claude-opus-5:free"),
            ["claude-opus-5"]
        );
        assert_eq!(primary_keys("llama3.1:70b-instruct"), ["llama3.1"]);
        assert_eq!(primary_keys("bedrock/us.anthropic.claude-x-v1:0").len(), 1);
        assert!(!candidates("anthropic/claude-opus-5:free").contains(&"free".to_string()));
        assert!(!candidates("llama3.1:70b-instruct").contains(&"70b-instruct".to_string()));
        assert_eq!(
            resolved(&b, "anthropic/claude-opus-5:free", None),
            Some(("claude-opus-5".into(), "exact"))
        );
        assert_eq!(resolved(&b, "llama3.1:70b-instruct", None), None);
    }

    #[test]
    fn four_digit_peel_only_when_the_exact_id_is_absent() {
        let s = Store::open_memory().unwrap();
        put_model(&s, "x-model", 1.0, 2.0);
        let b = PriceBook::load(&s).unwrap();
        assert_eq!(
            resolved(&b, "x-model-0423", None),
            Some(("x-model".into(), "prefix"))
        );
        put_model(&s, "x-model-0423", 3.0, 4.0);
        let b = PriceBook::load(&s).unwrap();
        assert_eq!(
            resolved(&b, "x-model-0423", None),
            Some(("x-model-0423".into(), "exact"))
        );
    }

    #[test]
    fn routing_placeholder_is_never_priced_by_a_namesake_row() {
        let b = sample_book(); // two sources price "auto"
        assert_eq!(resolved(&b, "auto", None), None);
        assert_eq!(resolved(&b, "Auto", None), None);
        assert_eq!(
            resolved(&b, "auto", Some("claude-opus-5")),
            Some(("claude-opus-5".into(), "prefix"))
        );
        assert_eq!(resolved(&b, "auto", Some("auto")), None);
    }

    #[test]
    fn rule_aliases_map_a_name_to_a_book_key() {
        let mut b = sample_book();
        assert_eq!(resolved(&b, "my-local-glm", None), None);
        b.rules
            .aliases
            .insert("My-Local-GLM".into(), "glm-5.3".into());
        assert_eq!(
            resolved(&b, "my-local-glm", None),
            Some(("glm-5.3".into(), "alias"))
        );
        assert_eq!(
            resolved(&b, "something", Some("my-local-glm")),
            Some(("glm-5.3".into(), "alias"))
        );
        // Alias-priced rows are computed, not estimated.
        let ev = UsageEvent {
            model: Some("my-local-glm".into()),
            input_tokens: 1_000_000,
            ..Default::default()
        };
        let (pm, cost, src) = b.price(&ev);
        assert_eq!(
            (pm.as_deref(), src),
            (Some("glm-5.3"), CostSource::Computed)
        );
        assert!((cost - 3.0).abs() < 1e-9);
    }

    #[test]
    fn price_marks_only_peeled_matches_as_estimated() {
        let b = sample_book();
        let mk = |m: &str| UsageEvent {
            model: Some(m.into()),
            input_tokens: 1_000_000,
            ..Default::default()
        };
        assert_eq!(b.price(&mk("glm-5.3")).2, CostSource::Computed);
        assert_eq!(
            b.price(&mk("deepseek-v4-pro-202606")).2,
            CostSource::Estimated
        );
        assert_eq!(b.price(&mk("auto")).2, CostSource::Unpriced);
        assert_eq!(b.price(&UsageEvent::default()).2, CostSource::Unpriced);
    }

    #[test]
    fn every_bundled_feed_format_has_an_importer_and_unknown_ones_do_not() {
        let r = Rules::bundled();
        assert!(r.feeds.iter().all(|f| importer(f).is_some()));
        let mut f = r.feeds[0].clone();
        f.format = "hologram".into();
        assert!(importer(&f).is_none());
    }

    #[test]
    fn long_context_tiers_apply_above_the_threshold_only() {
        let p = Price {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            tier_above_200k_input: Some(6.0),
            tier_above_200k_output: Some(22.5),
            tier_above_200k_cache_read: Some(0.6),
            ..Default::default()
        };
        let ev = |input: u64| UsageEvent {
            input_tokens: input,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            ..Default::default()
        };
        // 1M cache-read alone puts the context above 200k.
        let long = compute(&ev(1_000_000), &p);
        assert!((long - (6.0 + 22.5 + 0.6)).abs() < 1e-9, "{long}");
        let short_ev = UsageEvent {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 0,
            ..Default::default()
        };
        // At the threshold boundary itself the base prices still apply.
        let at = UsageEvent {
            input_tokens: 200_000,
            output_tokens: 1_000_000,
            ..Default::default()
        };
        assert!((compute(&at, &p) - (0.6 + 15.0)).abs() < 1e-9);
        assert!((compute_at(&short_ev, &p, 2_000_000) - (3.0 + 15.0)).abs() < 1e-9);
        // No tier columns → base prices even above the threshold.
        let plain = Price {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            ..Default::default()
        };
        assert!((compute(&ev(1_000_000), &plain) - (3.0 + 15.0 + 0.3)).abs() < 1e-9);
    }

    fn event_row(s: &Store, key: &str) -> (f64, String, Option<String>) {
        s.conn()
            .query_row(
                "SELECT cost_usd, cost_source, pricing_model FROM usage_events WHERE dedup_key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    #[test]
    fn rules_repass_reprices_book_rows_once_and_rebuilds_rollups() {
        let s = Store::open_memory().unwrap();
        put_model(&s, "auto", 1.5, 1.5);
        put_model(&s, "claude-opus-5", 5.0, 25.0);
        let mk = |key: &str, model: &str, cost: f64, src: CostSource| UsageEvent {
            dedup_key: key.into(),
            app: crate::model::apps::CLAUDE.into(),
            pricing_model: (model != "auto").then(|| model.to_string()),
            model: Some(model.into()),
            ts_start: Some(1_800_000_000_000),
            input_tokens: 1_000_000,
            cost_usd: Some(cost),
            cost_source: Some(src),
            ..Default::default()
        };
        s.upsert_event(&mk("auto1", "auto", 1.5, CostSource::Estimated))
            .unwrap();
        s.upsert_event(&mk(
            "rep",
            "claude-opus-5",
            9.0,
            CostSource::ProviderReported,
        ))
        .unwrap();
        s.upsert_event(&mk("stale", "claude-opus-5", 0.0001, CostSource::Computed))
            .unwrap();
        s.upsert_event(&mk("ok", "claude-opus-5", 5.0, CostSource::Computed))
            .unwrap();
        s.rebuild_rollups("+00:00").unwrap();

        assert_eq!(reprice_if_rules_changed(&s).unwrap(), Some(2));
        assert_eq!(event_row(&s, "auto1"), (0.0, "unpriced".into(), None));
        assert_eq!(event_row(&s, "rep").0, 9.0); // adapter cost never touched
        assert_eq!(event_row(&s, "rep").1, "provider_reported");
        assert_eq!(
            event_row(&s, "stale"),
            (5.0, "computed".into(), Some("claude-opus-5".into()))
        );
        let marker = s.get_state(APPLIED_KEY).unwrap().unwrap();
        assert!(marker.starts_with(&format!("{PRICING_LOGIC_VERSION}:")));
        // Second run: nothing to do.
        assert_eq!(reprice_if_rules_changed(&s).unwrap(), None);
        // Rollups follow the new costs (9 + 5 + 5, the auto row now free).
        let rolled: f64 = s
            .conn()
            .query_row("SELECT SUM(cost_usd) FROM daily_rollups", [], |r| r.get(0))
            .unwrap();
        assert!((rolled - 19.0).abs() < 1e-9, "{rolled}");
        // New rules (here: an alias) invalidate the marker.
        let mut newer = rules::Rules::bundled();
        newer.revision = 9;
        newer.aliases.insert("auto".into(), "claude-opus-5".into());
        rules::store_remote(&s, &serde_json::to_string(&newer).unwrap()).unwrap();
        assert_eq!(reprice_if_rules_changed(&s).unwrap(), Some(1));
        assert_eq!(event_row(&s, "auto1").1, "computed");
    }

    #[test]
    fn zero_token_unpriced_rows_stay_unpriced_when_the_model_resolves() {
        let s = Store::open_memory().unwrap();
        put_model(&s, "claude-opus-5", 5.0, 25.0);
        let mk = |key: &str, tokens: u64| UsageEvent {
            dedup_key: key.into(),
            app: crate::model::apps::CLAUDE.into(),
            model: Some("claude-opus-5".into()),
            input_tokens: tokens,
            cost_usd: Some(0.0),
            cost_source: Some(CostSource::Unpriced),
            ..Default::default()
        };
        // Metadata-tier activity record vs. a real usage row, same model.
        s.upsert_event(&mk("meta", 0)).unwrap();
        s.upsert_event(&mk("real", 1_000_000)).unwrap();
        let book = PriceBook::load(&s).unwrap();
        assert_eq!(reprice_unpriced(&s, &book).unwrap(), 1);
        assert_eq!(event_row(&s, "meta"), (0.0, "unpriced".into(), None));
        assert_eq!(
            event_row(&s, "real"),
            (5.0, "computed".into(), Some("claude-opus-5".into()))
        );
        // Same guard on the rules repass (a fresh marker, so it does run).
        s.upsert_event(&mk("meta2", 0)).unwrap();
        assert_eq!(reprice_if_rules_changed(&s).unwrap(), Some(0));
        assert_eq!(event_row(&s, "meta"), (0.0, "unpriced".into(), None));
        assert_eq!(event_row(&s, "meta2"), (0.0, "unpriced".into(), None));
    }

    #[test]
    fn reprice_unpriced_uses_the_shared_function_and_rebuilds_rollups() {
        let s = Store::open_memory().unwrap();
        let mut e = unpriced_ev("u1", "brand-new-model-202606");
        e.ts_start = Some(1_800_000_000_000);
        s.upsert_event(&e).unwrap();
        s.rebuild_rollups("+00:00").unwrap();
        put_model(&s, "brand-new-model", 2.0, 10.0);
        let book = PriceBook::load(&s).unwrap();
        assert_eq!(reprice_unpriced(&s, &book).unwrap(), 1);
        assert_eq!(event_row(&s, "u1").1, "estimated"); // peeled date suffix
        let rolled: f64 = s
            .conn()
            .query_row("SELECT SUM(cost_usd) FROM daily_rollups", [], |r| r.get(0))
            .unwrap();
        assert!((rolled - 0.007).abs() < 1e-9, "{rolled}");
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
