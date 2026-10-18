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
//! `rowid` is monotone with insertion order → nothing is missed. Migrations
//! reset the cursor so all rows rescan once (dedup keys unchanged → upsert
//! replaces in place): `rowid-v2` (timestamp → rowid watermark), `rowid-v3`
//! (input semantics, below).
//!
//! Input semantics: `input_tokens` is what the provider reported. For the
//! OpenAI-style providers ZCode is used with (DeepSeek, Zhipu, custom
//! OpenAI-compatible endpoints) that is `prompt_tokens` — the FULL prompt,
//! cache reads included — while an Anthropic-style endpoint reports the
//! uncached part only. Stored as-is, a cached DeepSeek request was billed
//! entirely at the uncached rate. Each row is classified, best evidence
//! first: `raw_usage_json` (`prompt_tokens` → includes; Anthropic cache_*
//! fields without it → excludes), `provider_total_tokens` (= input + output
//! → includes; + cache → excludes), else input ≥ cache → includes. Included
//! cache is subtracted via `normalize::input_excludes_cache`.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::input_excludes_cache;
use crate::store::Store;
use anyhow::Result;
use std::path::PathBuf;

/// Cursor marker; a new value forces one full rescan (see module docs).
const STATE: &str = "rowid-v3";

/// Providers whose usage is already ingested by their own adapters (§6.4).
const CROSS_PROVIDERS: &[&str] = &["anthropic", "openai"];

/// Does this row's `input_tokens` include the cached tokens? (module docs)
pub(crate) fn input_includes_cache(
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    provider_total: Option<u64>,
    raw_usage: Option<&str>,
) -> bool {
    let cache = cache_read.saturating_add(cache_write);
    if cache == 0 {
        return false; // nothing to subtract either way
    }
    if let Some(raw) = raw_usage.and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()) {
        let has = |k: &str| raw.get(k).is_some() || raw.pointer(&format!("/usage/{k}")).is_some();
        if has("prompt_tokens") || has("promptTokens") {
            return true;
        }
        if has("cache_read_input_tokens") || has("cache_creation_input_tokens") {
            return false;
        }
    }
    if let Some(total) = provider_total.filter(|t| *t > 0) {
        if total == input.saturating_add(output)
            || total == input.saturating_add(output).saturating_add(reasoning)
        {
            return true;
        }
        if total == input.saturating_add(output).saturating_add(cache)
            || total
                == input
                    .saturating_add(output)
                    .saturating_add(reasoning)
                    .saturating_add(cache)
        {
            return false;
        }
    }
    input >= cache
}

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
        // Migration: pre-v3 cursors held a `started_at` epoch-ms watermark or a
        // v2 rowid watermark from the old input semantics. Reset once → full
        // rescan; dedup keys make it idempotent.
        let migrated = cur.state.as_deref() == Some(STATE);
        let since_rowid = if migrated { cur.offset as i64 } else { 0 };
        let conn = super::opencode::open_ro(&item.path)?;
        // Older ZCode builds lack the provider total / raw usage columns.
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('model_usage')")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        let col = |name: &str| {
            if columns.iter().any(|c| c == name) {
                name.to_string()
            } else {
                "NULL".to_string()
            }
        };
        let mut st = conn.prepare(&format!(
            "SELECT rowid, logical_request_id, attempt_index, session_id, turn_id,
                    provider_id, model_id, status, started_at, completed_at,
                    duration_ms, time_to_first_token_ms,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_creation_input_tokens, cache_read_input_tokens,
                    error_type, {}, {}
             FROM model_usage WHERE rowid > ?1 ORDER BY rowid",
            col("provider_total_tokens"),
            col("raw_usage_json"),
        ))?;
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
                r.get::<_, Option<i64>>(18)?,
                r.get::<_, Option<String>>(19)?,
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
                provider_total,
                raw_usage,
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

            let includes = input_includes_cache(
                tin,
                tout,
                treason,
                tcr,
                tcw,
                provider_total.map(|t| t.max(0) as u64),
                raw_usage.as_deref(),
            );
            let tin = if includes {
                input_excludes_cache(tin, tcr, tcw)
            } else {
                tin
            };
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
                state: Some(STATE.into()),
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

    #[test]
    fn input_semantics_are_classified_per_row() {
        // The reported case: DeepSeek prompt_tokens include the cache.
        assert!(input_includes_cache(
            478_589, 469, 0, 478_208, 0, None, None
        ));
        assert!(input_includes_cache(
            478_589,
            469,
            0,
            478_208,
            0,
            None,
            Some(
                r#"{"prompt_tokens":478589,"completion_tokens":469,"prompt_tokens_details":{"cached_tokens":478208}}"#
            )
        ));
        // Provider total = input + output → inclusive; total that also counts
        // the cache on top → exclusive.
        assert!(input_includes_cache(
            5_000,
            100,
            0,
            4_000,
            0,
            Some(5_100),
            None
        ));
        assert!(!input_includes_cache(
            300,
            100,
            0,
            4_000,
            0,
            Some(4_400),
            None
        ));
        // Anthropic-style raw usage: input excludes the cache.
        assert!(!input_includes_cache(
            9_000,
            50,
            0,
            2_000,
            500,
            None,
            Some(
                r#"{"input_tokens":9000,"output_tokens":50,"cache_read_input_tokens":2000,"cache_creation_input_tokens":500}"#
            )
        ));
        // Small fresh input after a big cached prefix: exclusive.
        assert!(!input_includes_cache(120, 30, 0, 50_000, 0, None, None));
        // No cache: nothing to subtract.
        assert!(!input_includes_cache(1_000, 10, 0, 0, 0, None, None));
    }

    #[test]
    fn cached_openai_style_rows_bill_only_the_uncached_part() {
        let (db, _dir) = fixture_db("semantics");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "ALTER TABLE model_usage ADD COLUMN provider_total_tokens INTEGER;
             ALTER TABLE model_usage ADD COLUMN raw_usage_json TEXT;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_usage(id, logical_request_id, attempt_index, session_id,
               provider_id, model_id, status, started_at, input_tokens, output_tokens,
               cache_read_input_tokens, provider_total_tokens)
             VALUES ('d', 'req-d', 0, 's', 'builtin:deepseek', 'deepseek-v4.1-flash',
               'completed', 1000000, 478589, 469, 478208, 479058)",
            [],
        )
        .unwrap();
        drop(conn);
        let store = Store::open_memory().unwrap();
        let out = ZCode.scan_sqlite(&item_for(&db), &store).unwrap();
        let e = &out.events[0];
        assert_eq!(e.input_tokens, 381, "478,589 − 478,208 cached");
        assert_eq!(e.cache_read_tokens, 478_208);
        assert_eq!(e.output_tokens, 469);
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
        assert_eq!(cur.state.as_deref(), Some(STATE));
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
