//! ZCode adapter (spec §6.4): `~/.zcode/cli/db/db.sqlite` → `model_usage`
//! (cleanest five-dimension source + duration/TTFT/status).
//! Dedup key: `logical_request_id + attempt_index` (ids carry retry suffixes).
//! Cross-source rule: rows whose provider is anthropic/openai are also logged
//! by those tools' own files — skip & count to prevent double counting.
//! google is NOT skipped: no other adapter ingests google rows from this db,
//! so skipping them discarded that usage permanently.
//!
//! High-water mark: `rowid`, NOT `started_at`. `model_usage` rows are inserted
//! when a request COMPLETES, stamped with its original `started_at` — observed
//! lateness up to ~13 min on real data (a long request lands far behind the
//! timestamp watermark and a `started_at > mark` filter drops it forever).
//! `rowid` is monotone with insertion order → nothing is missed. One-time
//! migration: `adapter_state = "rowid-v2"` resets the timestamp cursor so all
//! rows rescan once (dedup keys unchanged → upsert is idempotent).

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::Result;
use std::path::PathBuf;

/// Providers whose usage is already ingested by their own adapters (§6.4).
const CROSS_PROVIDERS: &[&str] = &["anthropic", "openai"];

pub struct ZCode;

impl SourceAdapter for ZCode {
    fn id(&self) -> &'static str {
        apps::ZCODE
    }
    fn display_name(&self) -> &'static str {
        "ZCode"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".zcode/cli/db")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let p = crate::sync::home(".zcode/cli/db/db.sqlite");
        Ok(p.exists()
            .then(|| SourceItem {
                key: p.to_string_lossy().to_string(),
                path: p,
                kind: SourceKind::Sqlite,
            })
            .into_iter()
            .collect())
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        // Migration: pre-"rowid-v2" cursors held a `started_at` epoch-ms
        // watermark that missed late-inserted rows (inserts happen at request
        // completion). Reset once → full rescan; dedup keys make it idempotent.
        let migrated = cur.state.as_deref() == Some("rowid-v2");
        let since_rowid = if migrated { cur.offset as i64 } else { 0 };
        let conn = super::opencode::open_ro(&item.path)?;
        let mut st = conn.prepare(
            "SELECT rowid, logical_request_id, attempt_index, session_id, turn_id,
                    provider_id, model_id, status, started_at, completed_at,
                    duration_ms, time_to_first_token_ms,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_creation_input_tokens, cache_read_input_tokens,
                    error_type
             FROM model_usage WHERE rowid > ?1 ORDER BY rowid",
        )?;
        let rows = st.query_map(rusqlite::params![since_rowid], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<i64>>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, Option<i64>>(12)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(13)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(14)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(16)?.unwrap_or(0) as u64,
                r.get::<_, Option<String>>(17)?,
            ))
        })?;

        let mut out = ScanOutcome::default();
        let mut max_rowid = since_rowid;
        // A still-running row must not cap the watermark — it gets rescanned
        // (idempotently, along with everything after it) once it completes.
        let mut first_running_rowid: Option<i64> = None;
        for row in rows {
            let (
                rid,
                lrid,
                attempt,
                session,
                turn,
                provider,
                model,
                status,
                t0,
                t1,
                dur,
                ttft,
                tin,
                tout,
                treason,
                tcw,
                tcr,
                err_ty,
            ) = row?;
            if status.as_deref() == Some("running") {
                out.skipped += 1;
                first_running_rowid = first_running_rowid.or(Some(rid));
                continue;
            }
            max_rowid = max_rowid.max(rid);

            // Cross-provider rows are counted by the owning tool's adapter.
            let base = provider
                .as_deref()
                .unwrap_or("")
                .trim_start_matches("builtin:")
                .split(':')
                .next()
                .unwrap_or("");
            if CROSS_PROVIDERS.contains(&base) {
                out.skipped += 1;
                continue;
            }

            out.events.push(UsageEvent {
                dedup_key: format!("zcode:{lrid}:{attempt}"),
                app: apps::ZCODE.into(),
                session_id: session,
                provider_id: provider,
                model: model.clone(),
                request_model: model,
                ts_start: t0,
                ts_end: t1,
                input_tokens: tin,
                output_tokens: tout,
                reasoning_tokens: treason,
                cache_read_tokens: tcr,
                cache_write_5m_tokens: tcw,
                provenance: Provenance::LocalSqlite,
                duration_ms: dur,
                ttft_ms: ttft,
                status: status.or(turn).map(|s| s.to_string()),
                error: err_ty,
                raw_ref: Some(format!(
                    "{}#model_usage:{lrid}:{attempt}",
                    item.path.display()
                )),
                ..Default::default()
            });
        }
        let next = first_running_rowid.map_or(max_rowid, |r| r - 1);
        if next as u64 != cur.offset || !migrated {
            out.pending_cursor = Some(PendingCursor {
                key: item.key.clone(),
                path: item.path.clone(),
                offset: next.max(0) as u64,
                mtime_ms: 0,
                state: Some("rowid-v2".into()),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::scan_and_commit;
    use rusqlite::Connection;
    use std::path::Path;

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_db(tag: &str) -> (PathBuf, TmpDir) {
        let dir = std::env::temp_dir().join(format!("gtt-zc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("db.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE model_usage(
               id TEXT PRIMARY KEY, logical_request_id TEXT NOT NULL,
               attempt_index INTEGER NOT NULL DEFAULT 0, session_id TEXT NOT NULL,
               turn_id TEXT, provider_id TEXT NOT NULL, model_id TEXT NOT NULL,
               status TEXT NOT NULL, started_at INTEGER NOT NULL,
               completed_at INTEGER, duration_ms INTEGER,
               time_to_first_token_ms INTEGER,
               input_tokens INTEGER NOT NULL DEFAULT 0,
               output_tokens INTEGER NOT NULL DEFAULT 0,
               reasoning_tokens INTEGER NOT NULL DEFAULT 0,
               cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
               cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
               error_type TEXT);",
        )
        .unwrap();
        (db, TmpDir(dir))
    }

    fn insert(db: &Path, id: &str, lrid: &str, status: &str, started: i64, tokens: i64) {
        let conn = Connection::open(db).unwrap();
        conn.execute(
            "INSERT INTO model_usage(id, logical_request_id, session_id,
               provider_id, model_id, status, started_at, input_tokens)
             VALUES(?1,?2,'s','zhipu','glm-4.6',?3,?4,?5)",
            rusqlite::params![id, lrid, status, started, tokens],
        )
        .unwrap();
    }

    fn item_for(db: &Path) -> SourceItem {
        SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.to_path_buf(),
            kind: SourceKind::Sqlite,
        }
    }

    /// Rows are written at request completion; a long request lands with a
    /// `started_at` far behind the watermark — a timestamp watermark misses it
    /// forever, a rowid watermark doesn't.
    #[test]
    fn late_inserted_row_is_not_missed() {
        let (db, _dir) = fixture_db("late");
        let store = Store::open_memory().unwrap();
        let item = item_for(&db);
        insert(&db, "a", "req-a", "completed", 1_000_000, 100);
        let out = scan_and_commit(&ZCode, &store, &item);
        assert_eq!(out.events.len(), 1);

        // 60s-late insert — started_at long before the first row's.
        insert(&db, "b", "req-b", "completed", 940_000, 200);
        let out = scan_and_commit(&ZCode, &store, &item);
        assert_eq!(out.events.len(), 1, "late insert must be caught");
        assert_eq!(out.events[0].dedup_key, "zcode:req-b:0");
        assert_eq!(out.events[0].input_tokens, 200);
        assert_eq!(out.events[0].ts_start, Some(940_000));
    }

    /// Pre-migration cursors hold epoch-ms started_at — must rescan from 0.
    #[test]
    fn timestamp_cursor_migrates_and_rescans() {
        let (db, _dir) = fixture_db("mig");
        let store = Store::open_memory().unwrap();
        let item = item_for(&db);
        insert(&db, "a", "req-a", "completed", 1_000_000, 100);
        // Simulate the old watermark: an epoch-ms offset that dwarfs rowids.
        store
            .save_cursor("zcode", &item.key, &item.path, 1_790_519_732_267, 0, None)
            .unwrap();
        let out = scan_and_commit(&ZCode, &store, &item);
        assert_eq!(out.events.len(), 1, "migration must rescan all rows");
        let cur = store.load_cursor(&item.key).unwrap();
        assert_eq!(cur.state.as_deref(), Some("rowid-v2"));
        // Second scan: nothing new.
        let out = scan_and_commit(&ZCode, &store, &item);
        assert!(out.events.is_empty());
    }

    /// A 'running' row must not cap the watermark: it is skipped now and
    /// rescanned (along with everything after it) until it completes.
    #[test]
    fn running_row_keeps_watermark_back() {
        let (db, _dir) = fixture_db("run");
        let store = Store::open_memory().unwrap();
        let item = item_for(&db);
        insert(&db, "a", "req-a", "completed", 1_000_000, 100);
        insert(&db, "b", "req-b", "running", 1_000_100, 0);
        insert(&db, "c", "req-c", "completed", 1_000_200, 300);
        let out = scan_and_commit(&ZCode, &store, &item);
        assert_eq!(out.events.len(), 2);
        assert_eq!(out.skipped, 1);

        // req-b finishes in place (status flip) — rescan picks it up; req-c
        // re-emits but merges by dedup key.
        Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE model_usage SET status='completed', input_tokens=200
                 WHERE id='b'",
                [],
            )
            .unwrap();
        let out = scan_and_commit(&ZCode, &store, &item);
        assert!(
            out.events
                .iter()
                .any(|e| e.dedup_key == "zcode:req-b:0" && e.input_tokens == 200),
            "completed update must be rescanned"
        );
    }
}
