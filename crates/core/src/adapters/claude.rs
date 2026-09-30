//! Claude Code adapter (spec §6.1): `~/.claude/projects/<slug>/*.jsonl`.
//!
//! IRON RULE: streamed messages write multiple rows sharing `message.id`
//! (usage accumulates per line). Keep only the LAST line per id — measured
//! 2.1× overcount otherwise (spec §3.1).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{derived_duration, num, text, ts_ms};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Claude;

/// Timestamp of the latest `user` input line (prompt or tool result) per
/// `isSidechain` value — the start of the next assistant call. Persisted so a
/// call whose input landed in an earlier chunk still gets its duration.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ClaudeState {
    #[serde(default)]
    last_input: [Option<i64>; 2],
}

impl SourceAdapter for Claude {
    fn id(&self) -> &'static str {
        apps::CLAUDE
    }
    fn display_name(&self) -> &'static str {
        "Claude Code"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".claude/projects")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let root = crate::sync::home(".claude/projects");
        Ok(crate::sync::collect_files(&root, "jsonl", 3)
            .into_iter()
            .map(|p| SourceItem {
                key: p.to_string_lossy().to_string(),
                path: p,
                kind: SourceKind::Jsonl,
            })
            .collect())
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        from: u64,
        data: &[u8],
        prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut by_msg: HashMap<String, UsageEvent> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut state: ClaudeState = prior_state
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let mut start_of: HashMap<String, Option<i64>> = HashMap::new();

        let mut pos = 0u64;
        for line in seg.split(|&b| b == b'\n') {
            let line_start = from + pos;
            pos += line.len() as u64 + 1;
            let line = trim_cr(line);
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let side = usize::from(v["isSidechain"].as_bool().unwrap_or(false));
            match v.get("type").and_then(Value::as_str) {
                Some("user") => {
                    if let Some(t) = ts_ms(&v["timestamp"]) {
                        state.last_input[side] = Some(t);
                    }
                    continue;
                }
                Some("assistant") => {}
                _ => continue,
            }
            let Some(msg) = v.get("message") else {
                continue;
            };
            let Some(id) = msg.get("id").and_then(Value::as_str) else {
                continue;
            };
            let u = msg.get("usage").cloned().unwrap_or(Value::Null);

            // cache_creation split: prefer the explicit 5m/1h breakdown;
            // absent → all writes priced at the 5m tier (spec §6.1).
            let cc_total = num(&u["cache_creation_input_tokens"]);
            let (cw5, cw1) = match u.get("cache_creation") {
                Some(cc) => (
                    num(&cc["ephemeral_5m_input_tokens"]),
                    num(&cc["ephemeral_1h_input_tokens"]),
                ),
                None => (cc_total, 0),
            };
            let model = text(&msg["model"]);
            let ts = ts_ms(&v["timestamp"]);
            let start = *start_of
                .entry(id.to_string())
                .or_insert(state.last_input[side]);
            let ev = UsageEvent {
                // Global message.id dedup — resumed/forked sessions rewrite the
                // same messages into new files; keying per-file double-counts
                // ~7% cache_read (verified against cc-switch, spec §3.1).
                dedup_key: format!("claude:{id}"),
                app: apps::CLAUDE.into(),
                session_id: text(&v["sessionId"]),
                project: text(&v["cwd"]).or_else(|| project_from_path(&item.path)),
                model: model.clone(),
                request_model: model,
                ts_start: ts,
                input_tokens: num(&u["input_tokens"]),
                output_tokens: num(&u["output_tokens"]),
                reasoning_tokens: num(&u["output_tokens_details"]["thinking_tokens"]),
                cache_read_tokens: num(&u["cache_read_input_tokens"]),
                cache_write_5m_tokens: cw5,
                cache_write_1h_tokens: cw1,
                provenance: Provenance::LocalJsonl,
                duration_ms: num64(&v["durationMs"]).or_else(|| derived_duration(start, ts)),
                status: msg
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(String::from),
                raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                ..Default::default()
            };
            // Last line per message.id wins (streaming accumulation).
            if !by_msg.contains_key(id) {
                order.push(id.to_string());
            }
            by_msg.insert(id.to_string(), ev);
        }

        let events = order.iter().filter_map(|id| by_msg.remove(id)).collect();
        Ok(ScanOutcome {
            events,
            consumed,
            new_state: serde_json::to_string(&state).ok(),
            ..Default::default()
        })
    }
}

fn num64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

fn trim_cr(l: &[u8]) -> &[u8] {
    l.strip_suffix(b"\r").unwrap_or(l)
}

/// Project slug from the directory name (`D--Bilibili-Innocent-Lab` →
/// best-effort `D:/Bilibili-Innocent-Lab`; kept raw-ish for display).
fn project_from_path(p: &std::path::Path) -> Option<String> {
    p.parent()?
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> SourceItem {
        SourceItem {
            key: "k".into(),
            path: PathBuf::from("proj/s.jsonl"),
            kind: SourceKind::Jsonl,
        }
    }

    /// `sec` seconds after 10:00:00 (fits inside the hour for these tests).
    fn ts(sec: u32) -> String {
        format!(
            "2026-09-01T{:02}:{:02}:{:02}.000Z",
            10 + sec / 3600,
            sec / 60 % 60,
            sec % 60
        )
    }

    fn user(sec: u32, side: bool) -> String {
        format!(
            r#"{{"type":"user","isSidechain":{side},"timestamp":"{}","message":{{"role":"user"}}}}"#,
            ts(sec)
        )
    }

    fn asst(id: &str, sec: u32, side: bool, extra: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":{side},"timestamp":"{}"{extra},"message":{{"id":"{id}","model":"m","usage":{{"input_tokens":1,"output_tokens":2}}}}}}"#,
            ts(sec)
        )
    }

    fn parse(lines: &[String], state: Option<&str>) -> ScanOutcome {
        let data = format!("{}\n", lines.join("\n"));
        Claude
            .parse_jsonl(&item(), 0, data.as_bytes(), state)
            .unwrap()
    }

    fn dur(o: &ScanOutcome, id: &str) -> Option<i64> {
        o.events
            .iter()
            .find(|e| e.dedup_key == format!("claude:{id}"))
            .unwrap()
            .duration_ms
    }

    #[test]
    fn duration_is_assistant_minus_preceding_input() {
        let o = parse(&[user(0, false), asst("a", 4, false, "")], None);
        assert_eq!(dur(&o, "a"), Some(4_000));
    }

    #[test]
    fn streamed_message_uses_its_last_line() {
        let o = parse(
            &[
                user(0, false),
                asst("a", 1, false, ""),
                asst("a", 5, false, ""),
                user(6, false),
                asst("b", 8, false, ""),
            ],
            None,
        );
        assert_eq!(o.events.len(), 2);
        assert_eq!(dur(&o, "a"), Some(5_000));
        assert_eq!(dur(&o, "b"), Some(2_000));
    }

    #[test]
    fn state_chains_input_timestamps_across_chunks() {
        let first = parse(&[user(0, false)], None);
        assert!(first.events.is_empty());
        let st = first.new_state.unwrap();
        let second = parse(&[asst("a", 7, false, "")], Some(&st));
        assert_eq!(dur(&second, "a"), Some(7_000));
        // No carried state → nothing to measure from.
        assert_eq!(dur(&parse(&[asst("a", 7, false, "")], None), "a"), None);
        // Garbage state degrades to empty instead of failing the scan.
        assert_eq!(
            dur(&parse(&[asst("a", 7, false, "")], Some("{nope")), "a"),
            None
        );
    }

    #[test]
    fn sidechain_has_its_own_input_slot() {
        let o = parse(
            &[
                user(0, false),
                user(10, true),
                asst("main", 12, false, ""),
                asst("side", 14, true, ""),
            ],
            None,
        );
        assert_eq!(dur(&o, "main"), Some(12_000));
        assert_eq!(dur(&o, "side"), Some(4_000));
    }

    #[test]
    fn implausible_gaps_are_dropped_and_explicit_duration_wins() {
        let o = parse(&[user(0, false), asst("a", 31 * 60, false, "")], None);
        assert_eq!(dur(&o, "a"), None);
        let o = parse(&[user(0, false), asst("a", 30 * 60, false, "")], None);
        assert_eq!(dur(&o, "a"), Some(1_800_000));
        let o = parse(
            &[user(0, false), asst("a", 4, false, r#","durationMs":1234"#)],
            None,
        );
        assert_eq!(dur(&o, "a"), Some(1_234));
    }
}
