//! Codex adapter (spec §6.2): `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
//! (+ `archived_sessions/`). `token_count` events carry per-turn increments
//! (`last_token_usage`) — each becomes one row; `rate_limits` rides along as
//! free official quota signals. Session model comes from the most recent
//! `turn_context` line (order-tracked, models can change mid-session).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, QuotaSnapshot, UsageEvent, apps};
use crate::normalize::{derived_duration, fnum, input_excludes_cache, num, text, ts_ms};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

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
    /// When the model call in flight began: the last turn start, user message,
    /// tool result or emitted token_count. Absent in states written before
    /// durations.
    #[serde(default)]
    call_start_ms: Option<i64>,
    /// Latest model-output line (response item / usage record) of that call.
    #[serde(default)]
    model_out_ms: Option<i64>,
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

    fn watch_roots(&self) -> Vec<PathBuf> {
        [".codex/sessions", ".codex/archived_sessions"]
            .iter()
            .map(|s| crate::sync::home(s))
            .collect()
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

        // A call's duration runs from `call_start` to the LAST model-output line
        // before its `token_count`: Codex writes the token_count after the tool
        // has run (tool output, then token_count within milliseconds), so
        // token_count's own timestamp would measure the tool, not the model.
        // A tool result that arrives while a token_count is still pending is
        // `staged`: it starts the NEXT call once this one is emitted.
        let mut call_start: Option<i64> = prev.call_start_ms;
        let mut model_out: Option<i64> = prev.model_out_ms;
        let mut staged_start: Option<i64> = None;
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
            let ts = ts_ms(&v["timestamp"]);
            if ty == "turn_context" {
                let p = &v["payload"];
                cur_model = text(&p["model"]).or(cur_model);
                cur_cwd = text(&p["cwd"]).or(cur_cwd);
                continue;
            }
            if ty == "token_usage_record" {
                if ts.is_some() {
                    model_out = ts;
                }
                continue;
            }
            let p = &v["payload"];
            let pty = p.get("type").and_then(Value::as_str);
            if ty == "response_item" {
                match (pty, p["role"].as_str()) {
                    (Some("custom_tool_call_output" | "function_call_output"), _) => {
                        if model_out.is_some() {
                            staged_start = ts.or(staged_start);
                        } else if ts.is_some() {
                            call_start = ts;
                        }
                    }
                    (Some("message"), Some("user")) => {
                        if ts.is_some() {
                            call_start = ts;
                            model_out = None;
                            staged_start = None;
                        }
                    }
                    (Some("message"), Some("developer")) => {}
                    _ => {
                        if ts.is_some() {
                            model_out = ts;
                        }
                    }
                }
                continue;
            }
            if ty != "event_msg" {
                continue;
            }
            if pty == Some("task_started") {
                if ts.is_some() {
                    call_start = ts;
                    model_out = None;
                    staged_start = None;
                }
                continue;
            }
            if pty != Some("token_count") {
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
            prev.inc = inc;
            prev.cum = cum;

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
                    ts_start: ts,
                    duration_ms: derived_duration(call_start, model_out.or(ts)),
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
                call_start = staged_start.take().or(ts);
                model_out = None;
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
        prev.call_start_ms = call_start;
        prev.model_out_ms = model_out;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> SourceItem {
        SourceItem {
            key: "k".into(),
            path: PathBuf::from("rollout-abc.jsonl"),
            kind: SourceKind::Jsonl,
        }
    }

    fn ts(sec: u32) -> String {
        format!("2026-09-01T10:{:02}:{:02}.000Z", sec / 60 % 60, sec % 60)
    }

    fn started(sec: u32) -> String {
        format!(
            r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"task_started"}}}}"#,
            ts(sec)
        )
    }

    fn response(sec: u32, payload: &str) -> String {
        format!(
            r#"{{"timestamp":"{}","type":"response_item","payload":{payload}}}"#,
            ts(sec)
        )
    }

    /// `out` = this call's output tokens, `cum` = cumulative output so far.
    fn token_count(sec: u32, out: u64, cum: u64) -> String {
        format!(
            r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":10,"output_tokens":{out}}},"total_token_usage":{{"input_tokens":10,"output_tokens":{cum}}}}}}}}}"#,
            ts(sec)
        )
    }

    fn parse(lines: &[String], state: Option<&str>) -> ScanOutcome {
        let data = format!("{}\n", lines.join("\n"));
        Codex
            .parse_jsonl(&item(), 0, data.as_bytes(), state)
            .unwrap()
    }

    fn durs(o: &ScanOutcome) -> Vec<Option<i64>> {
        o.events.iter().map(|e| e.duration_ms).collect()
    }

    fn asst(sec: u32, ty: &str) -> String {
        response(sec, &format!(r#"{{"type":"{ty}","role":"assistant"}}"#))
    }

    fn usage_record(sec: u32) -> String {
        format!(
            r#"{{"timestamp":"{}","type":"token_usage_record","payload":{{}}}}"#,
            ts(sec)
        )
    }

    fn user_msg(sec: u32) -> String {
        response(sec, r#"{"type":"message","role":"user"}"#)
    }

    fn tool_out(sec: u32) -> String {
        response(sec, r#"{"type":"custom_tool_call_output"}"#)
    }

    #[test]
    fn plain_turn_runs_from_turn_start_to_the_last_model_output() {
        let o = parse(
            &[
                started(0),
                user_msg(0),
                asst(3, "reasoning"),
                asst(5, "message"),
                token_count(5, 7, 7),
            ],
            None,
        );
        assert_eq!(durs(&o), [Some(5_000)]);
    }

    #[test]
    fn token_count_after_tool_output_does_not_measure_the_tool() {
        // Real order: model output → tool runs → tool output → token_count
        // (milliseconds later). The call is the model's 5 s, then 4 s.
        let o = parse(
            &[
                started(0),
                user_msg(0),
                asst(5, "custom_tool_call"),
                usage_record(5),
                tool_out(8),
                token_count(8, 7, 7),
                asst(12, "custom_tool_call"),
                usage_record(12),
                tool_out(15),
                token_count(15, 8, 15),
            ],
            None,
        );
        assert_eq!(durs(&o), [Some(5_000), Some(4_000)]);
    }

    #[test]
    fn older_order_token_count_before_tool_output_restarts_at_the_result() {
        let o = parse(
            &[
                started(0),
                asst(4, "function_call"),
                token_count(4, 7, 7),
                tool_out(10),
                asst(13, "function_call"),
                token_count(13, 8, 15),
            ],
            None,
        );
        assert_eq!(durs(&o), [Some(4_000), Some(3_000)]);
    }

    #[test]
    fn a_new_user_message_restarts_the_clock_and_assistant_text_does_not() {
        let o = parse(
            &[
                started(0),
                asst(3, "message"),
                token_count(3, 7, 7),
                user_msg(40),
                asst(44, "message"),
                token_count(44, 1, 8),
            ],
            None,
        );
        assert_eq!(durs(&o), [Some(3_000), Some(4_000)]);
    }

    #[test]
    fn a_dup_line_neither_emits_nor_resets() {
        let o = parse(
            &[
                started(0),
                asst(3, "message"),
                token_count(3, 7, 7),
                token_count(4, 7, 7), // verbatim repeat of the previous usage
                asst(9, "message"),
                token_count(9, 8, 15),
            ],
            None,
        );
        assert_eq!(durs(&o), [Some(3_000), Some(6_000)]);
    }

    #[test]
    fn call_state_survives_chunk_boundaries_and_old_state_still_parses() {
        let first = parse(&[started(0), asst(3, "message")], None);
        assert!(first.events.is_empty());
        let st = first.new_state.unwrap();
        let second = parse(&[token_count(4, 7, 7)], Some(&st));
        assert_eq!(durs(&second), [Some(3_000)]);
        // A state persisted before durations existed has neither field.
        let old = r#"{"inc":{"input":10,"cached":0,"cache_write":0,"output":7,"reasoning":0},
                      "cum":{"input":10,"cached":0,"cache_write":0,"output":7,"reasoning":0}}"#;
        let o = parse(&[token_count(8, 8, 15)], Some(old));
        assert_eq!(durs(&o), [None]);
        let o = parse(
            &[started(6), asst(7, "message"), token_count(8, 8, 15)],
            Some(old),
        );
        assert_eq!(durs(&o), [Some(1_000)]);
    }

    #[test]
    fn idle_gap_longer_than_30_minutes_is_not_a_duration() {
        let o = parse(
            &[
                started(0),
                asst(31 * 60, "message"),
                token_count(31 * 60, 7, 7),
            ],
            None,
        );
        assert_eq!(durs(&o), [None]);
    }
}
