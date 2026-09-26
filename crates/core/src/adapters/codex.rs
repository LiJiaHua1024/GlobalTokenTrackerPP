//! Codex adapter (spec §6.2): `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
//! (+ `archived_sessions/`). `token_count` events carry per-turn increments
//! (`last_token_usage`) — each becomes one row; `rate_limits` rides along as
//! free official quota signals. Session model comes from the most recent
//! `turn_context` line (order-tracked, models can change mid-session).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, QuotaSnapshot, UsageEvent, apps};
use crate::normalize::{fnum, input_excludes_cache, num, text, ts_ms};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One line's usage snapshot (both the per-call increment and cumulative).
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq)]
struct Totals5 {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
}

impl Totals5 {
    fn sum(&self) -> u64 {
        self.input + self.cached + self.cache_write + self.output + self.reasoning
    }
}

/// Previous token_count line's signature, persisted as `adapter_state` so
/// duplicate detection also works across append scans.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
struct PrevLine {
    inc: Totals5,
    cum: Totals5,
}

impl PrevLine {
    fn is_same(&self, inc: &Totals5, cum: &Totals5) -> bool {
        // `self` is zeroed on the first line ever — a real first line has
        // cum.inc-equal non-zero values, so no false-positive dup.
        self.inc == *inc && self.cum == *cum && inc.sum() > 0
    }
}

pub struct Codex;

impl SourceAdapter for Codex {
    fn id(&self) -> &'static str {
        apps::CODEX
    }
    fn display_name(&self) -> &'static str {
        "Codex"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut out = Vec::new();
        for sub in [".codex/sessions", ".codex/archived_sessions"] {
            let root = crate::sync::home(sub);
            out.extend(
                crate::sync::collect_files(&root, "jsonl", 6)
                    .into_iter()
                    .map(|p| SourceItem {
                        key: p.to_string_lossy().to_string(),
                        path: p,
                        kind: SourceKind::Jsonl,
                    }),
            );
        }
        Ok(out)
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        from: u64,
        data: &[u8],
        prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut out = ScanOutcome {
            consumed,
            ..Default::default()
        };
        let file = item
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let session_id = file.trim_start_matches("rollout-").to_string();

        // `last_token_usage` is the true per-call usage (verified: within a
        // sequence inc == Δcum, and it grows with context size). The same usage
        // is occasionally re-emitted on a second token_count line (identical
        // cum AND inc) — dedupe those consecutive verbatim repeats only.
        // `total_token_usage` resets on compaction and interleaves across
        // parallel sequences, so it's only used for dup detection, never summed.
        let mut prev: PrevLine = prior_state
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let mut cur_model: Option<String> = None;
        let mut cur_cwd: Option<String> = None;
        let mut last_quota: Option<(f64, i64)> = None; // (used_percent, resets_at) change-detect

        let mut pos = 0u64;
        for line in seg.split(|&b| b == b'\n') {
            let line_start = from + pos;
            pos += line.len() as u64 + 1;
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
            if ty == "turn_context" {
                let p = &v["payload"];
                cur_model = text(&p["model"]).or(cur_model);
                cur_cwd = text(&p["cwd"]).or(cur_cwd);
                continue;
            }
            if ty != "event_msg" {
                continue;
            }
            let p = &v["payload"];
            if p.get("type").and_then(Value::as_str) != Some("token_count") {
                continue;
            }
            let info = &p["info"];
            let u = &info["last_token_usage"];
            let tot = &info["total_token_usage"];
            let inc = Totals5 {
                input: num(&u["input_tokens"]),
                cached: num(&u["cached_input_tokens"]),
                cache_write: num(&u["cache_write_input_tokens"]),
                output: num(&u["output_tokens"]),
                reasoning: num(&u["reasoning_output_tokens"]),
            };
            let cum = Totals5 {
                input: num(&tot["input_tokens"]),
                cached: num(&tot["cached_input_tokens"]),
                cache_write: num(&tot["cache_write_input_tokens"]),
                output: num(&tot["output_tokens"]),
                reasoning: num(&tot["reasoning_output_tokens"]),
            };
            // Verbatim repeat of the previous emission → duplicate, skip usage
            // (quota below is still change-detected on its own signature).
            let is_dup = prev.is_same(&inc, &cum);
            prev = PrevLine { inc, cum };

            if !is_dup && inc.sum() > 0 {
                out.events.push(UsageEvent {
                    dedup_key: format!("codex:{file}:{line_start}"),
                    app: apps::CODEX.into(),
                    session_id: Some(session_id.clone()),
                    project: cur_cwd.clone(),
                    // Routing aliases (gpt-reserve, codex-auto-review) land here;
                    // priced or unpriced per §7.3.
                    model: cur_model.clone(),
                    request_model: cur_model.clone(),
                    ts_start: ts_ms(&v["timestamp"]),
                    // OpenAI semantic: input INCLUDES cached → normalize out.
                    input_tokens: input_excludes_cache(inc.input, inc.cached, inc.cache_write),
                    output_tokens: inc.output,
                    reasoning_tokens: inc.reasoning,
                    cache_read_tokens: inc.cached,
                    cache_write_5m_tokens: inc.cache_write,
                    provenance: Provenance::LocalJsonl,
                    raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                    ..Default::default()
                });
            }

            // Free official quota signal riding on the same line.
            let qs = quotas_from(&p["rate_limits"]);
            if let Some(first) = qs.first() {
                let sig = (
                    first.used_percent.unwrap_or(-1.0),
                    first.resets_at.unwrap_or(0),
                );
                if last_quota.as_ref() != Some(&sig) {
                    out.quotas.extend(qs);
                    last_quota = Some(sig);
                }
            }
        }
        out.new_state = serde_json::to_string(&prev).ok();
        Ok(out)
    }
}

fn quotas_from(rl: &Value) -> Vec<QuotaSnapshot> {
    if rl.is_null() {
        return vec![];
    }
    let primary = &rl["primary"];
    let used_pct = fnum(&primary["used_percent"]);
    let window_min = primary["window_minutes"].as_i64();
    let resets_at = primary["resets_at"].as_i64().map(|s| s * 1000);
    let plan = text(&rl["plan_type"]);
    let kind = match window_min {
        Some(m) if m <= 360 => "5h_block",
        Some(m) if m <= 1500 => "daily",
        Some(m) if m <= 11000 => "weekly",
        Some(_) => "monthly",
        None => "window",
    };
    let mut out = vec![];
    if used_pct.is_some() || resets_at.is_some() {
        out.push(QuotaSnapshot {
            app: apps::CODEX.into(),
            account: plan.clone(),
            captured_at: crate::store::now_ms(),
            window_kind: kind.into(),
            used: None,
            limit_value: None,
            used_percent: used_pct,
            resets_at,
            raw_json: Some(rl.to_string()),
        });
    }
    let creds = &rl["credits"];
    if creds["has_credits"].as_bool() == Some(true) {
        out.push(QuotaSnapshot {
            app: apps::CODEX.into(),
            account: plan,
            captured_at: crate::store::now_ms(),
            window_kind: "credits".into(),
            used: None,
            limit_value: fnum(&creds["balance"]),
            used_percent: None,
            resets_at: None,
            raw_json: None,
        });
    }
    out
}
