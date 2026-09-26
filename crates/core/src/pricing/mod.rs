//! Pricing pipeline (spec §7.2–§7.4): alias BFS → price lookup → USD math.
//!
//! Lookup order: `price_overrides` (user) → models.dev (provider-anchored) →
//! LiteLLM (prefixed keys) → unpriced. Never guess, never spread across a
//! family — unpriced is flagged, counted 0, and routed to the overrides UI.

mod seed;

use crate::model::{CostSource, UsageEvent};
use crate::store::Store;
use anyhow::Result;
use std::collections::HashMap;

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
        let mut st = store.conn().prepare(
            "SELECT model_id, input, output, cache_read, cache_write,
                    tier_above_200k_input, tier_1h_cache_write, tier_batch
             FROM prices ORDER BY CASE source WHEN 'seed' THEN 0 ELSE 1 END",
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
}
