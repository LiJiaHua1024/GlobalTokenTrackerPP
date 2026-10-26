//! Display-currency exchange rates: multi-source fetch with ordered
//! fallback, store-cached, offline-tolerant.
//!
//! Sources, in order (all key-less, all ECB-derived or ECB-mirrored):
//! 1. Frankfurter — ECB reference rates as JSON;
//! 2. open.er-api.com — daily JSON mirror;
//! 3. ECB daily XML direct — cross-computed from the EUR base.
//!
//! The ledger stays USD end to end; rates only drive presentation
//! (`viewmodel::fmt`). A stale cache still converts — `FxStatus` reports its
//! age so the settings page can say so.

use crate::store::{self, Store};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const RATES_KEY: &str = "fx_rates";
pub const DISPLAY_KEY: &str = "display_currency";

/// ECB publishes once a day (~16:00 CET); half a day of staleness is
/// invisible in a cost column and a stale cache beats no cache offline.
const TTL_MS: i64 = 12 * 60 * 60 * 1000;

const FRANKFURTER_URL: &str = "https://api.frankfurter.dev/v1/latest?base=";
const ER_API_URL: &str = "https://open.er-api.com/v6/latest/";
const ECB_XML_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    pub base: String,
    /// `base`-denominated price of one unit of each currency; base itself is 1.
    pub rates: BTreeMap<String, f64>,
    pub source: String,
    pub fetched_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FxStatus {
    /// Configured display currency ("USD" when unset).
    pub display: String,
    /// Multiplier from ledger USD to `display`; 1.0 when unknown.
    pub usd_to: f64,
    pub source: Option<String>,
    pub fetched_at_ms: Option<i64>,
    pub stale: bool,
}

pub fn display_currency(store: &Store) -> String {
    store
        .get_state(DISPLAY_KEY)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "USD".into())
}

pub fn set_display_currency(store: &Store, code: &str) -> Result<()> {
    store.set_state(DISPLAY_KEY, code)
}

pub fn cached_rates(store: &Store) -> Option<Rates> {
    let raw = store.get_state(RATES_KEY).ok()??;
    serde_json::from_str(&raw).ok()
}

pub fn status(store: &Store) -> FxStatus {
    let display = display_currency(store);
    let rates = cached_rates(store);
    let available = rates.as_ref().is_some_and(|r| {
        display == r.base
            || r.rates
                .get(&display)
                .is_some_and(|v| v.is_finite() && *v > 0.0)
    });
    let (source, fetched_at_ms, stale) = match &rates {
        Some(r) if available => (
            Some(r.source.clone()),
            Some(r.fetched_at_ms),
            store::now_ms() - r.fetched_at_ms > TTL_MS,
        ),
        _ => (None, None, false),
    };
    let usd_to = match &rates {
        Some(r) if display == r.base => 1.0,
        Some(r) => r.rates.get(&display).copied().unwrap_or(1.0),
        None => 1.0,
    };
    FxStatus {
        display,
        usd_to,
        source,
        fetched_at_ms,
        stale,
    }
}

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct FlightGuard;

impl Drop for FlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// Explicit refresh bypasses TTL. Call on a worker thread; automatic and
/// manual fetches share one guard, and a failed fetch keeps the old cache.
pub fn refresh(store: &Store) -> Result<FxStatus> {
    anyhow::ensure!(
        !IN_FLIGHT.swap(true, Ordering::SeqCst),
        "A rate sync is already running"
    );
    let _guard = FlightGuard;
    let display = display_currency(store);
    refresh_with(store, || fetch_rates_for("USD", Some(&display)))
}

fn refresh_with(store: &Store, fetch: impl FnOnce() -> Result<Rates>) -> Result<FxStatus> {
    let rates = fetch()?;
    anyhow::ensure!(
        rates.base == "USD"
            && !rates.rates.is_empty()
            && rates.rates.values().all(|v| v.is_finite() && *v > 0.0),
        "Invalid exchange-rate response"
    );
    let display = display_currency(store);
    anyhow::ensure!(
        display == rates.base || rates.rates.contains_key(&display),
        "Exchange-rate response lacks selected currency {display}"
    );
    store.set_state(RATES_KEY, &serde_json::to_string(&rates)?)?;
    Ok(status(store))
}

/// Kick a background refresh when the cache is missing or past TTL. Never
/// blocks and never touches the network from the caller's thread; `:memory:`
/// stores (tests) are skipped — they have nothing to persist.
pub fn ensure_fresh(store: &Store) {
    let display = display_currency(store);
    if let Some(r) = cached_rates(store)
        && store::now_ms() - r.fetched_at_ms <= TTL_MS
        && (display == r.base || r.rates.contains_key(&display))
    {
        return;
    }
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    // `:memory:` stores (tests) report a path but have nothing to persist.
    let Some(path) = store
        .path()
        .filter(|p| *p != Path::new(":memory:"))
        .map(Path::to_path_buf)
    else {
        IN_FLIGHT.store(false, Ordering::SeqCst);
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("gtt-fx".into())
        .spawn(move || {
            let _guard = FlightGuard;
            match fetch_rates_for("USD", Some(&display)) {
                Ok(rates) => match serde_json::to_string(&rates) {
                    Ok(json) => match Store::open(&path) {
                        Ok(s) => {
                            if let Err(e) = s.set_state(RATES_KEY, &json) {
                                tracing::warn!("fx cache write failed: {e:#}");
                            }
                        }
                        Err(e) => tracing::warn!("fx cache store open failed: {e:#}"),
                    },
                    Err(e) => tracing::warn!("fx cache serialize failed: {e:#}"),
                },
                Err(e) => tracing::debug!("fx fetch failed: {e:#}"),
            }
        });
    if spawned.is_err() {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// One lazy source: name + thunk. The thunk keeps the fallback order lazy —
/// an early success never costs a later request.
type Source<'a> = (
    &'static str,
    Box<dyn FnOnce() -> Result<BTreeMap<String, f64>> + 'a>,
);

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent()
}

/// Try the sources in order; the first non-empty rate map wins. Closures keep
/// the fallback lazy — an early success never costs a later request.
pub fn fetch_rates(base: &str) -> Result<Rates> {
    fetch_rates_for(base, None)
}

fn fetch_rates_for(base: &str, required: Option<&str>) -> Result<Rates> {
    let agent = agent();
    let sources: [Source<'_>; 3] = [
        ("frankfurter", Box::new(|| fetch_frankfurter(&agent, base))),
        ("er-api", Box::new(|| fetch_er_api(&agent, base))),
        ("ecb", Box::new(|| fetch_ecb_xml(&agent, base))),
    ];
    match required {
        Some(currency) => pick_for(base, Some(currency), sources),
        None => pick(base, sources),
    }
}

fn pick<'a>(base: &str, sources: impl IntoIterator<Item = Source<'a>>) -> Result<Rates> {
    pick_for(base, None, sources)
}

fn pick_for<'a>(
    base: &str,
    required: Option<&str>,
    sources: impl IntoIterator<Item = Source<'a>>,
) -> Result<Rates> {
    let mut last: Option<anyhow::Error> = None;
    for (name, fetch) in sources {
        match fetch() {
            Ok(rates)
                if !rates.is_empty()
                    && required.is_none_or(|code| code == base || rates.contains_key(code)) =>
            {
                return Ok(Rates {
                    base: base.into(),
                    rates,
                    source: name.into(),
                    fetched_at_ms: store::now_ms(),
                });
            }
            Ok(_) => {
                last = Some(anyhow::anyhow!(
                    "{name}: empty rates or selected currency unavailable"
                ));
            }
            Err(e) => last = Some(e.context(name)),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no fx source configured")))
}

fn fetch_frankfurter(agent: &ureq::Agent, base: &str) -> Result<BTreeMap<String, f64>> {
    let body = get_text(agent, &format!("{FRANKFURTER_URL}{base}"))?;
    rates_object(&serde_json::from_str::<serde_json::Value>(&body)?)
}

fn fetch_er_api(agent: &ureq::Agent, base: &str) -> Result<BTreeMap<String, f64>> {
    let body = get_text(agent, &format!("{ER_API_URL}{base}"))?;
    er_api_rates(&serde_json::from_str::<serde_json::Value>(&body)?)
}

fn er_api_rates(v: &serde_json::Value) -> Result<BTreeMap<String, f64>> {
    if v.get("result").and_then(|r| r.as_str()) != Some("success") {
        anyhow::bail!("er-api reported {}", v.get("result").unwrap_or(v));
    }
    rates_object(v)
}

/// ECB publishes EUR-based; cross every pair through EUR so the map is
/// `base`-denominated like the JSON sources.
fn fetch_ecb_xml(agent: &ureq::Agent, base: &str) -> Result<BTreeMap<String, f64>> {
    let body = get_text(agent, ECB_XML_URL)?;
    let eur = parse_ecb_cubes(&body);
    let eur_base = eur
        .get(base)
        .copied()
        .with_context(|| format!("ECB feed lacks {base}"))?;
    let mut out: BTreeMap<String, f64> = eur
        .into_iter()
        .map(|(cur, per_eur)| (cur, per_eur / eur_base))
        .collect();
    out.insert("EUR".into(), 1.0 / eur_base);
    out.insert(base.into(), 1.0);
    Ok(out)
}

/// `<Cube currency='USD' rate='1.16'/>` pairs; attribute order is fixed by
/// the feed, so a linear scan needs no XML dependency.
fn parse_ecb_cubes(xml: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    let mut rest = xml;
    while let Some(i) = rest.find("currency='") {
        let after_cur = &rest[i + "currency='".len()..];
        let Some(ce) = after_cur.find('\'') else {
            break;
        };
        let cur = &after_cur[..ce];
        let Some(j) = after_cur.find("rate='") else {
            break;
        };
        let after_rate = &after_cur[j + "rate='".len()..];
        let Some(re) = after_rate.find('\'') else {
            break;
        };
        if let Ok(rate) = after_rate[..re].parse::<f64>()
            && rate > 0.0
        {
            out.insert(cur.to_string(), rate);
        }
        rest = &after_rate[re + 1..];
    }
    out
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String> {
    let mut resp = agent.get(url).call()?;
    Ok(resp.body_mut().read_to_string()?)
}

fn rates_object(v: &serde_json::Value) -> Result<BTreeMap<String, f64>> {
    let obj = v
        .get("rates")
        .and_then(|r| r.as_object())
        .context("missing rates object")?;
    let mut out = BTreeMap::new();
    for (k, val) in obj {
        if let Some(n) = val.as_f64()
            && n > 0.0
        {
            out.insert(k.clone(), n);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn manual_refresh_bypasses_fresh_cache_and_preserves_ledger() {
        let store = mem();
        let old = Rates {
            base: "USD".into(),
            rates: BTreeMap::from([("CNY".into(), 7.0)]),
            source: "old".into(),
            fetched_at_ms: store::now_ms(),
        };
        store
            .set_state(RATES_KEY, &serde_json::to_string(&old).unwrap())
            .unwrap();
        set_display_currency(&store, "CNY").unwrap();
        store
            .upsert_event(&crate::UsageEvent {
                dedup_key: "fx-fixture".into(),
                app: "fixture".into(),
                input_tokens: 123,
                ..Default::default()
            })
            .unwrap();
        let status = refresh_with(&store, || {
            Ok(Rates {
                rates: BTreeMap::from([("CNY".into(), 7.2)]),
                source: "new".into(),
                ..old
            })
        })
        .unwrap();
        assert_eq!(status.usd_to, 7.2);
        assert_eq!(status.source.as_deref(), Some("new"));
        assert_eq!(
            store.totals(None, None, None, None).unwrap().input_tokens,
            123
        );
    }

    #[test]
    fn failed_or_invalid_refresh_keeps_previous_cache() {
        let store = mem();
        let old = Rates {
            base: "USD".into(),
            rates: BTreeMap::from([("CNY".into(), 7.0)]),
            source: "old".into(),
            fetched_at_ms: 42,
        };
        let raw = serde_json::to_string(&old).unwrap();
        store.set_state(RATES_KEY, &raw).unwrap();
        assert!(refresh_with(&store, || anyhow::bail!("offline fixture")).is_err());
        assert_eq!(
            store.get_state(RATES_KEY).unwrap().as_deref(),
            Some(raw.as_str())
        );
        assert!(
            refresh_with(&store, || Ok(Rates {
                rates: BTreeMap::from([("CNY".into(), f64::INFINITY)]),
                ..old
            }))
            .is_err()
        );
        assert_eq!(
            store.get_state(RATES_KEY).unwrap().as_deref(),
            Some(raw.as_str())
        );
    }

    #[test]
    fn display_currency_defaults_and_round_trips() {
        let s = mem();
        assert_eq!(display_currency(&s), "USD");
        set_display_currency(&s, "CNY").unwrap();
        assert_eq!(display_currency(&s), "CNY");
    }

    #[test]
    fn status_without_cache_is_identity() {
        let st = status(&mem());
        assert_eq!(st.display, "USD");
        assert_eq!(st.usd_to, 1.0);
        assert!(st.source.is_none());
        assert!(!st.stale);
    }

    #[test]
    fn status_converts_from_cache_and_flags_staleness() {
        let s = mem();
        let rates = Rates {
            base: "USD".into(),
            rates: BTreeMap::from([("CNY".into(), 7.1)]),
            source: "frankfurter".into(),
            fetched_at_ms: store::now_ms() - TTL_MS - 1,
        };
        s.set_state(RATES_KEY, &serde_json::to_string(&rates).unwrap())
            .unwrap();
        set_display_currency(&s, "CNY").unwrap();
        let st = status(&s);
        assert_eq!(st.usd_to, 7.1);
        assert_eq!(st.source.as_deref(), Some("frankfurter"));
        assert!(st.stale);
    }

    #[test]
    fn unknown_display_code_falls_back_to_identity() {
        let s = mem();
        let rates = Rates {
            base: "USD".into(),
            rates: BTreeMap::from([("CNY".into(), 7.1)]),
            source: "ecb".into(),
            fetched_at_ms: store::now_ms(),
        };
        s.set_state(RATES_KEY, &serde_json::to_string(&rates).unwrap())
            .unwrap();
        set_display_currency(&s, "XAU").unwrap();
        let st = status(&s);
        assert_eq!(st.usd_to, 1.0);
        assert!(st.source.is_none()); // No fictitious 1 USD = 1 XAU quote.
        assert!(st.fetched_at_ms.is_none());
    }

    #[test]
    fn ensure_fresh_skips_in_memory_stores() {
        ensure_fresh(&mem());
        assert!(!IN_FLIGHT.load(Ordering::SeqCst));
    }

    #[test]
    fn selected_currency_missing_from_first_source_uses_fallback() {
        let sources: Vec<Source<'static>> = vec![
            (
                "first",
                Box::new(|| Ok(BTreeMap::from([("CNY".into(), 7.1)]))),
            ),
            (
                "fallback",
                Box::new(|| Ok(BTreeMap::from([("TWD".into(), 31.0)]))),
            ),
        ];
        let rates = pick_for("USD", Some("TWD"), sources).unwrap();
        assert_eq!(rates.source, "fallback");
        assert_eq!(rates.rates["TWD"], 31.0);
    }

    #[test]
    fn pick_prefers_first_non_empty_source() {
        let sources: Vec<Source<'static>> = vec![
            (
                "first",
                Box::new(|| Ok(BTreeMap::from([("CNY".into(), 7.1)]))),
            ),
            (
                "second",
                Box::new(|| Ok(BTreeMap::from([("CNY".into(), 9.9)]))),
            ),
        ];
        let r = pick("USD", sources).unwrap();
        assert_eq!(r.source, "first");
        assert_eq!(r.rates["CNY"], 7.1);
    }

    #[test]
    fn pick_falls_through_errors_and_empty_maps() {
        let sources: Vec<Source<'static>> = vec![
            ("down", Box::new(|| anyhow::bail!("connect refused"))),
            ("empty", Box::new(|| Ok(BTreeMap::new()))),
            (
                "third",
                Box::new(|| Ok(BTreeMap::from([("JPY".into(), 155.0)]))),
            ),
        ];
        let r = pick("USD", sources).unwrap();
        assert_eq!(r.source, "third");
    }

    #[test]
    fn pick_all_failing_reports_last_error() {
        let sources: Vec<Source<'static>> = vec![
            ("a", Box::new(|| anyhow::bail!("boom a"))),
            ("b", Box::new(|| anyhow::bail!("boom b"))),
        ];
        let e = pick("USD", sources).unwrap_err();
        assert!(format!("{e:#}").contains("boom b"));
    }

    #[test]
    fn frankfurter_payload_parses() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"amount":1.0,"base":"USD","date":"2026-10-16","rates":{"CNY":7.08,"JPY":152.3}}"#,
        )
        .unwrap();
        let m = rates_object(&v).unwrap();
        assert_eq!(m["CNY"], 7.08);
        assert_eq!(m["JPY"], 152.3);
    }

    #[test]
    fn er_api_payload_requires_success() {
        let bad: serde_json::Value =
            serde_json::from_str(r#"{"result":"error","rates":{"CNY":7.0}}"#).unwrap();
        assert!(er_api_rates(&bad).is_err());
        let good: serde_json::Value =
            serde_json::from_str(r#"{"result":"success","rates":{"CNY":7.0}}"#).unwrap();
        assert_eq!(er_api_rates(&good).unwrap()["CNY"], 7.0);
    }

    #[test]
    fn ecb_xml_cross_computes_from_eur() {
        let xml = "<gesmes:Envelope><Cube><Cube time='2026-10-16'><Cube currency='USD' rate='1.16'/><Cube currency='CNY' rate='8.236'/><Cube currency='JPY' rate='176.7'/></Cube></Cube></gesmes:Envelope>";
        let eur = parse_ecb_cubes(xml);
        assert_eq!(eur["USD"], 1.16);
        let usd_cny = eur["CNY"] / eur["USD"];
        assert!((usd_cny - 7.1).abs() < 1e-9);
        // Full path: base-denominated map with EUR and base pinned.
        let mut out: BTreeMap<String, f64> = eur
            .into_iter()
            .map(|(cur, per_eur)| (cur, per_eur / 1.16))
            .collect();
        out.insert("EUR".into(), 1.0 / 1.16);
        out.insert("USD".into(), 1.0);
        assert_eq!(out["USD"], 1.0);
        assert!((out["EUR"] - 1.0 / 1.16).abs() < 1e-12);
        assert!((out["CNY"] - 7.1).abs() < 1e-9);
    }

    #[test]
    fn ecb_parser_ignores_malformed_cubes() {
        let xml = "<Cube currency='USD' rate='abc'/><Cube currency='GBP' rate='0.86'/><Cube currency='XX' rate='-2'/>";
        let eur = parse_ecb_cubes(xml);
        assert_eq!(eur.len(), 1);
        assert_eq!(eur["GBP"], 0.86);
    }
}
