//! WorkBuddy adapter (spec §6.6): `~/.workbuddy/projects/**/*.jsonl`
//! (including nested `subagents/agent-*.jsonl`) — every record carrying
//! `providerData.rawUsage` counts, deduped by `providerData.messageId`
//! (verified: 4819 rows, 0 repeats; `traceId`/`conversationRequestId` are
//! per-session groups, NOT response ids).
//!
//! Aux SQLite `~/.workbuddy/workbuddy.db session_usage`: per-session context
//! watermark (`used`/`size`) → quota_snapshots, high-water mark on updated_at.
//! `credit_json` keys are md5 → unresolvable to model names; per-turn credits
//! already flow through `rawUsage.credit`.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{Provenance, QuotaSnapshot, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, input_excludes_cache, num, text};
use crate::store::Store;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct WorkBuddy;

impl SourceAdapter for WorkBuddy {
    fn id(&self) -> &'static str {
        apps::WORKBUDDY
    }
    fn display_name(&self) -> &'static str {
        "WorkBuddy"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".workbuddy")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut out: Vec<SourceItem> =
            crate::sync::collect_files(&crate::sync::home(".workbuddy/projects"), "jsonl", 5)
                .into_iter()
                .map(|p| SourceItem {
                    key: p.to_string_lossy().to_string(),
                    path: p,
                    kind: SourceKind::Jsonl,
                })
                .collect();
        let db = crate::sync::home(".workbuddy/workbuddy.db");
        if db.exists() {
            out.push(SourceItem {
                key: db.to_string_lossy().to_string(),
                path: db,
                kind: SourceKind::Sqlite,
            });
        }
        Ok(out)
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
        let session_id = item
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // projects/<project-slug>/<session>.jsonl or .../<session>/subagents/agent-*.jsonl
        let project = item
            .path
            .strip_prefix(crate::sync::home(".workbuddy/projects"))
            .ok()
            .and_then(|rel| rel.components().next())
            .map(|c| c.as_os_str().to_string_lossy().to_string());

        let mut pos = 0u64;
        for line in seg.split(|&b| b == b'\n') {
            let line_start = from + pos;
            pos += line.len() as u64 + 1;
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() || !line.windows(8).any(|w| w == b"rawUsage") {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let pd = &v["providerData"];
            let raw = &pd["rawUsage"];
            if !raw.is_object() {
                continue;
            }
            // Cache-read variants across providers (spec §6.6): take the max.
            let cache_read = [
                "cache_read_input_tokens",
                "prompt_cache_hit_tokens",
                "cached_tokens",
            ]
            .iter()
            .map(|k| num(&raw[k]))
            .chain(std::iter::once(num(&raw["prompt_tokens_details"]["cached_tokens"])))
            .max()
            .unwrap_or(0);
            let cache_write = [
                "cache_creation_input_tokens",
                "prompt_cache_write_tokens",
            ]
            .iter()
            .map(|k| num(&raw[k]))
            .max()
            .unwrap_or(0);
            let prompt = num(&raw["prompt_tokens"]);
            let reasoning = num(&raw["completion_tokens_details"]["reasoning_tokens"])
                .max(num(&raw["completion_thinking_tokens"]));
            let response_id = text(&pd["messageId"]).unwrap_or_else(|| {
                format!("{}@{}", item.key, line_start)
            });
            out.events.push(UsageEvent {
                dedup_key: format!("workbuddy:{response_id}"),
                app: apps::WORKBUDDY.into(),
                session_id: Some(session_id.clone()),
                project: project.clone(),
                model: text(&pd["model"]),
                request_model: text(&pd["requestModelId"]).or_else(|| text(&pd["requestModelName"])),
                ts_start: epoch_ms(&v["timestamp"]),
                input_tokens: input_excludes_cache(prompt, cache_read, cache_write),
                output_tokens: num(&raw["completion_tokens"]),
                reasoning_tokens: reasoning,
                cache_read_tokens: cache_read,
                cache_write_5m_tokens: cache_write,
                credits: fnum(&raw["credit"]),
                provenance: Provenance::LocalJsonl,
                raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                ..Default::default()
            });
        }
        Ok(out)
    }

    /// `session_usage` → per-session context watermark quota rows.
    /// `used`/`size` are the context-window fill of that session.
    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let since = (cur.offset as i64).saturating_sub(1000);
        let conn = super::opencode::open_ro(&item.path)?;
        let mut st = conn.prepare(
            "SELECT session_id, used, size, updated_at FROM session_usage
             WHERE updated_at > ?1",
        )?;
        let rows = st.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = ScanOutcome::default();
        let mut max_updated = cur.offset as i64;
        for row in rows {
            let (_sid, used, size, updated) = row?;
            max_updated = max_updated.max(updated);
            if size <= 0 {
                continue;
            }
            out.quotas.push(QuotaSnapshot {
                app: apps::WORKBUDDY.into(),
                // latest_quotas groups by (app, window_kind) → surfaces the most
                // recently-updated session's context fill.
                account: None,
                captured_at: updated,
                window_kind: "session_ctx".into(),
                used: Some(used as f64),
                limit_value: Some(size as f64),
                used_percent: Some(used as f64 * 100.0 / size as f64),
                resets_at: None,
                raw_json: None,
            });
        }
        if (max_updated as u64) > cur.offset {
            store.save_cursor(
                self.id(),
                &item.key,
                &item.path,
                max_updated as u64,
                0,
                None,
            )?;
        }
        Ok(out)
    }
}
