//! MiniMax Code adapter (MiniMax-AI/minimax-code): local usage lives in
//! SQLite at `~/.minimax/v2/sqlite/runtime-state.sqlite`
//! (`packages/local-runtime-v2/src/infra/db/client.ts`, `schema/usage.ts`).
//!
//!   table `local_runtime_token_usage` — ONE ROW PER LLM CALL (deltas):
//!     id INTEGER PK AUTOINCREMENT, session_id, agent_name, framework_type,
//!     turn_id, model, ts INTEGER (epoch ms), input_tokens, output_tokens,
//!     reasoning_tokens, cache_read_tokens, cache_write_tokens (all NOT NULL),
//!     cost_usd REAL NULL, raw TEXT.
//!   `local_runtime_sessions` joins for project_workspace_dir/workspace_dir.
//!
//! Incremental: `id` is monotonic → high-water mark in the cursor's offset
//! field; `id > last` yields exactly the new rows. `cost_usd` is recorded by
//! the tool itself → ProviderReported.
//!
//! Legacy root `~/.mavis` is replaced by a junction/symlink to `.minimax`
//! after data-dir migration (`packages/config/src/data-dir.ts`) — scanning it
//! too would read the same DB twice, so `.mavis*` is only used when no
//! `.minimax*` dir exists at all. Profiles produce `~/.minimax-<name>` and are
//! all collected.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::Result;
use std::path::PathBuf;

pub struct MiniMaxCode;

/// Same wall-clock floor and skew allowance as the antigravity adapter:
/// 2020-01-01 in epoch ms, plus one hour of tolerated skew.
const MIN_TS_MS: i64 = 1_577_836_800_000;
const SKEW_MS: i64 = 60 * 60 * 1000;

/// `~/.minimax*` dirs (incl. `~/.minimax-<profile>`); falls back to real
/// `~/.mavis*` dirs only when no `.minimax*` exists (post-migration `.mavis`
/// is a junction into `.minimax` → same file, would double-count).
fn data_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    let read = |prefix: &str| -> Vec<PathBuf> {
        std::fs::read_dir(&home)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().starts_with(prefix))
                            .unwrap_or(false)
                            && p.is_dir()
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let primary = read(".minimax");
    if !primary.is_empty() {
        return primary;
    }
    // Legacy only when there is no `.minimax` to migrate into — and never
    // through a junction (that IS the migrated `.minimax` dir).
    read(".mavis")
        .into_iter()
        .filter(|p| !p.is_symlink())
        .collect()
}

fn db_files() -> Vec<PathBuf> {
    data_roots()
        .into_iter()
        .map(|r| r.join("v2/sqlite/runtime-state.sqlite"))
        .filter(|p| p.is_file())
        .collect()
}

impl SourceAdapter for MiniMaxCode {
    fn id(&self) -> &'static str {
        apps::MINIMAX_CODE
    }
    fn display_name(&self) -> &'static str {
        "MiniMax Code"
    }
    /// Per-LLM-call token rows, vendor-recorded — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        data_roots()
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(db_files()
            .into_iter()
            .map(|p| SourceItem {
                key: p.to_string_lossy().to_string(),
                path: p,
                kind: SourceKind::Sqlite,
            })
            .collect())
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let conn = super::opencode::open_ro(&item.path)?;
        // Older builds may lack the table — treat as "no data", not an error.
        let has_table: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
              WHERE type='table' AND name='local_runtime_token_usage')",
            [],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )?;
        if !has_table {
            return Ok(ScanOutcome::default());
        }
        let mut st = conn.prepare(
            "SELECT u.id, u.session_id, u.agent_name, u.model, u.ts,
                    u.input_tokens, u.output_tokens, u.reasoning_tokens,
                    u.cache_read_tokens, u.cache_write_tokens, u.cost_usd,
                    s.project_workspace_dir, s.workspace_dir
             FROM local_runtime_token_usage u
             LEFT JOIN local_runtime_sessions s ON s.session_id = u.session_id
             WHERE u.id > ?1 ORDER BY u.id",
        )?;
        let rows = st.query_map(rusqlite::params![cur.offset as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, Option<f64>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, Option<String>>(12)?,
            ))
        })?;

        // Tag the dedup key with the data-root basename so `.minimax` and
        // `.minimax-work` (separate profiles, separate DBs) can't collide.
        let tag = item
            .path
            .ancestors()
            .nth(3)
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut out = ScanOutcome::default();
        let mut max_id = cur.offset as i64;
        let now = crate::store::now_ms();
        for row in rows {
            let (
                id,
                sid,
                agent,
                model,
                ts,
                input,
                output,
                reasoning,
                cache_read,
                cache_write,
                cost,
                proj_dir,
                ws_dir,
            ) = row?;
            max_id = max_id.max(id);
            // minimax stamps epoch ms; older builds wrote seconds. Scale a
            // sub-2020 value up; anything still absurd stays undated rather
            // than poisoning the 1970 rollups.
            let ts = if ts > 0 && ts < MIN_TS_MS { ts * 1000 } else { ts };
            let ts = (MIN_TS_MS..=now + SKEW_MS).contains(&ts).then_some(ts);
            let all_zero = input <= 0
                && output <= 0
                && reasoning <= 0
                && cache_read <= 0
                && cache_write <= 0
                && cost.unwrap_or(0.0) <= 0.0;
            if all_zero {
                out.skipped += 1;
                continue;
            }
            // Project = the workspace dir verbatim (claude/codex/qoder store
            // full cwd too — grouping by real path, not basename).
            let project = proj_dir.or(ws_dir).filter(|d| !d.is_empty());
            out.events.push(UsageEvent {
                dedup_key: format!("minimax_code:{tag}:{id}"),
                app: apps::MINIMAX_CODE.into(),
                session_id: sid.filter(|s| !s.is_empty()),
                project,
                model: model.clone(),
                request_model: model,
                ts_start: ts,
                input_tokens: input.max(0) as u64,
                output_tokens: output.max(0) as u64,
                reasoning_tokens: reasoning.max(0) as u64,
                cache_read_tokens: cache_read.max(0) as u64,
                cache_write_5m_tokens: cache_write.max(0) as u64,
                cost_usd: cost.filter(|c| *c > 0.0),
                cost_source: cost
                    .filter(|c| *c > 0.0)
                    .map(|_| CostSource::ProviderReported),
                provenance: Provenance::LocalSqlite,
                raw_ref: Some(format!(
                    "{}#token_usage.id={id}{}",
                    item.path.display(),
                    agent.map(|a| format!(" agent={a}")).unwrap_or_default()
                )),
                ..Default::default()
            });
        }
        if (max_id as u64) > cur.offset {
            out.pending_cursor = Some(PendingCursor {
                key: item.key.clone(),
                path: item.path.clone(),
                offset: max_id as u64,
                mtime_ms: 0,
                state: None,
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

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_db(tag: &str) -> (PathBuf, TmpDir) {
        let dir = std::env::temp_dir().join(format!("gtt-mm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // stale file from a prior run
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("runtime-state.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE local_runtime_token_usage(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL, agent_name TEXT NOT NULL,
               framework_type TEXT NOT NULL, turn_id TEXT, model TEXT,
               ts INTEGER NOT NULL, input_tokens INTEGER NOT NULL,
               output_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL,
               cache_read_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL,
               cost_usd REAL, raw TEXT);
             CREATE TABLE local_runtime_sessions(
               session_id TEXT PRIMARY KEY, record_json TEXT NOT NULL,
               updated_at_ms INTEGER NOT NULL, columnar_version INTEGER NOT NULL DEFAULT 0,
               archived INTEGER NOT NULL DEFAULT 0, visibility TEXT NOT NULL DEFAULT 'visible',
               session_kind TEXT NOT NULL DEFAULT 'unknown',
               purpose_kind TEXT NOT NULL DEFAULT '',
               is_default_workspace INTEGER NOT NULL DEFAULT 0,
               extra_data_json TEXT NOT NULL DEFAULT '{}',
               workspace_dir TEXT, project_workspace_dir TEXT);
             INSERT INTO local_runtime_sessions(session_id, record_json, updated_at_ms,
               workspace_dir, project_workspace_dir)
               VALUES('s1','{}',1,'D:/proj/demo','D:/proj/demo');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_token_usage(session_id,agent_name,framework_type,
               turn_id,model,ts,input_tokens,output_tokens,reasoning_tokens,
               cache_read_tokens,cache_write_tokens,cost_usd,raw)
             VALUES('s1','main','pi-agent','t1','minimax-m2.5',1779256800300,
                    1000,50,20,800,200,0.0123,'{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_token_usage(session_id,agent_name,framework_type,
               turn_id,model,ts,input_tokens,output_tokens,reasoning_tokens,
               cache_read_tokens,cache_write_tokens,cost_usd,raw)
             VALUES('s1','main','pi-agent','t1','minimax-m2.5',1779256800500,
                    0,0,0,0,0,NULL,NULL)",
            [],
        )
        .unwrap();
        (db, TmpDir(dir))
    }

    #[test]
    fn seconds_timestamps_scale_and_zero_stays_undated() {
        let (db, _dir) = fixture_db("ts-sanity");
        {
            let conn = Connection::open(&db).unwrap();
            for (turn, ts) in [("t1", 1_779_256_800i64), ("t2", 0)] {
                conn.execute(
                    "INSERT INTO local_runtime_token_usage(session_id,agent_name,framework_type,
                       turn_id,model,ts,input_tokens,output_tokens,reasoning_tokens,
                       cache_read_tokens,cache_write_tokens,cost_usd,raw)
                     VALUES('s1','main','pi-agent',?1,'minimax-m2.5',?2,
                            100,10,0,0,0,NULL,NULL)",
                    rusqlite::params![turn, ts],
                )
                .unwrap();
            }
        }
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&MiniMaxCode, &store, &item);
        // The fixture seeds one believable row (id 1) and an all-zero one
        // (id 2, skipped); our rows land as ids 3 and 4.
        assert_eq!(out.events.len(), 3);
        assert_eq!(out.events[0].ts_start, Some(1_779_256_800_300));
        // Seconds-era row scaled ×1000 into the ms domain.
        assert_eq!(out.events[1].ts_start, Some(1_779_256_800_000));
        // A zero ts stays undated instead of poisoning the 1970 rollups.
        assert_eq!(out.events[2].ts_start, None);
    }

    #[test]
    fn token_usage_rows_incremental_and_exact() {
        let (db, _dir) = fixture_db("usage");
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&MiniMaxCode, &store, &item);
        assert_eq!(out.events.len(), 1);
        let ev = &out.events[0];
        assert_eq!(ev.app, apps::MINIMAX_CODE);
        assert_eq!(ev.model.as_deref(), Some("minimax-m2.5"));
        assert_eq!(ev.input_tokens, 1000);
        assert_eq!(ev.output_tokens, 50);
        assert_eq!(ev.reasoning_tokens, 20);
        assert_eq!(ev.cache_read_tokens, 800);
        assert_eq!(ev.cache_write_5m_tokens, 200);
        assert_eq!(ev.cost_usd, Some(0.0123));
        assert_eq!(ev.project.as_deref(), Some("D:/proj/demo"));
        assert_eq!(ev.ts_start, Some(1779256800300));
        // all-zero row skipped but cursor advanced past it
        assert_eq!(out.skipped, 1);

        // Re-scan: watermark at id=2 → nothing new.
        let out2 = scan_and_commit(&MiniMaxCode, &store, &item);
        assert!(out2.events.is_empty());

        // Append a row → only the new row arrives.
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO local_runtime_token_usage(session_id,agent_name,framework_type,
               turn_id,model,ts,input_tokens,output_tokens,reasoning_tokens,
               cache_read_tokens,cache_write_tokens,cost_usd,raw)
             VALUES('s1','main','pi-agent','t2','minimax-m2.5',1779256900000,
                    2000,80,0,0,0,NULL,NULL)",
            [],
        )
        .unwrap();
        drop(conn);
        let out3 = scan_and_commit(&MiniMaxCode, &store, &item);
        assert_eq!(out3.events.len(), 1);
        assert_eq!(out3.events[0].input_tokens, 2000);
        assert!(out3.events[0].cost_usd.is_none());
        assert!(out3.events[0].cost_source.is_none());
    }

    #[test]
    fn missing_table_is_empty_not_error() {
        let dir = std::env::temp_dir().join(format!("gtt-mm-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("runtime-state.sqlite");
        Connection::open(&db).unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db,
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&MiniMaxCode, &store, &item);
        assert!(out.events.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
