//! Qoder adapter (spec §6.x): `~/.qoder/projects/<project-dir>/<session>.jsonl`
//! and `.../subagents/*.jsonl`.
//!
//! Qoder transcripts are stored in `~/.qoder/projects/`. Each interactive or
//! subagent conversation step produces an assistant turn carrying vendor
//! billing metrics in `message.usage.credits` (or `usage.credits`), along with
//! `cwd`, `sessionId`, `model`, and `request_id`.
//!
//! Byte-offset watermarks come from the Jsonl path like other adapters;
//! dedup is per-request (`qoder:{req_id}`).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{derived_duration, epoch_ms, fnum, input_excludes_cache, num, text};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct Qoder;

const ROOT: &str = ".qoder/projects";

impl SourceAdapter for Qoder {
    fn id(&self) -> &'static str {
        apps::QODER
    }
    fn display_name(&self) -> &'static str {
        "Qoder"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(ROOT)]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        // <project>/<session-uuid>.jsonl (depth 2)
        // <project>/<session-uuid>/subagents/<agent>.jsonl (depth 4)
        Ok(
            crate::sync::collect_files(&crate::sync::home(ROOT), "jsonl", 5)
                .into_iter()
                .map(|p| SourceItem {
                    key: p.to_string_lossy().to_string(),
                    path: p,
                    kind: SourceKind::Jsonl,
                })
                .collect(),
        )
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        from: u64,
        data: &[u8],
        _prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut out = ScanOutcome {
            consumed,
            ..Default::default()
        };

        let mut current_project: Option<String> = None;
        let mut last_user_ts: Option<i64> = None;
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

            let line_type = v.get("type").and_then(Value::as_str).unwrap_or("");
            match line_type {
                "workspace-directories" => {
                    if let Some(first) = v
                        .get("directories")
                        .and_then(Value::as_array)
                        .and_then(|dirs| dirs.first())
                        .and_then(text)
                    {
                        current_project = Some(first);
                    }
                }
                "user" => {
                    last_user_ts = epoch_ms(&v["timestamp"]);
                }
                "assistant" => {
                    let msg = &v["message"];
                    let usage = msg.get("usage").or_else(|| v.get("usage"));
                    let Some(usage) = usage else {
                        continue;
                    };
                    if usage.get("billable").and_then(Value::as_bool) == Some(false) {
                        continue;
                    }

                    let prompt = num(&usage["input_tokens"]);
                    let cache_read = num(&usage["cache_read_input_tokens"]);
                    let cache_write = num(&usage["cache_creation_input_tokens"]);
                    let input_tokens = input_excludes_cache(prompt, cache_read, cache_write);
                    let output_tokens = num(&usage["output_tokens"]);
                    let credits = fnum(&usage["credits"])
                        .or_else(|| fnum(&usage["original_credits"]))
                        .filter(|&c| c > 0.0);

                    if credits.is_none() && input_tokens == 0 && output_tokens == 0 {
                        continue;
                    }

                    let req_id = text(&usage["request_id"])
                        .or_else(|| text(&v["requestTokenAnchor"]["requestId"]))
                        .or_else(|| text(&v["uuid"]))
                        .or_else(|| text(&msg["id"]))
                        .unwrap_or_else(|| format!("{}@{}", item.key, line_start));

                    let session_id = text(&v["sessionId"])
                        .or_else(|| text(&v["session_id"]))
                        .or_else(|| {
                            if item.path.parent().and_then(|p| p.file_name())
                                == Some(std::ffi::OsStr::new("subagents"))
                            {
                                item.path
                                    .parent()
                                    .and_then(|p| p.parent())
                                    .and_then(|p| p.file_name())
                                    .map(|s| s.to_string_lossy().to_string())
                            } else {
                                item.path
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                            }
                        });

                    let project = text(&v["cwd"]).or_else(|| current_project.clone());
                    let model = text(&msg["model"]).or_else(|| text(&v["model"]));
                    let ts = epoch_ms(&v["timestamp"]);
                    let duration_ms = derived_duration(last_user_ts, ts);

                    out.events.push(UsageEvent {
                        dedup_key: format!("qoder:{req_id}"),
                        app: apps::QODER.into(),
                        session_id,
                        project,
                        model: model.clone(),
                        request_model: model,
                        ts_start: ts,
                        duration_ms,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens: cache_read,
                        cache_write_5m_tokens: cache_write,
                        credits,
                        status: text(&msg["stop_reason"]),
                        provenance: Provenance::LocalJsonl,
                        raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str) -> SourceItem {
        SourceItem {
            key: path.into(),
            path: PathBuf::from(path),
            kind: SourceKind::Jsonl,
        }
    }

    #[test]
    fn project_transcript_emits_credit_events() {
        let p = "/home/u/.qoder/projects/D--TestProj/sess-1234.jsonl";
        let data = br#"{"type":"workspace-directories","sessionId":"sess-1234","directories":["D:\\TestProj"]}
{"type":"user","uuid":"u1","timestamp":"2026-10-08T06:50:50.000Z","message":{"role":"user","content":"hello"}}
{"type":"assistant","uuid":"a1","timestamp":"2026-10-08T06:51:10.000Z","cwd":"D:\\TestProj","sessionId":"sess-1234","message":{"id":"resp_1","role":"assistant","model":"smodel","stop_reason":"tool_use","usage":{"input_tokens":0,"output_tokens":0,"credits":21.2433,"billable":true,"request_id":"req-abc-123"}}}
"#;
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert_eq!(out.consumed, data.len() as u64);
        assert_eq!(out.events.len(), 1);
        let ev = &out.events[0];
        assert_eq!(ev.dedup_key, "qoder:req-abc-123");
        assert_eq!(ev.session_id.as_deref(), Some("sess-1234"));
        assert_eq!(ev.project.as_deref(), Some("D:\\TestProj"));
        assert_eq!(ev.model.as_deref(), Some("smodel"));
        assert_eq!(ev.credits, Some(21.2433));
        assert_eq!(ev.status.as_deref(), Some("tool_use"));
        assert_eq!(ev.duration_ms, Some(20_000));
        assert!(ev.is_billable());
    }

    #[test]
    fn subagent_transcript_resolves_parent_session() {
        let p = "/home/u/.qoder/projects/D--TestProj/sess-1234/subagents/agent-xyz.jsonl";
        let data = br#"{"type":"user","uuid":"u2","timestamp":"2026-10-08T06:51:11.000Z","message":{"role":"user","content":"sub"}}
{"type":"assistant","uuid":"a2","timestamp":"2026-10-08T06:51:15.000Z","cwd":"D:\\TestProj","message":{"id":"resp_2","role":"assistant","model":"ultimate","stop_reason":"end_turn","usage":{"credits":7.5,"billable":true,"request_id":"req-sub-456"}}}
"#;
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        let ev = &out.events[0];
        assert_eq!(ev.dedup_key, "qoder:req-sub-456");
        // Inherits parent sess-1234 from path
        assert_eq!(ev.session_id.as_deref(), Some("sess-1234"));
        assert_eq!(ev.model.as_deref(), Some("ultimate"));
        assert_eq!(ev.credits, Some(7.5));
        assert_eq!(ev.duration_ms, Some(4_000));
    }

    #[test]
    fn non_billable_or_zero_credit_skipped() {
        let p = "/home/u/.qoder/projects/D--TestProj/sess-1234.jsonl";
        let data = br#"{"type":"assistant","uuid":"a3","timestamp":"2026-10-08T06:51:20.000Z","cwd":"D:\\TestProj","sessionId":"sess-1234","message":{"id":"resp_3","role":"assistant","model":"smodel","usage":{"credits":0.0,"billable":true,"request_id":"zero"}}}
{"type":"assistant","uuid":"a4","timestamp":"2026-10-08T06:51:21.000Z","cwd":"D:\\TestProj","sessionId":"sess-1234","message":{"id":"resp_4","role":"assistant","model":"smodel","usage":{"credits":10.0,"billable":false,"request_id":"unbilled"}}}
"#;
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert!(out.events.is_empty());
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn partial_tail_not_consumed() {
        let p = "/home/u/.qoder/projects/D--TestProj/sess-1234.jsonl";
        let data = b"{\"type\":\"user\",\"timestamp\":\"2026-10-08T06:50:50Z\"}\n{\"type\":\"assistant\",\"time";
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert!(out.events.is_empty());
        assert!(out.consumed < data.len() as u64);
    }
}
