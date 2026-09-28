//! Qoder adapter (spec §6.x): `~/.qoder/logs/sessions/<group>/<session>/segments/*.jsonl`.
//!
//! Qoder's CLI workers run `--no-session-persistence` — no transcript with
//! per-request `usage` lands on disk, and `logs/runs/*/manifest.json` plus
//! `qodercli.log` carry only process/runtime metadata. The reliable local
//! signal is the session segment log: exactly one `session.config.loaded`
//! line per file (verified 86/86 on a live install) carrying
//! `data.project_root` and `data.model`.
//!
//! One event per segment file: ts from the config line, `duration_ms` from
//! the segment's first→last line span (the session-root phase is never
//! logged — headless runs exit without a `phase.finished` wrapper).
//! Tokens are genuinely unavailable → zero + `Unpriced`, never estimated.
//!
//! `ai-code-tracking-session-end` hook payloads and `tmp/telemetry` blobs
//! were checked: auth tokens and process ids only, no usage counters.
//!
//! Byte-offset watermarks come from the Jsonl path like other adapters;
//! dedup is per-file (`qoder:{file_stem}` — stem embeds ts+rand+pid).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{text, ts_ms};
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct Qoder;

const ROOT: &str = ".qoder/logs/sessions";

impl SourceAdapter for Qoder {
    fn id(&self) -> &'static str {
        apps::QODER
    }
    fn display_name(&self) -> &'static str {
        "Qoder"
    }
    fn capability(&self) -> Capability {
        Capability::Metadata
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(ROOT)]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        // <group>/<session-uuid>/segments/<file>.jsonl → depth 4.
        Ok(
            crate::sync::collect_files(&crate::sync::home(ROOT), "jsonl", 4)
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

        // Buffer the config line; the event is emitted once the segment is
        // fully walked so first/last timestamps give the observed span.
        let mut cfg: Option<(u64, Value)> = None; // (offset, line)
        let mut first_ts: Option<i64> = None;
        let mut last_ts: Option<i64> = None;
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
            if let Some(ts) = ts_ms(&v["ts"]) {
                if first_ts.is_none() {
                    first_ts = Some(ts);
                }
                last_ts = Some(ts);
            }
            if v.get("type").and_then(Value::as_str) == Some("session.config.loaded") {
                cfg = Some((line_start, v));
            }
        }

        if let Some((off, v)) = cfg {
            let data = &v["data"];
            let ts = ts_ms(&v["ts"]);
            // File stem embeds timestamp+rand+pid — unique per process run.
            let stem = item
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("{off}"));
            // Parent dir is the session uuid.
            let session_id = item
                .path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string());
            out.events.push(UsageEvent {
                dedup_key: format!("qoder:{stem}"),
                app: apps::QODER.into(),
                session_id,
                project: text(&data["project_root"]),
                model: text(&data["model"]),
                request_model: text(&data["model"]),
                ts_start: first_ts.or(ts),
                ts_end: last_ts,
                duration_ms: match (first_ts, last_ts) {
                    (Some(a), Some(b)) if b > a => Some(b - a),
                    _ => None,
                },
                status: Some(
                    if data["interactive"].as_bool().unwrap_or(false) {
                        "interactive"
                    } else {
                        "headless"
                    }
                    .into(),
                ),
                provenance: Provenance::LocalJsonl,
                raw_ref: Some(format!("{}@{off}", item.path.display())),
                ..Default::default()
            });
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
    fn config_line_emits_one_session_event() {
        let p = "/home/u/.qoder/logs/sessions/grp/sess-uuid/segments/run-p42.jsonl";
        let data = br#"{"ts":"2026-09-26T09:41:50.592+08:00","seq":1,"type":"cli.route.entered","data":{"route":"headlessStreamJson"}}
{"ts":"2026-09-26T09:41:50.700+08:00","seq":7,"type":"session.config.loaded","data":{"project_root":"C:\\x","model":"smodel","interactive":false}}
not json at all
{"ts":"2026-09-26T09:42:01.949+08:00","seq":12,"type":"session.phase.finished","data":{"phase":"mcp_context.refresh","duration_ms":12}}
"#;
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert_eq!(out.consumed, data.len() as u64);
        assert_eq!(out.events.len(), 1);
        let ev = &out.events[0];
        assert_eq!(ev.dedup_key, "qoder:run-p42");
        assert_eq!(ev.session_id.as_deref(), Some("sess-uuid"));
        assert_eq!(ev.project.as_deref(), Some("C:\\x"));
        assert_eq!(ev.model.as_deref(), Some("smodel"));
        assert_eq!(ev.status.as_deref(), Some("headless"));
        // span = first(09:41:50.592) → last(09:42:01.949)
        assert_eq!(ev.duration_ms, Some(11_357));
        assert_eq!(ev.input_tokens, 0);
    }

    #[test]
    fn no_config_line_no_event() {
        let p = "/s/grp/uuid/segments/f.jsonl";
        let data =
            br#"{"ts":"2026-09-26T09:41:50.592+08:00","type":"session.phase.started","data":{}}
"#;
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert!(out.events.is_empty());
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn partial_tail_not_consumed() {
        let p = "/s/grp/uuid/segments/f.jsonl";
        let data = b"{\"ts\":\"2026-09-26T09:41:50Z\",\"type\":\"x\",\"data\":{}}\n{\"ts\":\"2026-09-26T09";
        let out = Qoder.parse_jsonl(&item(p), 0, data, None).unwrap();
        assert!(out.events.is_empty());
        assert!(out.consumed < data.len() as u64);
    }
}
