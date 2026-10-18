//! CodeBuddy CLI adapter (spec §6.7) — Tencent's `@tencent-ai/codebuddy-code`.
//!
//! Local transcripts (Claude-Code-like layout, no config-dir override):
//!   `~/.codebuddy/projects/<encoded-cwd>/<sessionId>.jsonl`
//!   `~/.codebuddy/projects/<encoded-cwd>/<sessionId>/subagents/agent-*.jsonl`
//!
//! The record shape is NOT Claude Code's: the CLI and the WorkBuddy desktop
//! app are two front-ends of the same Tencent agent runtime, so every billed
//! LLM call lands as a `type:"function_call"` row carrying the vendor's own
//! counts under `providerData.rawUsage` (assistant `message` rows carry no
//! counts of their own), deduped by `providerData.messageId`. Measured on this
//! machine: 427 usage rows / 427 distinct ids / 0 repeats — hence the parse is
//! delegated to `workbuddy::parse_raw_usage`, the §6.6 parser.
//!
//! `session_id` stays the file stem (parity with WorkBuddy: a subagent file
//! reports its own name) and `project` the encoded-cwd slug.
//!
//! Three sibling products keep three separate ledgers — `codebuddy_cli` here,
//! `codebuddy_ide` (the VS Code extension, `%LOCALAPPDATA%`) and `workbuddy`
//! (the desktop app) — never folded together.

use super::workbuddy;
use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::apps;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Ledger prefix — distinct from `workbuddy:` / `codebuddy_ide:` so a shared
/// `messageId` can never collide across products.
const DEDUP_PREFIX: &str = "codebuddy_cli";

pub struct CodeBuddyCli;

/// `~/.codebuddy/projects` — JSONL root only. The rest of `~/.codebuddy`
/// (`blobs/`, `traces/`, `file-history/`, `logs/`) holds no usage and churns
/// heavily, so it is neither scanned nor watched.
fn projects_root() -> PathBuf {
    crate::sync::home(".codebuddy/projects")
}

impl CodeBuddyCli {
    /// Parse one segment against an explicit root: production always passes
    /// `projects_root()`, tests point it at a fixture directory.
    fn parse_segment(
        &self,
        root: &Path,
        item: &SourceItem,
        from: u64,
        data: &[u8],
    ) -> Result<ScanOutcome> {
        workbuddy::parse_raw_usage(item, from, data, apps::CODEBUDDY_CLI, DEDUP_PREFIX, root)
    }
}

impl SourceAdapter for CodeBuddyCli {
    fn id(&self) -> &'static str {
        apps::CODEBUDDY_CLI
    }
    fn display_name(&self) -> &'static str {
        "CodeBuddy CLI"
    }
    /// Per-LLM-call counts recorded by the vendor — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![projects_root()]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(crate::sync::collect_files(&projects_root(), "jsonl", 5)
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
        self.parse_segment(&projects_root(), item, from, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;
    use crate::store::Store;
    use serde_json::{Value, json};

    /// `~/.codebuddy/projects/<slug>/<session>` fixture (the root doubles as
    /// the projects root the parser strips to derive the project slug).
    fn fixture(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gtt-cbcli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("f-Agentic Project-GlobalTokenTracker");
        std::fs::create_dir_all(&dir).unwrap();
        (root, dir)
    }

    fn item_for(file: &Path) -> SourceItem {
        SourceItem {
            key: file.to_string_lossy().to_string(),
            path: file.to_path_buf(),
            kind: SourceKind::Jsonl,
        }
    }

    /// A billed row exactly as the runtime writes it.
    fn call_row(record_id: &str, msg_id: &str, ts: i64, raw: Value) -> String {
        json!({
            "id": record_id,
            "timestamp": ts,
            "type": "function_call",
            "name": "Glob",
            "providerData": {
                "messageId": msg_id,
                "model": "glm-5.3",
                "requestModelId": "glm-5.3",
                "requestModelName": "GLM-5.3",
                "agent": "cli",
                "rawUsage": raw,
            },
            "sessionId": "01a0fb9b-2185-7b5e-964e-7ca474f93f0f",
            "cwd": "f:\\Agentic Project\\GlobalTokenTracker",
        })
        .to_string()
    }

    #[test]
    fn function_call_raw_usage_maps_to_exact_event() {
        let (root, dir) = fixture("usage");
        let file = dir.join("01a0fb9b-2185-7b5e-964e-7ca474f93f0f.jsonl");
        let body = [
            // Assistant text row: no counts of its own → no event.
            json!({
                "id": "a",
                "timestamp": 1791201456620i64,
                "type": "message",
                "role": "assistant",
                "providerData": {"messageId": "m1", "model": "glm-5.3"},
                "sessionId": "01a0fb9b-2185-7b5e-964e-7ca474f93f0f",
            })
            .to_string(),
            // Billed call: `prompt_tokens` is the whole prompt (6592 of it hit
            // the cache), DeepSeek-style `prompt_cache_hit_tokens` carries the
            // read count and `completion_tokens` includes the 245 reasoning.
            call_row(
                "b",
                "m1",
                1791201456650,
                json!({
                    "prompt_tokens": 27023,
                    "completion_tokens": 298,
                    "total_tokens": 27321,
                    "prompt_cache_hit_tokens": 6592,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "prompt_cache_write_tokens": 0,
                    "completion_tokens_details": {"reasoning_tokens": 245},
                    "completion_thinking_tokens": 245,
                    "credit": 3.7,
                }),
            ),
            // Anthropic-shaped cache keys are honoured too (max of variants).
            call_row(
                "c",
                "m2",
                1791201456900,
                json!({
                    "prompt_tokens": 5000,
                    "completion_tokens": 100,
                    "cache_read_input_tokens": 3000,
                    "cache_creation_input_tokens": 500,
                    "credit": 0.5,
                }),
            ),
        ]
        .join("\n")
            + "\n";
        std::fs::write(&file, &body).unwrap();

        let item = item_for(&file);
        let data = std::fs::read(&file).unwrap();
        let out = CodeBuddyCli.parse_segment(&root, &item, 0, &data).unwrap();

        assert_eq!(out.events.len(), 2);
        assert_eq!(out.consumed, data.len() as u64);

        let e = &out.events[0];
        assert_eq!(e.app, apps::CODEBUDDY_CLI);
        assert_eq!(e.dedup_key, "codebuddy_cli:m1");
        assert_eq!(
            e.session_id.as_deref(),
            Some("01a0fb9b-2185-7b5e-964e-7ca474f93f0f")
        );
        assert_eq!(
            e.project.as_deref(),
            Some("f-Agentic Project-GlobalTokenTracker")
        );
        assert_eq!(e.model.as_deref(), Some("glm-5.3"));
        assert_eq!(e.request_model.as_deref(), Some("glm-5.3"));
        assert_eq!(e.ts_start, Some(1791201456650));
        // 27023 prompt − 6592 cache read − 0 cache write = fresh input.
        assert_eq!(e.input_tokens, 20431);
        assert_eq!(e.output_tokens, 298);
        assert_eq!(e.reasoning_tokens, 245);
        assert_eq!(e.cache_read_tokens, 6592);
        assert_eq!(e.cache_write_5m_tokens, 0);
        assert_eq!(e.credits, Some(3.7));
        assert_eq!(e.provenance, Provenance::LocalJsonl);
        assert!(e.is_billable());
        assert!(
            e.raw_ref
                .as_deref()
                .is_some_and(|r| r.starts_with(&file.to_string_lossy().to_string()))
        );

        let e2 = &out.events[1];
        assert_eq!(e2.dedup_key, "codebuddy_cli:m2");
        assert_eq!(e2.input_tokens, 5000 - 3000 - 500);
        assert_eq!(e2.cache_read_tokens, 3000);
        assert_eq!(e2.cache_write_5m_tokens, 500);
        assert_eq!(e2.output_tokens, 100);
        assert_eq!(e2.credits, Some(0.5));

        std::fs::remove_dir_all(&root).ok();
    }

    /// Subagent transcripts (`<session>/subagents/agent-*.jsonl`) sit one level
    /// deeper than the session file — the discovery depth must still reach them
    /// and the project slug must stay the encoded cwd.
    #[test]
    fn subagent_transcript_maps_to_parent_project() {
        let (root, dir) = fixture("subagent");
        let sub = dir.join("01a10be8-04ba-7eee-8477-dc338730c41e/subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("agent-68d7904a.jsonl");
        std::fs::write(
            &file,
            call_row(
                "d",
                "m3",
                1791201460000,
                json!({"prompt_tokens": 1000, "completion_tokens": 20, "credit": 0.1}),
            ) + "\n",
        )
        .unwrap();

        // Same call `discover()` makes.
        let found = crate::sync::collect_files(&root, "jsonl", 5);
        assert_eq!(found, vec![file.clone()]);

        let item = item_for(&file);
        let data = std::fs::read(&file).unwrap();
        let out = CodeBuddyCli.parse_segment(&root, &item, 0, &data).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].dedup_key, "codebuddy_cli:m3");
        assert_eq!(out.events[0].session_id.as_deref(), Some("agent-68d7904a"));
        assert_eq!(
            out.events[0].project.as_deref(),
            Some("f-Agentic Project-GlobalTokenTracker")
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// The runtime stamps one `messageId` on every row of a call; re-emitting
    /// it must reuse the key so the store's completeness-UPSERT merges instead
    /// of appending a second row.
    #[test]
    fn repeated_message_id_merges_into_one_ledger_row() {
        let (root, dir) = fixture("dedup");
        let file = dir.join("sess.jsonl");
        let raw = json!({"prompt_tokens": 1000, "completion_tokens": 20, "credit": 0.1});
        std::fs::write(
            &file,
            [
                call_row("e", "m4", 1791201460000, raw.clone()),
                call_row("f", "m4", 1791201460000, raw),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let item = item_for(&file);
        let data = std::fs::read(&file).unwrap();
        let out = CodeBuddyCli.parse_segment(&root, &item, 0, &data).unwrap();
        assert_eq!(out.events.len(), 2);
        assert_eq!(out.events[0].dedup_key, out.events[1].dedup_key);

        let store = Store::open_memory().unwrap();
        assert!(store.upsert_event(&out.events[0]).unwrap());
        // The re-emitted row lands on the same key: whatever the store does
        // with it (skip, or refresh `raw_ref`), the ledger keeps ONE call.
        store.upsert_event(&out.events[1]).unwrap();
        assert_eq!(store.event_count(None, None).unwrap(), 1);
        let t = store.totals(None, None, None, None).unwrap();
        assert_eq!(t.input_tokens, 1000);
        assert_eq!(t.credits, 0.1);

        std::fs::remove_dir_all(&root).ok();
    }

    /// A half-written trailing line stays unconsumed so the next pass re-reads
    /// it whole (the engine's cursor contract).
    #[test]
    fn trailing_partial_line_is_not_consumed() {
        let (root, dir) = fixture("partial");
        let file = dir.join("sess.jsonl");
        let complete = call_row(
            "g",
            "m5",
            1791201460000,
            json!({"prompt_tokens": 800, "completion_tokens": 10, "credit": 0.2}),
        ) + "\n";
        let partial = "{\"id\":\"h\",\"type\":\"function_call\",\"providerData\":{\"rawUsa";
        std::fs::write(&file, format!("{complete}{partial}")).unwrap();

        let item = item_for(&file);
        let data = std::fs::read(&file).unwrap();
        let out = CodeBuddyCli.parse_segment(&root, &item, 0, &data).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.consumed, complete.len() as u64);

        std::fs::remove_dir_all(&root).ok();
    }
}
