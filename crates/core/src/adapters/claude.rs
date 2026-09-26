//! Claude Code adapter (spec §6.1): `~/.claude/projects/<slug>/*.jsonl`.
//!
//! IRON RULE: streamed messages write multiple rows sharing `message.id`
//! (usage accumulates per line). Keep only the LAST line per id — measured
//! 2.1× overcount otherwise (spec §3.1).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{num, text, ts_ms};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Claude;

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
        _prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut by_msg: HashMap<String, UsageEvent> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

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
            if v.get("type").and_then(Value::as_str) != Some("assistant") {
                continue;
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
                ts_start: ts_ms(&v["timestamp"]),
                input_tokens: num(&u["input_tokens"]),
                output_tokens: num(&u["output_tokens"]),
                reasoning_tokens: num(&u["output_tokens_details"]["thinking_tokens"]),
                cache_read_tokens: num(&u["cache_read_input_tokens"]),
                cache_write_5m_tokens: cw5,
                cache_write_1h_tokens: cw1,
                provenance: Provenance::LocalJsonl,
                duration_ms: num64(&v["durationMs"]),
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
