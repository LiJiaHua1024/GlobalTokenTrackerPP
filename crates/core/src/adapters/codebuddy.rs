//! CodeBuddy adapter (spec §6.11) — Tencent's `tencent-cloud.coding-copilot`
//! extension, hosted by CodeBuddy IDE and VS Code. Sibling of WorkBuddy (same
//! vendor/account plane) but a **separate product**: it gets its own app id
//! and its own ledger rows, never folded into `workbuddy`.
//!
//! Local layout (Windows/macOS share `dirs::data_local_dir()`):
//!   `CodeBuddyExtension/Data/<user>/<host>/<acct>/history/<ws>/<conv>/`
//!     `index.json`      — conversation list (fingerprint file for the cursor)
//!     `messages/*.json` — one message per file; `role`/`createdAt`/`extra`
//!
//! Assistant messages carry `extra` (JSON string) with a cumulative
//! `statsSnapshot`: {inputTokens, outputTokens, cachedInputTokens,
//! cacheWriteTokens, thinkingTokens, elapsedMs, credit}. The last snapshot in
//! a conversation IS its exact token total; component-wise deltas between
//! consecutive snapshots bundle one or more LLM calls into one event — totals
//! stay exact, per-call granularity is traded away (Capability::Estimate).
//!
//! Every conversation dir is one `Sqlite`-kind SourceItem so `scan_sqlite`
//! owns the watermark: `adapter_state` persists processed file names and the
//! last cumulative snapshot, making incremental scans O(new files).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, num, text};
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct CodeBuddyIde {
    /// `session:`→cwd map is per-scan stable; without this cache the IDE
    /// session db was walked once per conversation (the dominant scan cost).
    cwds: std::sync::Mutex<Option<BTreeMap<String, String>>>,
}

impl CodeBuddyIde {
    pub fn new() -> Self {
        Self {
            cwds: std::sync::Mutex::new(None),
        }
    }
}

impl Default for CodeBuddyIde {
    fn default() -> Self {
        Self::new()
    }
}

/// `…/CodeBuddyExtension/Data` — hosts `Data/<user>/<host>/<acct>/history`.
fn data_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| crate::sync::home(".local/share"))
        .join("CodeBuddyExtension/Data")
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// conversationId → cwd, best-effort from the IDE's own session index db(s)
/// (`<config>/<Product>/codebuddy-sessions.vscdb` next to globalStorage).
fn session_dirs() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(cfg) = dirs::config_dir() else {
        return map;
    };
    let Ok(products) = std::fs::read_dir(cfg) else {
        return map;
    };
    for db in products
        .flatten()
        .map(|e| e.path().join("codebuddy-sessions.vscdb"))
    {
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            format!("file:{}?mode=ro", db.to_string_lossy().replace('\\', "/")),
            rusqlite::OpenFlags::SQLITE_OPEN_URI | rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut st) = conn.prepare("SELECT key, value FROM ItemTable") else {
            continue;
        };
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (key, val) = row;
                let Some(id) = key.strip_prefix("session:") else {
                    continue;
                };
                if let Ok(v) = serde_json::from_str::<Value>(&val)
                    && let Some(cwd) = text(&v["cwd"])
                {
                    map.insert(id.to_string(), cwd);
                }
            }
        }
    }
    map
}

/// Cumulative counters carried by `statsSnapshot` — component-wise `max`
/// against the persisted watermark yields the delta for a new message.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
struct Cum {
    input: u64,
    output: u64,
    cached: u64,
    cache_write: u64,
    thinking: u64,
    elapsed_ms: i64,
    credit: f64,
}

impl Cum {
    fn from_snap(s: &Value) -> Self {
        Self {
            input: num(&s["inputTokens"]),
            output: num(&s["outputTokens"]),
            cached: num(&s["cachedInputTokens"]),
            cache_write: num(&s["cacheWriteTokens"]),
            thinking: num(&s["thinkingTokens"]),
            elapsed_ms: s["elapsedMs"].as_i64().unwrap_or(0),
            credit: crate::normalize::fnum(&s["credit"]).unwrap_or(0.0),
        }
    }
    /// `self - base` with per-component clamp at 0 (a late-discovered older
    /// message would otherwise produce negative deltas).
    fn delta(&self, base: &Cum) -> Cum {
        Cum {
            input: self.input.saturating_sub(base.input),
            output: self.output.saturating_sub(base.output),
            cached: self.cached.saturating_sub(base.cached),
            cache_write: self.cache_write.saturating_sub(base.cache_write),
            thinking: self.thinking.saturating_sub(base.thinking),
            elapsed_ms: self.elapsed_ms.saturating_sub(base.elapsed_ms),
            credit: (self.credit - base.credit).max(0.0),
        }
    }
    fn any(&self) -> bool {
        self.input > 0 || self.output > 0 || self.cached > 0 || self.cache_write > 0
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConvState {
    /// Processed message file names (stem) — append-only.
    seen: BTreeSet<String>,
    /// High-water cumulative snapshot; deltas are `snap - cum`.
    #[serde(default)]
    cum: Cum,
}

impl SourceAdapter for CodeBuddyIde {
    fn id(&self) -> &'static str {
        apps::CODEBUDDY_IDE
    }
    fn display_name(&self) -> &'static str {
        "CodeBuddy"
    }
    /// Totals are vendor-exact; events bundle ≥1 LLM call per delta.
    fn capability(&self) -> Capability {
        Capability::Estimate
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![
            data_root()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(data_root),
        ]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        // Data/<user>/<host>/<acct>/history/<ws>/<conv>/messages/
        let mut out = Vec::new();
        for user in subdirs(&data_root()) {
            for host in subdirs(&user) {
                for acct in subdirs(&host) {
                    for ws in subdirs(&acct.join("history")) {
                        for conv in subdirs(&ws) {
                            if conv.join("messages").is_dir() {
                                out.push(SourceItem {
                                    key: conv.to_string_lossy().to_string(),
                                    path: conv,
                                    kind: SourceKind::Sqlite,
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let mut state: ConvState = cur
            .state
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let conv = item
            .path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let project = {
            let mut g = self.cwds.lock().unwrap_or_else(|e| e.into_inner());
            g.get_or_insert_with(session_dirs)
                .get(&conv)
                .and_then(|cwd| {
                    Path::new(cwd)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                })
        };

        // New message files, oldest first so deltas walk the timeline forward.
        // Each candidate is read exactly once; unparseable files are still
        // marked seen (a corrupt stub shouldn't wedge the watermark).
        let mut files: Vec<(i64, String, PathBuf, Value)> =
            std::fs::read_dir(item.path.join("messages"))
                .map(|rd| {
                    rd.flatten()
                        .filter_map(|e| {
                            let p = e.path();
                            if p.extension().is_none_or(|x| x != "json") {
                                return None;
                            }
                            let stem = p.file_stem()?.to_string_lossy().to_string();
                            if state.seen.contains(&stem) {
                                return None;
                            }
                            let v: Value =
                                serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
                            Some((epoch_ms(&v["createdAt"]).unwrap_or(0), stem, p, v))
                        })
                        .collect()
                })
                .unwrap_or_default();
        files.sort_by_key(|(ts, ..)| *ts);
        let had_new = !files.is_empty();

        let mut out = ScanOutcome::default();
        for (_ts, stem, f, v) in files {
            state.seen.insert(stem.clone());
            if text(&v["role"]).as_deref() != Some("assistant") {
                continue;
            }
            let extra: Value = match v.get("extra").and_then(|e| e.as_str()) {
                Some(s) => serde_json::from_str(s).unwrap_or_default(),
                None => continue,
            };
            let snap_v = &extra["statsSnapshot"];
            if !snap_v.is_object() {
                // No cumulative counter — its tokens are already inside the
                // surrounding snapshots; `lastStep*` would double-count.
                continue;
            }
            let snap = Cum::from_snap(snap_v);
            let d = snap.delta(&state.cum);
            state.cum = Cum {
                input: state.cum.input.max(snap.input),
                output: state.cum.output.max(snap.output),
                cached: state.cum.cached.max(snap.cached),
                cache_write: state.cum.cache_write.max(snap.cache_write),
                thinking: state.cum.thinking.max(snap.thinking),
                elapsed_ms: state.cum.elapsed_ms.max(snap.elapsed_ms),
                credit: state.cum.credit.max(snap.credit),
            };
            if !d.any() {
                continue;
            }
            let msg_id = text(&extra["responseId"])
                .or_else(|| text(&extra["requestId"]))
                .unwrap_or_else(|| stem.clone());
            out.events.push(UsageEvent {
                dedup_key: format!("codebuddy_ide:{conv}:{msg_id}"),
                app: apps::CODEBUDDY_IDE.into(),
                session_id: Some(conv.clone()),
                project: project.clone(),
                model: text(&extra["modelId"]).or_else(|| text(&extra["modelName"])),
                ts_start: epoch_ms(&v["createdAt"]),
                input_tokens: crate::normalize::input_excludes_cache(
                    d.input,
                    d.cached,
                    d.cache_write,
                ),
                output_tokens: d.output,
                reasoning_tokens: d.thinking,
                cache_read_tokens: d.cached,
                cache_write_5m_tokens: d.cache_write,
                credits: (d.credit > 0.0).then_some(d.credit),
                duration_ms: (d.elapsed_ms > 0).then_some(d.elapsed_ms),
                provenance: Provenance::LocalJsonl,
                raw_ref: Some(f.to_string_lossy().to_string()),
                ..Default::default()
            });
        }

        // Persist via the conversation's index.json (a real file — the dir
        // itself can't produce a tail fingerprint). Skip the write entirely
        // when nothing new was processed — otherwise every quiet tick issues
        // one adapter_state UPDATE per conversation.
        let fp_file = [
            item.path.join("index.json"),
            item.path.join("..").join("index.json"),
        ]
        .into_iter()
        .find(|p| p.is_file());
        if !had_new && cur.state.is_some() {
            return Ok(out);
        }
        if let Some(fp) = fp_file {
            store.save_cursor(
                self.id(),
                &item.key,
                &fp,
                state.seen.len() as u64,
                0,
                Some(&serde_json::to_string(&state)?),
            )?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(dir: &Path, name: &str, role: &str, created: &str, extra: Option<Value>) {
        let mut v = serde_json::json!({"role": role, "id": name, "createdAt": created});
        if let Some(e) = extra {
            v["extra"] = Value::String(e.to_string());
        }
        std::fs::write(dir.join(format!("{name}.json")), v.to_string()).unwrap();
    }

    #[test]
    fn stats_snapshot_deltas_accumulate() {
        let dir = std::env::temp_dir().join(format!("gtt-cb-{}", std::process::id()));
        let conv = dir.join("convX");
        let msgs = conv.join("messages");
        std::fs::create_dir_all(&msgs).unwrap();
        std::fs::write(conv.join("index.json"), "{}").unwrap();
        let snap = |i: u64, o: u64, c: u64, t: u64, cr: f64| {
            serde_json::json!({
                "requestId": format!("req-{i}-{o}"),
                "modelId": "glm-5.3",
                "statsSnapshot": {
                    "inputTokens": i, "outputTokens": o,
                    "cachedInputTokens": c, "cacheWriteTokens": 0,
                    "thinkingTokens": t, "elapsedMs": 1000 * i, "credit": cr
                }
            })
        };
        msg(&msgs, "a", "user", "2026-09-03T10:00:00Z", None);
        msg(
            &msgs,
            "b",
            "assistant",
            "2026-09-03T10:00:10Z",
            Some(snap(1000, 50, 800, 10, 1.5)),
        );
        msg(
            &msgs,
            "c",
            "assistant",
            "2026-09-03T10:01:00Z",
            Some(snap(3000, 120, 2000, 30, 2.0)),
        );
        msg(&msgs, "d", "assistant", "2026-09-03T10:02:00Z", None); // no snapshot

        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: "convX".into(),
            path: conv.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = CodeBuddyIde::default().scan_sqlite(&item, &store).unwrap();
        assert_eq!(out.events.len(), 2);
        let first = &out.events[0];
        assert_eq!(first.dedup_key, "codebuddy_ide:convX:req-1000-50");
        assert_eq!(first.app, apps::CODEBUDDY_IDE);
        // input excludes cache-read: 1000 - 800
        assert_eq!(first.input_tokens, 200);
        assert_eq!(first.cache_read_tokens, 800);
        assert_eq!(first.output_tokens, 50);
        assert_eq!(first.reasoning_tokens, 10);
        assert_eq!(first.credits, Some(1.5));
        let second = &out.events[1];
        // delta vs first snapshot: d_input=2000, d_cached=1200 → 800 fresh
        assert_eq!(second.input_tokens, 800);
        assert_eq!(second.cache_read_tokens, 1200);
        assert_eq!(second.output_tokens, 70);
        assert_eq!(second.credits, Some(0.5));

        // Re-scan: cursor state persists → zero new events.
        let out2 = CodeBuddyIde::default().scan_sqlite(&item, &store).unwrap();
        assert_eq!(out2.events.len(), 0);

        std::fs::remove_dir_all(&dir).ok();
    }
}
