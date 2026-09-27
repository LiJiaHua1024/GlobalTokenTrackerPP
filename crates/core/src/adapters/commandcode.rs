//! Command Code adapter (CommandCodeAI/command-code; shipped as the
//! `command-code` npm package — dist bundle is the source of truth).
//!
//! Session transcripts are append-only JSONL:
//!   `~/.commandcode/projects/<slugify(cwd)>/<sessionId>.jsonl`
//! Sibling files `<id>.checkpoints.jsonl` / `<id>.prompts.jsonl` /
//! `<id>.v2.bak` are filtered out (isSessionTranscriptFileName).
//!
//! First line is the session header:
//!   {type:"session", version, id, timestamp:ISO, cwd, parentSession?}
//! Usage rides on assistant message entries — one `model_request_end` is
//! folded into exactly one assistant `message` entry by createSessionRecorder:
//!   {type:"message", id, parentId, timestamp:ISO, message:{role:"assistant",…},
//!    usage:{inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens,
//!           cacheWriteTokens1h?, costUsd?}, model, effort?}
//!
//! Token semantics (bundle `estimateSessionCostUsd`):
//!   freshInput = max(0, inputTokens - cacheReadTokens - cacheWriteTokens)
//!   write1h    = min(cacheWriteTokens1h ?? 0, cacheWriteTokens)
//!   write5m    = cacheWriteTokens - write1h
//! i.e. `inputTokens` INCLUDES cache traffic and `cacheWriteTokens` is the
//! TOTAL write count whose 1h tier is carved out by `cacheWriteTokens1h`.
//! `costUsd` is the tool's own display-rate estimate → ProviderReported.
//!
//! Append-mode segments start past the `session` header — the resolved
//! `cwd`/`session id` are cached in adapter_state (same pattern as
//! kimi_code) and derived fallbacks (dir slug, file stem) stand in until
//! the header is read.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, input_excludes_cache, num, text};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub struct CommandCode;

fn data_root() -> PathBuf {
    crate::sync::home(".commandcode/projects")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionState {
    /// Session header `cwd` — authoritative project. Cached for append
    /// segments that start past the header line.
    #[serde(default)]
    cwd: Option<String>,
}

impl SourceAdapter for CommandCode {
    fn id(&self) -> &'static str {
        apps::COMMANDCODE
    }
    fn display_name(&self) -> &'static str {
        "Command Code"
    }
    /// Per-LLM-call usage on assistant entries, vendor-recorded — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".commandcode")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        // projects/<slug>/<session>.jsonl; sidecars are excluded by name.
        Ok(crate::sync::collect_files(&data_root(), "jsonl", 3)
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .map(|n| {
                        let n = n.to_string_lossy();
                        !n.contains(".checkpoints.")
                            && !n.contains(".prompts.")
                            && !n.ends_with(".v2.bak")
                    })
                    .unwrap_or(false)
            })
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
        let mut out = ScanOutcome {
            consumed,
            ..Default::default()
        };
        let session_id = item
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let slug = item
            .path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string());

        let mut state: SessionState = prior_state
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
                // Session header — first line; also emitted on forks/resumes.
                Some("session") => {
                    if state.cwd.is_none() {
                        state.cwd = text(&v["cwd"]);
                    }
                }
                Some("message") => {
                    if v["message"]["role"].as_str() != Some("assistant") {
                        continue;
                    }
                    let u = &v["usage"];
                    if !u.is_object() {
                        continue;
                    }
                    let input_total = num(&u["inputTokens"]);
                    let output = num(&u["outputTokens"]);
                    let cache_read = num(&u["cacheReadTokens"]);
                    let cw_total = num(&u["cacheWriteTokens"]);
                    let cw_1h = num(&u["cacheWriteTokens1h"]).min(cw_total);
                    let cw_5m = cw_total - cw_1h;
                    let cost = fnum(&u["costUsd"]);
                    if input_total + output + cache_read + cw_total == 0 && cost.is_none() {
                        out.skipped += 1;
                        continue;
                    }
                    let id = v
                        .get("id")
                        .map(|x| x.to_string().trim_matches('"').to_string())
                        .unwrap_or_else(|| format!("{line_start}"));
                    out.events.push(UsageEvent {
                        dedup_key: format!("commandcode:{session_id}:{id}"),
                        app: apps::COMMANDCODE.into(),
                        session_id: Some(session_id.clone()),
                        // Header cwd wins; dir slug is the fallback label.
                        project: state.cwd.clone().or_else(|| slug.clone()),
                        model: text(&v["model"]),
                        request_model: text(&v["model"]),
                        ts_start: epoch_ms(&v["timestamp"]),
                        input_tokens: input_excludes_cache(input_total, cache_read, cw_total),
                        output_tokens: output,
                        reasoning_tokens: 0, // not broken out by Command Code
                        cache_read_tokens: cache_read,
                        cache_write_5m_tokens: cw_5m,
                        cache_write_1h_tokens: cw_1h,
                        cost_usd: cost,
                        cost_source: cost.map(|_| CostSource::ProviderReported),
                        provenance: Provenance::LocalJsonl,
                        raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }

        if state.cwd.is_some() {
            out.new_state = Some(serde_json::to_string(&state)?);
            // Header may have resolved after early events were emitted.
            for ev in &mut out.events {
                if ev.project.is_none() || ev.project == slug {
                    ev.project = state.cwd.clone();
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(tag: &str) -> (TmpDir, PathBuf) {
        let root = std::env::temp_dir().join(format!("gtt-cc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("projects/D--proj-demo");
        std::fs::create_dir_all(&dir).unwrap();
        (TmpDir(root), dir)
    }

    #[test]
    fn assistant_usage_and_cache_split() {
        let (_root, dir) = fixture("usage");
        let file = dir.join("sess-1.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"session","version":3,"id":"sess-1","timestamp":"2026-09-27T10:00:00Z","cwd":"D:\\proj\\demo"}"#,
                "\n",
                r#"{"type":"message","id":7,"timestamp":"2026-09-27T10:00:05Z","message":{"role":"user","content":[]}}"#,
                "\n",
                r#"{"type":"message","id":8,"timestamp":"2026-09-27T10:00:10Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]},"usage":{"inputTokens":10000,"outputTokens":200,"cacheReadTokens":8000,"cacheWriteTokens":500,"cacheWriteTokens1h":100,"costUsd":0.021},"model":"cc-claude-sonnet"}"#,
                "\n",
                r#"{"type":"message","id":9,"timestamp":"2026-09-27T10:00:20Z","message":{"role":"assistant","content":[]}}"#,
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
        let out = CommandCode.parse_jsonl(&item, 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!(e.app, apps::COMMANDCODE);
        assert_eq!(e.dedup_key, "commandcode:sess-1:8");
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));
        assert_eq!(e.project.as_deref(), Some("D:\\proj\\demo"));
        assert_eq!(e.model.as_deref(), Some("cc-claude-sonnet"));
        // 10000 total − 8000 read − 500 write = 1500 fresh input
        assert_eq!(e.input_tokens, 1500);
        assert_eq!(e.output_tokens, 200);
        assert_eq!(e.cache_read_tokens, 8000);
        assert_eq!(e.cache_write_5m_tokens, 400);
        assert_eq!(e.cache_write_1h_tokens, 100);
        assert_eq!(e.cost_usd, Some(0.021));
        assert_eq!(e.cost_source, Some(CostSource::ProviderReported));
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn append_segment_keeps_cached_cwd() {
        let (_root, dir) = fixture("append");
        let file = dir.join("sess-2.jsonl");
        let seg = concat!(
            r#"{"type":"message","id":3,"timestamp":"2026-09-27T11:00:00Z","message":{"role":"assistant","content":[]},"usage":{"inputTokens":100,"outputTokens":10,"cacheReadTokens":0,"cacheWriteTokens":0},"model":"cc-kimi"}"#,
            "\n"
        );
        std::fs::write(&file, seg).unwrap();
        let item = SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.clone(),
            kind: SourceKind::Jsonl,
        };
        let prior = r#"{"cwd":"D:/cached/work"}"#;
        let out = CommandCode
            .parse_jsonl(&item, 1000, seg.as_bytes(), Some(prior))
            .unwrap();
        assert_eq!(out.events[0].project.as_deref(), Some("D:/cached/work"));
        // No header seen → project is the cached cwd, not the slug.
    }

    #[test]
    fn slug_fallback_and_zero_usage_skip() {
        let (_root, dir) = fixture("slug");
        let file = dir.join("sess-3.jsonl");
        std::fs::write(
            &file,
            concat!(
                // No session header at all → slug label stands in.
                r#"{"type":"message","id":1,"timestamp":"2026-09-27T12:00:00Z","message":{"role":"assistant","content":[]},"usage":{"inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0}}"#,
                "\n",
                r#"{"type":"message","id":2,"timestamp":"2026-09-27T12:00:01Z","message":{"role":"assistant","content":[]},"usage":{"inputTokens":50,"outputTokens":5,"cacheReadTokens":0,"cacheWriteTokens":0}}"#,
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
        let out = CommandCode.parse_jsonl(&item, 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.events[0].project.as_deref(), Some("D--proj-demo"));
        assert_eq!(out.events[0].input_tokens, 50);
        assert!(out.events[0].cost_usd.is_none());
        assert!(out.events[0].cost_source.is_none());
    }
}
