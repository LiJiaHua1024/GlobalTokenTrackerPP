//! Field-variant → unified-schema normalization helpers (spec §7.1).
//!
//! Adapters produce `UsageEvent`s whose `input_tokens` already EXCLUDES cache
//! read/write (`input_semantics = 'excludes_cache'`). These helpers implement
//! the per-family subtraction rules and tolerant JSON accessors.

use serde_json::Value;

/// `prompt_tokens`-style counts INCLUDE cache read+write (Grok/WorkBuddy/Qoder).
/// Bare input = prompt − cache_read − cache_write (clamp at 0).
pub fn input_excludes_cache(prompt: u64, cache_read: u64, cache_write: u64) -> u64 {
    prompt
        .saturating_sub(cache_read)
        .saturating_sub(cache_write)
}

/// Numeric getter tolerant of i64/u64/f64 JSON values.
pub fn num(v: &Value) -> u64 {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().map(|x| x.max(0) as u64))
            .or_else(|| n.as_f64().map(|x| x.max(0.0) as u64))
            .unwrap_or(0),
        _ => 0,
    }
}

pub fn num_opt(v: &Value) -> Option<u64> {
    match v {
        Value::Number(_) => Some(num(v)),
        _ => None,
    }
}

pub fn fnum(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// ISO-8601 / RFC3339 timestamp string → epoch ms.
pub fn ts_ms(v: &Value) -> Option<i64> {
    use std::str::FromStr;
    let s = text(v)?;
    jiff::Timestamp::from_str(&s)
        .ok()
        .map(|t| t.as_millisecond())
        .or_else(|| {
            // "2026-09-01T10:00:00" without zone — treat as UTC (tool logs are UTC).
            format!("{s}Z")
                .parse::<jiff::Timestamp>()
                .ok()
                .map(|t| t.as_millisecond())
        })
}

/// Longest call duration we accept when it is derived from log timestamps
/// (an idle gap between prompts is not a call).
pub const MAX_DERIVED_DURATION_MS: i64 = 30 * 60 * 1000;

/// `end − start` for adapters that infer a call's duration from timestamps;
/// `None` unless it lies in `(0, MAX_DERIVED_DURATION_MS]`.
pub fn derived_duration(start: Option<i64>, end: Option<i64>) -> Option<i64> {
    let d = end? - start?;
    (d > 0 && d <= MAX_DERIVED_DURATION_MS).then_some(d)
}

/// Epoch ms from a JSON number that may be s/ms/µs/ns or an ISO string.
pub fn epoch_ms(v: &Value) -> Option<i64> {
    if let Some(s) = text(v) {
        return ts_ms(&Value::String(s));
    }
    let n = v.as_i64()?;
    let abs = n.unsigned_abs();
    Some(if abs > 1_000_000_000_000_000_000 {
        n / 1_000_000 // ns
    } else if abs > 1_000_000_000_000_000 {
        n / 1_000 // µs
    } else if abs > 1_000_000_000_000 {
        n // ms
    } else {
        n * 1_000 // s
    })
}
