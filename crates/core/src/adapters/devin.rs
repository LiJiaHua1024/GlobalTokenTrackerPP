//! Devin adapter (spec §6.x): `<data_dir>/devin/cli/sessions.db`.
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
//!
//! High-water mark: `message_nodes.row_id` AUTOINCREMENT, kept in
//! `sync_cursors.last_byte_offset`.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, num, num_opt, text};
use crate::store::Store;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct Devin;

fn db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("devin/cli/sessions.db")
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
        vec![db_path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let db = db_path();
        Ok(if db.exists() {
            vec![SourceItem {
                key: db.to_string_lossy().to_string(),
                path: db,
                kind: SourceKind::Sqlite,
            }]
        } else {
            vec![]
        })
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let since = cur.offset as i64;
        let conn = super::opencode::open_ro(&item.path)?;
        let mut st = conn.prepare(
            // LIKE is a byte scan (cheap); the real gate is metrics.is_object()
            // in Rust — user/tool rows carry no metrics object.
            "SELECT n.row_id, n.session_id,
                    json_extract(n.chat_message, '$.metadata'),
                    s.model, s.working_directory
             FROM message_nodes n
             LEFT JOIN sessions s ON s.id = n.session_id
             WHERE n.row_id > ?1
               AND n.chat_message LIKE '%\"role\":\"assistant\"%'
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
            let Ok(md) = serde_json::from_str::<Value>(&msg_md) else {
                continue;
            };
            let metrics = &md["metrics"];
            if !metrics.is_object() {
                continue;
            }
            let rid = text(&md["request_id"])
                .unwrap_or_else(|| format!("{}:{row_id}", session_id.as_deref().unwrap_or("?")));
            out.events.push(UsageEvent {
                dedup_key: format!("devin:{rid}"),
                app: apps::DEVIN.into(),
                session_id,
                project: workdir,
                model: text(&md["generation_model"]).or_else(|| sess_model.clone()),
                request_model: sess_model,
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
        if (max_row as u64) > cur.offset {
            store.save_cursor(self.id(), &item.key, &item.path, max_row as u64, 0, None)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let dir = std::env::temp_dir().join(format!("gtt_devin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
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
        let out = Devin.scan_sqlite(&item, &store).unwrap();
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
        assert!(Devin.scan_sqlite(&item, &store).unwrap().events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
