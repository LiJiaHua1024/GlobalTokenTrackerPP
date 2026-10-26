//! Devin CLI adapter: `~/.local/share/devin/cli/sessions.db` (also on macOS),
//! `$XDG_DATA_HOME/devin/cli/sessions.db`, and the legacy native data directory
//! (`%APPDATA%` on Windows, `~/Library/Application Support` on macOS).
//!
//! `message_nodes.chat_message` holds one JSON chat message per row.
//! Assistant nodes carry `metadata.metrics{input_tokens, output_tokens,
//! cache_read_tokens, cache_creation_tokens, ttft_ms, total_time_ms,
//! tpot_ms, tokens_per_sec}` — the richest telemetry of any adapter,
//! including TTFT/TPOT. `metadata.request_id` is shared by the ~2
//! snapshot rows written per inference (the same message is re-stored
//! when tool calls land; token metrics are identical across snapshots,
//! only timing fields get filled in) → dedup on `devin:{request_id}` and
//! the completeness-UPSERT keeps the most complete snapshot.
//!
//! `sessions` joins for model / working_directory (→project) /
//! agent_mode. `sessions.metadata.total_acu_cost`/`total_credit_cost`
//! are session-cumulative counters, NOT per-request — deliberately not
//! mapped onto events (would multiply-count); nothing per-request exists
//! in the local schema today.
//! Internal `compactor` calls use a separate model (locally displayed as
//! "Compactor (Luna)"). The session's selected model is not their request
//! model: leave it unset so pricing cannot borrow the main agent's rate.
//!
//! High-water mark: `message_nodes.row_id` AUTOINCREMENT, kept in
//! `sync_cursors.last_byte_offset`. The v2 marker replays previously skipped
//! rows once: the old literal `"role":"assistant"` filter missed spaced JSON.
//! Existing request dedup keys are preserved, so backfill does not add copies.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, num, num_opt, text};
use crate::store::Store;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const METRICS_STATE: &str = "devin-metrics-v2";

pub struct Devin;

fn cli_roots(home: Option<&Path>, native: Option<&Path>, xdg: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for base in [
        native.map(Path::to_path_buf),
        home.map(|p| p.join(".local/share")),
        xdg.filter(|p| p.is_absolute()).map(Path::to_path_buf),
    ]
    .into_iter()
    .flatten()
    {
        let root = base.join("devin/cli");
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

fn roots() -> Vec<PathBuf> {
    cli_roots(
        dirs::home_dir().as_deref(),
        dirs::data_dir().as_deref(),
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .as_deref(),
    )
}

fn discover_in(roots: &[PathBuf]) -> Vec<SourceItem> {
    let mut seen = HashSet::new();
    roots
        .iter()
        .map(|p| p.join("sessions.db"))
        .filter(|p| p.is_file())
        // A migrated installation may leave a symlink to the same database.
        .filter(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())))
        .map(|path| SourceItem {
            key: path.to_string_lossy().into_owned(),
            path,
            kind: SourceKind::Sqlite,
        })
        .collect()
}

impl SourceAdapter for Devin {
    fn id(&self) -> &'static str {
        apps::DEVIN
    }
    fn display_name(&self) -> &'static str {
        "Devin"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        roots()
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(discover_in(&roots()))
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let since = if cur.state.as_deref() == Some(METRICS_STATE) {
            cur.offset as i64
        } else {
            0 // One-time backfill under the same request identities.
        };
        let conn = super::opencode::open_ro(&item.path)?;
        let mut st = conn.prepare(
            // Extract only role + metadata; chat content can be megabytes.
            // Structural JSON access accepts all whitespace layouts and never
            // mistakes an embedded assistant message in tool content for usage.
            "SELECT n.row_id, n.session_id,
                    CASE WHEN json_valid(n.chat_message) THEN
                        json_extract(n.chat_message, '$.role', '$.metadata') END,
                    s.model, s.working_directory
             FROM message_nodes n
             LEFT JOIN sessions s ON s.id = n.session_id
             WHERE n.row_id > ?1
             ORDER BY n.row_id",
        )?;
        let rows = st.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = ScanOutcome::default();
        let mut max_row = cur.offset as i64;
        for row in rows {
            let (row_id, session_id, msg_md, sess_model, workdir) = row?;
            max_row = max_row.max(row_id);
            let Some(msg_md) = msg_md else { continue };
            let Ok(message) = serde_json::from_str::<Value>(&msg_md) else {
                continue;
            };
            if message[0] != "assistant" {
                continue;
            }
            let md = &message[1];
            let metrics = &md["metrics"];
            if !metrics.is_object() {
                continue;
            }
            let rid = text(&md["request_id"])
                .unwrap_or_else(|| format!("{}:{row_id}", session_id.as_deref().unwrap_or("?")));
            let model = text(&md["generation_model"]).or_else(|| sess_model.clone());
            let request_model = if model.as_deref() == Some("compactor") {
                None
            } else {
                sess_model
            };
            out.events.push(UsageEvent {
                dedup_key: format!("devin:{rid}"),
                app: apps::DEVIN.into(),
                session_id,
                project: workdir,
                model,
                request_model,
                ts_start: epoch_ms(&md["started_generation_at"])
                    .or_else(|| epoch_ms(&md["created_at"])),
                ts_end: epoch_ms(&md["created_at"]),
                input_tokens: num(&metrics["input_tokens"]),
                output_tokens: num(&metrics["output_tokens"]),
                cache_read_tokens: num(&metrics["cache_read_tokens"]),
                // Devin doesn't split 5m/1h cache writes.
                cache_write_5m_tokens: num(&metrics["cache_creation_tokens"]),
                ttft_ms: num_opt(&metrics["ttft_ms"]).map(|v| v as i64),
                duration_ms: num_opt(&metrics["total_time_ms"]).map(|v| v as i64),
                status: text(&md["finish_reason"]),
                credits: fnum(&md["acu_cost"]),
                provenance: Provenance::LocalSqlite,
                raw_ref: Some(format!("{}#row:{}", item.path.display(), row_id)),
                ..Default::default()
            });
        }
        if (max_row as u64) > cur.offset || cur.state.as_deref() != Some(METRICS_STATE) {
            out.pending_cursor = Some(PendingCursor {
                key: item.key.clone(),
                path: item.path.clone(),
                offset: max_row as u64,
                mtime_ms: 0,
                state: Some(METRICS_STATE.into()),
            });
        }
        out.consumed = max_row as u64;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        Ok(crate::adapters::scan_and_commit(&Devin, store, item))
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gtt_devin_{name}_{}_{}",
            std::process::id(),
            crate::store::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn item(path: PathBuf) -> SourceItem {
        SourceItem {
            key: path.to_string_lossy().into_owned(),
            path,
            kind: SourceKind::Sqlite,
        }
    }

    fn insert(c: &rusqlite::Connection, message: &str) {
        c.execute(
            "INSERT INTO message_nodes(session_id,node_id,chat_message,created_at)
             VALUES('s1',1,?1,1)",
            [message],
        )
        .unwrap();
    }

    fn src_db(path: &std::path::Path) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, working_directory TEXT NOT NULL,
                backend_type TEXT, model TEXT, agent_mode TEXT, created_at INTEGER,
                last_activity_at INTEGER, metadata TEXT);
             CREATE TABLE message_nodes(row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT, node_id INTEGER, chat_message TEXT, created_at INTEGER);",
        )
        .unwrap();
        c
    }

    fn msg(rid: &str, out: u64, inp: u64, cr: u64, ttft: u64) -> String {
        format!(
            r#"{{"message_id":"m-{rid}","role":"assistant","content":"…",
            "metadata":{{"request_id":"{rid}","generation_model":"swe-2-max",
            "finish_reason":"tool_calls","started_generation_at":"2026-09-26T21:16:56Z",
            "created_at":"2026-09-26T21:17:01Z",
            "metrics":{{"ttft_ms":{ttft},"total_time_ms":10432,"input_tokens":{inp},
            "output_tokens":{out},"cache_read_tokens":{cr},"cache_creation_tokens":null,
            "tpot_ms":10.0,"tokens_per_sec":98.0}}}}}}"#
        )
    }

    #[test]
    fn devin_sqlite_parses_assistant_metrics_and_dedups_request() {
        let dir = test_dir("metrics");
        let src = dir.join("sessions.db");
        let c = src_db(&src);
        c.execute(
            "INSERT INTO sessions VALUES('s1','D:\\proj','windsurf','swe-2-max','bypass',1,1,'{}')",
            [],
        )
        .unwrap();
        // Same request snapshotted twice (tool-call re-store) + a user msg + one more request.
        c.execute("INSERT INTO message_nodes(session_id,node_id,chat_message,created_at) VALUES('s1',1,?1,1)",
            [msg("r1", 377, 588, 52394, 6620)]).unwrap();
        c.execute("INSERT INTO message_nodes(session_id,node_id,chat_message,created_at) VALUES('s1',2,?1,1)",
            [msg("r1", 377, 588, 52394, 6620)]).unwrap();
        c.execute("INSERT INTO message_nodes(session_id,node_id,chat_message,created_at) VALUES('s1',3,?1,1)",
            [r#"{"role":"user","content":"hi"}"#.to_string()]).unwrap();
        c.execute("INSERT INTO message_nodes(session_id,node_id,chat_message,created_at) VALUES('s1',4,?1,1)",
            [msg("r2", 100, 10, 0, 100)]).unwrap();
        drop(c);

        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: src.to_string_lossy().into(),
            path: src.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan(&item, &store).unwrap();
        assert_eq!(out.events.len(), 3); // 2 snapshots of r1 + r2
        let e = &out.events[0];
        assert_eq!(e.dedup_key, "devin:r1");
        assert_eq!(e.app, "devin");
        assert_eq!(e.model.as_deref(), Some("swe-2-max"));
        assert_eq!(e.project.as_deref(), Some(r"D:\proj"));
        assert_eq!(e.input_tokens, 588);
        assert_eq!(e.output_tokens, 377);
        assert_eq!(e.cache_read_tokens, 52394);
        assert_eq!(e.ttft_ms, Some(6620));
        assert_eq!(e.duration_ms, Some(10432));
        assert_eq!(e.status.as_deref(), Some("tool_calls"));
        // High-water mark advanced past all rows.
        assert_eq!(store.load_cursor(&item.key).unwrap().offset, 4);
        // Second scan returns nothing new.
        assert!(scan(&item, &store).unwrap().events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovers_native_unix_and_xdg_databases_without_duplicate_paths() {
        let dir = test_dir("discovery");
        let home = dir.join("home");
        let native = home.join("Library/Application Support");
        let xdg = dir.join("xdg");
        let roots = cli_roots(Some(&home), Some(&native), Some(&xdg));
        assert_eq!(roots.len(), 3);
        assert!(discover_in(&roots).is_empty());
        for root in &roots {
            std::fs::create_dir_all(root).unwrap();
            std::fs::write(root.join("sessions.db"), []).unwrap();
        }
        assert_eq!(discover_in(&roots).len(), 3);
        let unix = home.join(".local/share");
        assert_eq!(cli_roots(Some(&home), Some(&unix), Some(&unix)).len(), 1);
        assert_eq!(cli_roots(None, None, Some(Path::new("relative"))).len(), 0);
        // Duplicate discovery roots and symlink aliases must scan only once.
        let mut aliases = roots.clone();
        aliases.extend(roots.clone());
        #[cfg(unix)]
        {
            let alias = dir.join("alias");
            std::os::unix::fs::symlink(&roots[0], &alias).unwrap();
            aliases.push(alias);
        }
        assert_eq!(discover_in(&aliases).len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_cursor_backfills_spaced_json_without_double_counting() {
        let dir = test_dir("backfill");
        let src = item(dir.join("sessions.db"));
        let c = src_db(&src.path);
        let spaced = serde_json::to_string_pretty(
            &serde_json::from_str::<Value>(&msg("missing", 20, 10, 30, 40)).unwrap(),
        )
        .unwrap();
        insert(&c, &spaced);
        insert(&c, &msg("existing", 2, 1, 3, 4));
        let store = Store::open_memory().unwrap();
        // Simulate a legacy scanner that missed row 1 but ingested row 2.
        let existing = scan(&src, &store).unwrap().events.pop().unwrap();
        store.upsert_event(&existing).unwrap();
        store
            .save_cursor("devin", &src.key, &src.path, 2, 0, None)
            .unwrap();
        let out = scan(&src, &store).unwrap();
        assert_eq!(out.events.len(), 2);
        for e in &out.events {
            store.upsert_event(e).unwrap();
        }
        assert_eq!(store.event_count(None, None).unwrap(), 2);
        let totals: (i64, i64, i64) = store.conn().query_row(
            "SELECT SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens) FROM usage_events",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(totals, (11, 22, 33));
        let cur = store.load_cursor(&src.key).unwrap();
        assert_eq!(cur.state.as_deref(), Some(METRICS_STATE));
        assert_eq!(cur.offset, 2);
        assert!(scan(&src, &store).unwrap().events.is_empty());
        // An append after migration still uses the incremental watermark.
        insert(&c, &msg("appended", 6, 5, 7, 8));
        assert_eq!(
            scan(&src, &store).unwrap().events[0].dedup_key,
            "devin:appended"
        );
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn skips_invalid_and_non_assistant_rows_but_advances_past_them() {
        let dir = test_dir("roles");
        let src = item(dir.join("sessions.db"));
        let c = src_db(&src.path);
        insert(&c, &msg("valid", 20, 10, 30, 40));
        let mut tool: Value = serde_json::from_str(&msg("tool", 200, 100, 300, 400)).unwrap();
        tool["role"] = "tool".into();
        tool["content"] = serde_json::json!({"role":"assistant"});
        insert(&c, &tool.to_string());
        insert(&c, r#"{"role":"assistant","metadata":{"metrics":null}}"#);
        insert(&c, "{invalid JSON");
        let store = Store::open_memory().unwrap();
        let out = scan(&src, &store).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].dedup_key, "devin:valid");
        assert_eq!(store.load_cursor(&src.key).unwrap().offset, 4);
        assert!(scan(&src, &store).unwrap().events.is_empty());
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copies_in_multiple_locations_share_request_identity() {
        let dir = test_dir("copies");
        let store = Store::open_memory().unwrap();
        for name in ["native.db", "unix.db"] {
            let src = item(dir.join(name));
            let c = src_db(&src.path);
            insert(&c, &msg("shared", 20, 10, 30, 40));
            for e in scan(&src, &store).unwrap().events {
                store.upsert_event(&e).unwrap();
            }
            assert!(scan(&src, &store).unwrap().events.is_empty());
        }
        assert_eq!(store.event_count(None, None).unwrap(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compactor_usage_does_not_borrow_the_main_agents_price() {
        let dir = test_dir("compactor");
        let src = item(dir.join("sessions.db"));
        let c = src_db(&src.path);
        c.execute("INSERT INTO sessions(id,working_directory,model) VALUES('s1','/project','gpt-5.4-mini')", []).unwrap();
        let mut message: Value = serde_json::from_str(&msg("compact", 20, 10, 30, 40)).unwrap();
        message["metadata"]["generation_model"] = "compactor".into();
        insert(&c, &message.to_string());
        let store = Store::open_memory().unwrap();
        let mut event = scan(&src, &store).unwrap().events.pop().unwrap();
        assert_eq!(event.model.as_deref(), Some("compactor"));
        assert!(event.request_model.is_none());
        assert_eq!(event.input_tokens, 10);
        assert_eq!(event.output_tokens, 20);
        // A known session model has a seed price, but the compactor is opaque.
        let book = crate::pricing::PriceBook::load(&store).unwrap();
        assert!(matches!(
            book.resolve("gpt-5.4-mini", None, None),
            crate::pricing::Resolution::Priced(..)
        ));
        book.apply(&mut event);
        assert_eq!(event.cost_source, Some(crate::model::CostSource::Unpriced));
        assert!(event.pricing_model.is_none());
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
