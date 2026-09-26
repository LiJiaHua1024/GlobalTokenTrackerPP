//! Pricing pipeline (spec §7.2–§7.4): alias BFS → price lookup → USD math.
//!
//! Lookup order: `price_overrides` (user) → models.dev (provider-anchored) →
//! LiteLLM (prefixed keys) → unpriced. Never guess, never spread across a
//! family — unpriced is flagged, counted 0, and routed to the overrides UI.

mod seed;

use crate::model::{CostSource, UsageEvent};
use crate::store::{Store, now_ms};
use anyhow::{Context, Result};
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

pub struct PriceBook {
    /// normalized model key → price (models.dev ∪ litellm ∪ seed).
    map: HashMap<String, Price>,
    /// user overrides, keyed by the RAW model name (pre-normalization) too.
    overrides: HashMap<String, Price>,
}

impl PriceBook {
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            overrides: HashMap::new(),
        }
    }

    pub fn load(store: &Store) -> Result<Self> {
        seed::ensure_seeded(store)?;
        let mut map = HashMap::new();
        // Seed rows load first; live-synced sources overwrite on key collision.
        // Precedence on key collision: seed < litellm < models.dev (live).
        let mut st = store.conn().prepare(
            "SELECT model_id, input, output, cache_read, cache_write,
                    tier_above_200k_input, tier_1h_cache_write, tier_batch
             FROM prices
             ORDER BY CASE source
                 WHEN 'seed' THEN 0 WHEN 'litellm' THEN 1 ELSE 2 END",
        )?;
        for r in st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Price {
                    input: r.get::<_, f64>(1).unwrap_or(0.0),
                    output: r.get::<_, f64>(2).unwrap_or(0.0),
                    cache_read: r.get::<_, f64>(3).unwrap_or(0.0),
                    cache_write: r.get::<_, f64>(4).unwrap_or(0.0),
                    tier_above_200k_input: r.get(5)?,
                    tier_1h_cache_write: r.get(6)?,
                    tier_batch: r.get(7)?,
                },
            ))
        })? {
            let (k, v) = r?;
            map.insert(k, v);
        }

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
        Ok(Self { map, overrides })
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
        self.map.get(cand).copied()
    }
}

// ── Live price refresh (network) ──────────────────────────────────────────

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
/// Auto-refresh cadence for the UI path.
pub const PRICE_TTL_SECS: i64 = 24 * 3600;

pub struct RefreshReport {
    pub models_dev: usize,
    pub litellm: usize,
    /// Formerly-unpriced events that gained a price after the refresh.
    pub repriced: u64,
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

pub fn prices_stale(store: &Store) -> Result<bool> {
    Ok(last_live_sync(store)?.is_none_or(|t| {
        (now_ms() - t) / 1000 > PRICE_TTL_SECS
    }))
}

fn http_get(url: &str) -> Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .new_agent();
    let mut resp = agent
        .get(url)
        .header("User-Agent", "CodeLedger")
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(resp.body_mut().read_to_string()?)
}

/// Fetch models.dev + LiteLLM price maps, upsert into `prices`, then reprice
/// any event still marked `unpriced` so newly-covered models gain estimates.
/// Offline/failed fetch leaves the existing book untouched (seed fallback).
pub fn refresh(store: &Store) -> Result<RefreshReport> {
    let now = now_ms();
    let mut report = RefreshReport {
        models_dev: 0,
        litellm: 0,
        repriced: 0,
    };
    // Each source is independent: a single outage must not block the other.
    // Both failing → Err, book untouched.
    let mut errs = Vec::new();
    let md = match http_get(MODELS_DEV_URL) {
        Ok(b) => match serde_json::from_str::<Value>(&b) {
            Ok(v) => Some(v),
            Err(e) => {
                errs.push(format!("models.dev parse: {e}"));
                None
            }
        },
        Err(e) => {
            errs.push(format!("models.dev fetch: {e}"));
            None
        }
    };
    let ll_body = match http_get(LITELLM_URL) {
        Ok(b) => Some(b),
        Err(e) => {
            errs.push(format!("litellm fetch: {e}"));
            None
        }
    };
    if md.is_none() && ll_body.is_none() {
        anyhow::bail!("all price sources failed: {}", errs.join("; "));
    }
    let tx = store.conn().unchecked_transaction()?;

    // models.dev: {provider: {models: {id: {cost: {input,output,cache_read,cache_write}}}}}
    if let Some(md) = md {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write, source, fetched_at)
             VALUES ('models.dev', ?1, ?2, ?3, ?4, ?5, 'models.dev', ?6)",
        )?;
        for prov in md.as_object().into_iter().flatten() {
            for (id, m) in prov.1["models"].as_object().into_iter().flatten() {
                let c = &m["cost"];
                if !c.is_object() {
                    continue;
                }
                let f = |k: &str| c[k].as_f64().unwrap_or(0.0);
                st.execute(rusqlite::params![
                    normalize_key(id),
                    f("input"),
                    f("output"),
                    f("cache_read"),
                    f("cache_write"),
                    now
                ])?;
                report.models_dev += 1;
            }
        }
    }

    // LiteLLM: {model: {input_cost_per_token, output_cost_per_token,
    //   cache_read_input_token_cost, cache_creation_input_token_cost,
    //   input_cost_per_token_above_200k_tokens,
    //   cache_creation_input_token_cost_above_1hr,
    //   input_cost_per_token_batches}} — all $/token → ×1e6 to $/1M.
    if let Some(body) = ll_body
        && let Ok(ll) = serde_json::from_str::<Value>(&body)
    {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO prices(provider, model_id, input, output, cache_read, cache_write,
                    tier_above_200k_input, tier_1h_cache_write, tier_batch, source, fetched_at)
             VALUES ('litellm', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'litellm', ?9)",
        )?;
        let m = |v: &Value, k: &str| v[k].as_f64().map(|x| x * 1e6);
        for (id, v) in ll.as_object().into_iter().flatten() {
            if !v["input_cost_per_token"].is_number() {
                continue; // skip spec entries ("sample_spec", defaults)
            }
            st.execute(rusqlite::params![
                normalize_key(id),
                m(v, "input_cost_per_token").unwrap_or(0.0),
                m(v, "output_cost_per_token").unwrap_or(0.0),
                m(v, "cache_read_input_token_cost").unwrap_or(0.0),
                m(v, "cache_creation_input_token_cost").unwrap_or(0.0),
                m(v, "input_cost_per_token_above_200k_tokens"),
                m(v, "cache_creation_input_token_cost_above_1hr"),
                m(v, "input_cost_per_token_batches"),
                now
            ])?;
            report.litellm += 1;
        }
    }
    tx.commit()?;

    // Re-price events that were unpriced at ingest — a grown price book may
    // now cover them.
    let book = PriceBook::load(store)?;
    report.repriced = reprice_unpriced(store, &book)?;
    Ok(report)
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
            if let Resolution::Priced(key, via, p) =
                book.resolve(ev.model.as_deref().unwrap_or(""), ev.request_model.as_deref(), ev.provider_id.as_deref())
            {
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
        assert_eq!(
            normalize_key("stealth/custom-alpha"),
            "custom-alpha"
        );
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
        s.upsert_event(&unpriced_ev("u1", "brand-new-model")).unwrap();
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
}
