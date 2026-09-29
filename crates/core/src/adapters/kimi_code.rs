//! Kimi Code adapter (MoonshotAI/kimi-code): session journals under
//! `$KIMI_CODE_HOME` (default `~/.kimi-code`, docs/en/configuration/
//! data-locations.md):
//!
//!   sessions/<workDirKey>/<sessionId>/agents/<agent>/wire.jsonl
//!   session_index.jsonl — {sessionId, sessionDir, workDir} per line
//!
//! `wire.jsonl` is the agent's durable event journal
//! (`packages/agent-core-v2/src/wire`). Token usage arrives as `usage.record`
//! records — ONE PER LLM CALL, ALWAYS A DELTA
//! (`agent/usage/usageOps.ts` + `human/usage/machine.ts` accumulate them):
//!
//!   {"type":"usage.record","agentId":"main","model":"kimi-k2",
//!    "usage":{"inputOther":10,"output":5,"inputCacheRead":0,
//!             "inputCacheCreation":0},
//!    "usageScope":"turn"|"session","time":1779256800302}
//!
//! `usageScope` only labels WHERE the call originated (turn vs session-level
//! helper), it never changes the delta semantics → count every record,
//! never filter on scope (filtering would silently drop non-turn calls).
//! `TokenUsage` has only those four fields — Kimi records NO reasoning split.
//!
//! Project: `config.update` records carry `cwd` (in-file, authoritative);
//! fall back to `session_index.jsonl` (sessionId → workDir). The resolved
//! value is cached in `adapter_state` since append-mode segments start past
//! the `config.update` header line.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{num, text};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct KimiCode;

/// `$KIMI_CODE_HOME` > `~/.kimi-code`.
fn data_root() -> PathBuf {
    if let Ok(dir) = std::env::var("KIMI_CODE_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    crate::sync::home(".kimi-code")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WireState {
    /// Resolved workdir — cached so append segments (which start past the
    /// `config.update` header) keep reporting the right project.
    #[serde(default)]
    project: Option<String>,
}

impl SourceAdapter for KimiCode {
    fn id(&self) -> &'static str {
        apps::KIMI_CODE
    }
    fn display_name(&self) -> &'static str {
        "Kimi Code"
    }
    /// Per-LLM-call token deltas, vendor-recorded — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![data_root().join("sessions")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        // sessions/<wd>/<sid>/agents/<agent>/wire.jsonl — depth 6 also covers
        // any nested agent dirs the layout may grow.
        Ok(
            crate::sync::collect_files(&data_root().join("sessions"), "jsonl", 6)
                .into_iter()
                .filter(|p| p.file_name().is_some_and(|n| n == "wire.jsonl"))
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
        prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut out = ScanOutcome {
            consumed,
            ..Default::default()
        };
        // <sessionId>/agents/<agent>/wire.jsonl
        // <sessionId>/agents/<agent>/wire.jsonl — the agent name lives inside
        // item.key (the file path), so dedup keys stay unique across agents.
        let session_id = item
            .path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string());

        let mut state: WireState = prior_state
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

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
            match v.get("type").and_then(Value::as_str) {
                // First config.update seen wins — a session can emit several.
                Some("config.update") if state.project.is_none() => {
                    state.project = text(&v["cwd"]);
                }
                Some("usage.record") => {
                    let u = &v["usage"];
                    if !u.is_object() {
                        continue;
                    }
                    let input = num(&u["inputOther"]);
                    let output = num(&u["output"]);
                    let cache_read = num(&u["inputCacheRead"]);
                    let cache_write = num(&u["inputCacheCreation"]);
                    if input + output + cache_read + cache_write == 0 {
                        out.skipped += 1;
                        continue;
                    }
                    out.events.push(UsageEvent {
                        dedup_key: format!("kimi_code:{}:{}", item.key, line_start),
                        app: apps::KIMI_CODE.into(),
                        session_id: session_id.clone(),
                        project: state.project.clone(),
                        model: text(&v["model"]),
                        request_model: text(&v["model"]),
                        ts_start: crate::normalize::epoch_ms(&v["time"]),
                        input_tokens: input, // inputOther is already non-cache input
                        output_tokens: output,
                        reasoning_tokens: 0, // Kimi TokenUsage has no reasoning split
                        cache_read_tokens: cache_read,
                        cache_write_5m_tokens: cache_write,
                        provenance: Provenance::LocalJsonl,
                        raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }

        // Project fallback for segments that never saw a config.update:
        // session_index.jsonl maps sessionId → workDir. Resolved AFTER the
        // line loop → back-fill events emitted while it was still unknown.
        if state.project.is_none()
            && let Some(sid) = &session_id
        {
            state.project = session_index_workdir(&item.path, sid);
        }
        if let Some(p) = &state.project {
            for ev in &mut out.events {
                if ev.project.is_none() {
                    ev.project = Some(p.clone());
                }
            }
        }
        if state.project.is_some() {
            out.new_state = Some(serde_json::to_string(&state)?);
        }
        Ok(out)
    }
}

/// `session_index.jsonl` sits next to `sessions/` — the wire file path is
/// `<root>/sessions/<wd>/<sid>/agents/<agent>/wire.jsonl`, so the data root is
/// six ancestors up (self, agent, agents, sid, wd, sessions, root).
fn session_index_workdir(wire: &Path, session_id: &str) -> Option<String> {
    let root = wire.ancestors().nth(6)?;
    let idx = root.join("session_index.jsonl");
    let content = std::fs::read_to_string(idx).ok()?;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("sessionId").and_then(Value::as_str) == Some(session_id) {
            return text(&v["workDir"]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tag` must differ per test — same-process tests run in parallel and
    /// would otherwise share (and clobber) one fixture dir.
    fn wire_dir(tag: &str) -> (TmpDir, PathBuf) {
        let root = std::env::temp_dir().join(format!("gtt-kimi-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root
            .join("sessions")
            .join("wd_demo_abcdef123456")
            .join("sess-9")
            .join("agents")
            .join("main");
        std::fs::create_dir_all(&dir).unwrap();
        (TmpDir(root), dir)
    }

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn usage_records_are_per_call_deltas() {
        let (_root, dir) = wire_dir("deltas");
        let file = dir.join("wire.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"metadata","protocol_version":"1.1","created_at":1779256791085}"#,
                "\n",
                r#"{"type":"config.update","cwd":"/tmp/work","time":1779256791100}"#,
                "\n",
                r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":10,"output":5,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256800302}"#,
                "\n",
                // session-scoped records are deltas too — must be counted.
                r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":7,"output":3,"inputCacheRead":4,"inputCacheCreation":2},"usageScope":"session","time":1779256800500}"#,
                "\n",
            ),
        )
        .unwrap();
        let item = SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.clone(),
            kind: SourceKind::Jsonl,
        };
        let data = std::fs::read(&file).unwrap();
        let out = KimiCode.parse_jsonl(&item, 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 2);
        let e = &out.events[0];
        assert_eq!(e.app, apps::KIMI_CODE);
        assert_eq!(e.session_id.as_deref(), Some("sess-9"));
        assert_eq!(e.project.as_deref(), Some("/tmp/work"));
        assert_eq!(e.model.as_deref(), Some("kimi-k2"));
        assert_eq!(e.input_tokens, 10);
        assert_eq!(e.output_tokens, 5);
        let e2 = &out.events[1];
        assert_eq!(e2.input_tokens, 7);
        assert_eq!(e2.cache_read_tokens, 4);
        assert_eq!(e2.cache_write_5m_tokens, 2);
        // project resolved → cached in adapter_state for append segments
        assert!(out.new_state.unwrap().contains("/tmp/work"));
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn append_segment_uses_cached_project() {
        let (_root, dir) = wire_dir("append");
        let file = dir.join("wire.jsonl");
        // Segment starting mid-file — no config.update in view.
        let seg = concat!(
            r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":1,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1779256810000}"#,
            "\n"
        );
        std::fs::write(&file, seg).unwrap();
        let item = SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.clone(),
            kind: SourceKind::Jsonl,
        };
        let prior = r#"{"project":"/cached/dir"}"#;
        let data = std::fs::read(&file).unwrap();
        let out = KimiCode
            .parse_jsonl(&item, 500, &data, Some(prior))
            .unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].project.as_deref(), Some("/cached/dir"));
    }

    #[test]
    fn project_falls_back_to_session_index() {
        let (root, dir) = wire_dir("index");
        // No config.update anywhere — index must supply workDir.
        std::fs::write(
            root.0.join("session_index.jsonl"),
            r#"{"sessionId":"sess-9","sessionDir":"sessions/wd_demo_abcdef123456/sess-9","workDir":"D:/idx/proj"}"#
                .to_string()
                + "\n",
        )
        .unwrap();
        let file = dir.join("wire.jsonl");
        std::fs::write(
            &file,
            r#"{"type":"usage.record","model":"kimi-k2","usage":{"inputOther":2,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"time":1779256800000}"#
                .to_string()
                + "\n",
        )
        .unwrap();
        let item = SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.clone(),
            kind: SourceKind::Jsonl,
        };
        let data = std::fs::read(&file).unwrap();
        let out = KimiCode.parse_jsonl(&item, 0, &data, None).unwrap();
        assert_eq!(out.events[0].project.as_deref(), Some("D:/idx/proj"));
    }

    #[test]
    fn zero_usage_and_partial_tail_are_safe() {
        let (_root, dir) = wire_dir("tail");
        let file = dir.join("wire.jsonl");
        let body = concat!(
            r#"{"type":"usage.record","usage":{"inputOther":0,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"time":1}"#,
            "\n",
            r#"{"type":"usage.record","usage":{"inputOther":9"# // no \n → tail left unconsumed
        );
        std::fs::write(&file, body).unwrap();
        let item = SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.clone(),
            kind: SourceKind::Jsonl,
        };
        let data = std::fs::read(&file).unwrap();
        let out = KimiCode.parse_jsonl(&item, 0, &data, None).unwrap();
        assert!(out.events.is_empty());
        assert_eq!(out.skipped, 1);
        // Consumed stops right after the last newline — partial tail deferred.
        let last_nl = body.rfind('\n').unwrap() + 1;
        assert_eq!(out.consumed as usize, last_nl);
        assert!(last_nl < data.len());
    }
}
