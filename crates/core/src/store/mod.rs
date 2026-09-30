//! SQLite storage layer (spec §5). WAL mode, single writer, UPSERT-by-completeness.

mod cursor;
mod query;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub use cursor::{CursorAction, FileCursor, tail_fingerprint};
pub use query::{
    ActivityDay, AppSummary, DailyRow, EventRow, MODEL_EXPR, PriceQuote, PriceRow, QuotaRow,
    ShareRow, SourceHealth, Totals, filter_prices,
};

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 1;

/// Default database location: `~/.globaltokentracker/ledger.db`.
///
/// `GTT_DATA_DIR` (a directory) relocates the ledger, `ui.json` and backups —
/// for tests and screenshots, so a scratch instance never touches real data.
pub fn default_db_path() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("GTT_DATA_DIR") {
        return std::path::PathBuf::from(dir).join("ledger.db");
    }
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = home.join(".globaltokentracker");
    // Rename-era migration: pre-rename builds stored the ledger at
    // ~/.codeledger. Move the whole dir (db + ui.json) once, in place.
    let legacy = home.join(".codeledger");
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &dir);
    }
    dir.join("ledger.db")
}

/// Bounded ledger snapshot: `backups/ledger.db` plus one previous
/// generation — the only single point of failure left (adapter source
/// data is append-only ingested, so losing a tool's local logs never
/// loses history; losing this db would).
pub const BACKUP_INTERVAL_MS: i64 = 86_400_000;
/// Quota poll rows older than this are pruned during the daily backup pass.
const QUOTA_RETENTION_MS: i64 = 30 * 86_400_000;

pub struct Store {
    conn: Connection,
    /// `Some` for file-backed ledgers — drives backup/restore. `None` for
    /// in-memory test stores, which skip persistence work entirely.
    path: Option<std::path::PathBuf>,
}

impl Store {
    /// Open (and migrate) the ledger at `path`. Use `:memory:` in tests.
    /// When the file is missing or fails to open/migrate, the newest
    /// snapshot under `backups/` is restored first — a tool wiping its
    /// data dir must not take the ledger with it.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create db dir {}", dir.display()))?;
        }
        if path.exists()
            && let Ok(s) = Self::try_open(path)
        {
            return Ok(s);
        }
        if Self::restore_backup(path).unwrap_or(false) {
            return Self::try_open(path);
        }
        Self::try_open(path)
    }

    fn try_open(path: &Path) -> Result<Self> {
        let conn =
            Connection::open(path).with_context(|| format!("open ledger {}", path.display()))?;
        let store = Self {
            conn,
            path: Some(path.to_path_buf()),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn, path: None };
        store.migrate()?;
        Ok(store)
    }

    /// Copy `backups/ledger.db` (or the previous generation) over a
    /// missing/broken ledger. Stale `-wal`/`-shm` sidecars are removed
    /// first — SQLite would otherwise replay them onto the restored file
    /// and report corruption again. Returns true when a snapshot landed.
    fn restore_backup(path: &Path) -> Result<bool> {
        let dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backups");
        for cand in [dir.join("ledger.db"), dir.join("ledger.prev.db")] {
            if !cand.exists() {
                continue;
            }
            if path.exists() {
                // Keep the broken file for forensics — never silently drop.
                let aside = path.with_extension("db.corrupt");
                let _ = std::fs::remove_file(&aside);
                let _ = std::fs::rename(path, &aside);
            }
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
            std::fs::copy(&cand, path)?;
            if Self::try_open(path).is_ok() {
                return Ok(true);
            }
            let _ = std::fs::remove_file(path);
        }
        Ok(false)
    }

    /// Throttled snapshot — cheap to call every scan (one KV read).
    pub fn maybe_backup(&self) -> Result<bool> {
        if self.path.is_none() {
            return Ok(false);
        }
        let last: i64 = self
            .get_state("backup_last_at")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if now_ms() - last < BACKUP_INTERVAL_MS {
            return Ok(false);
        }
        self.prune_quotas(QUOTA_RETENTION_MS)?;
        self.backup_now()?;
        self.set_state("backup_last_at", &now_ms().to_string())?;
        Ok(true)
    }

    /// Age out quota history rows — they accumulate one row per poll per key
    /// and nothing but `latest_quotas` reads them. The newest row per key is
    /// always kept so a stale-but-live quota still displays.
    pub fn prune_quotas(&self, older_than_ms: i64) -> Result<u64> {
        let cutoff = now_ms() - older_than_ms;
        let n = self.conn.execute(
            "DELETE FROM quota_snapshots WHERE captured_at < ?1 AND id NOT IN (
                 SELECT MAX(id) FROM quota_snapshots
                 GROUP BY app, account, window_kind)",
            params![cutoff],
        )?;
        Ok(n as u64)
    }

    /// `VACUUM INTO` yields a compacted, fully-checkpointed copy in one
    /// C-level pass; tmp+rename means a crash mid-copy leaves the old
    /// snapshot intact rather than a torn file.
    pub fn backup_now(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backups");
        std::fs::create_dir_all(&dir)?;
        let cur = dir.join("ledger.db");
        let prev = dir.join("ledger.prev.db");
        let tmp = dir.join("ledger.tmp");
        let _ = std::fs::remove_file(&tmp);
        self.conn
            .execute("VACUUM INTO ?1", params![tmp.to_string_lossy().as_ref()])?;
        if cur.exists() {
            let _ = std::fs::remove_file(&prev);
            std::fs::rename(&cur, &prev)?;
        }
        std::fs::rename(&tmp, &cur)?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;
        // v2: sync_cursors.adapter_state — idempotent for existing DBs.
        let has: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sync_cursors') WHERE name='adapter_state'",
            [],
            |r| r.get(0),
        )?;
        if has == 0 {
            self.conn
                .execute_batch("ALTER TABLE sync_cursors ADD COLUMN adapter_state TEXT")?;
        }
        // Long-context output / cache-read tiers on `prices` — idempotent.
        for col in ["tier_above_200k_output", "tier_above_200k_cache_read"] {
            let has: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('prices') WHERE name=?1",
                [col],
                |r| r.get(0),
            )?;
            if has == 0 {
                self.conn
                    .execute_batch(&format!("ALTER TABLE prices ADD COLUMN {col} REAL"))?;
            }
        }
        // OpenCode used to book output WITHOUT reasoning (its `tokens.reasoning`
        // is separate); the project convention is reasoning ⊂ output. One-time
        // fix for rows ingested before the adapter changed. Runs at open —
        // before any scan — so freshly ingested rows are never added twice.
        const OPENCODE_MIG: &str = "mig_opencode_output_includes_reasoning";
        if self.get_state(OPENCODE_MIG)?.is_none() {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE usage_events SET output_tokens = output_tokens + reasoning_tokens
                 WHERE app=?1 AND reasoning_tokens > 0",
                [crate::model::apps::OPENCODE],
            )?;
            tx.execute(
                "INSERT INTO app_state(key, value) VALUES (?1, '1')
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [OPENCODE_MIG],
            )?;
            tx.commit()?;
        }
        // Claude/Codex now derive per-call duration from log timestamps.
        // Dropping their cursors (adapter_state goes with the row) makes the
        // next scan re-read those files from offset 0; the re-emitted rows
        // carry a duration, hence higher completeness, hence overwrite.
        const RESCAN_DURATIONS: &str = "mig_rescan_durations_v1";
        if self.get_state(RESCAN_DURATIONS)?.is_none() {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM sync_cursors WHERE source IN (?1, ?2)",
                [crate::model::apps::CLAUDE, crate::model::apps::CODEX],
            )?;
            tx.execute(
                "INSERT INTO app_state(key, value) VALUES (?1, '1')
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [RESCAN_DURATIONS],
            )?;
            tx.commit()?;
        }
        let version: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        if version < SCHEMA_VERSION {
            self.conn.execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![SCHEMA_VERSION, now_ms()],
            )?;
        }
        Ok(())
    }

    /// Insert-or-merge a usage event.
    ///
    /// Iron rule (spec §5, avoids cc-switch #6994): never `INSERT OR IGNORE`.
    /// A conflicting row is only overwritten when the incoming row is at least
    /// as complete — streaming snapshots upgrade to terminal rows, never the
    /// reverse.
    pub fn upsert_event(&self, ev: &crate::model::UsageEvent) -> Result<bool> {
        let completeness = ev.completeness();
        let n = self.conn.execute(
            r#"INSERT INTO usage_events(
                 dedup_key, app, session_id, project, account_id, provider_id,
                 model, request_model, pricing_model, ts_start, ts_end,
                 input_tokens, output_tokens, reasoning_tokens,
                 cache_read_tokens, cache_write_5m_tokens, cache_write_1h_tokens,
                 credits, cost_usd, cost_source, provenance,
                 duration_ms, ttft_ms, active_ms, status, error, raw_ref, completeness)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
                       ?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28)
               ON CONFLICT(dedup_key) DO UPDATE SET
                 session_id=excluded.session_id, project=excluded.project,
                 account_id=excluded.account_id, provider_id=excluded.provider_id,
                 model=excluded.model, request_model=excluded.request_model,
                 pricing_model=excluded.pricing_model,
                 ts_start=excluded.ts_start, ts_end=excluded.ts_end,
                 input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens,
                 reasoning_tokens=excluded.reasoning_tokens,
                 cache_read_tokens=excluded.cache_read_tokens,
                 cache_write_5m_tokens=excluded.cache_write_5m_tokens,
                 cache_write_1h_tokens=excluded.cache_write_1h_tokens,
                 credits=excluded.credits, cost_usd=excluded.cost_usd,
                 cost_source=excluded.cost_source, provenance=excluded.provenance,
                 duration_ms=excluded.duration_ms, ttft_ms=excluded.ttft_ms,
                 active_ms=excluded.active_ms, status=excluded.status,
                 error=excluded.error, raw_ref=excluded.raw_ref,
                 completeness=excluded.completeness
               WHERE excluded.completeness >= usage_events.completeness
                 -- Only when something actually differs. A source that re-emits
                 -- the same row every pass (OpenCode's in-flight message) used
                 -- to be a "write" each time: every idle tick then looked like
                 -- new data and paid a full view reload.
                 AND (usage_events.session_id, usage_events.project, usage_events.account_id,
                      usage_events.provider_id, usage_events.model, usage_events.request_model,
                      usage_events.pricing_model, usage_events.ts_start, usage_events.ts_end,
                      usage_events.input_tokens, usage_events.output_tokens,
                      usage_events.reasoning_tokens, usage_events.cache_read_tokens,
                      usage_events.cache_write_5m_tokens, usage_events.cache_write_1h_tokens,
                      usage_events.credits, usage_events.cost_usd, usage_events.cost_source,
                      usage_events.provenance, usage_events.duration_ms, usage_events.ttft_ms,
                      usage_events.active_ms, usage_events.status, usage_events.error,
                      usage_events.raw_ref, usage_events.completeness)
                     IS NOT
                     (excluded.session_id, excluded.project, excluded.account_id,
                      excluded.provider_id, excluded.model, excluded.request_model,
                      excluded.pricing_model, excluded.ts_start, excluded.ts_end,
                      excluded.input_tokens, excluded.output_tokens,
                      excluded.reasoning_tokens, excluded.cache_read_tokens,
                      excluded.cache_write_5m_tokens, excluded.cache_write_1h_tokens,
                      excluded.credits, excluded.cost_usd, excluded.cost_source,
                      excluded.provenance, excluded.duration_ms, excluded.ttft_ms,
                      excluded.active_ms, excluded.status, excluded.error,
                      excluded.raw_ref, excluded.completeness)"#,
            params![
                ev.dedup_key,
                ev.app,
                ev.session_id,
                ev.project,
                ev.account_id,
                ev.provider_id,
                ev.model,
                ev.request_model,
                ev.pricing_model,
                ev.ts_start,
                ev.ts_end,
                ev.input_tokens as i64,
                ev.output_tokens as i64,
                ev.reasoning_tokens as i64,
                ev.cache_read_tokens as i64,
                ev.cache_write_5m_tokens as i64,
                ev.cache_write_1h_tokens as i64,
                ev.credits,
                ev.cost_usd,
                ev.cost_source.map(crate::model::CostSource::as_str),
                ev.provenance.as_str(),
                ev.duration_ms,
                ev.ttft_ms,
                ev.active_ms,
                ev.status,
                ev.error,
                ev.raw_ref,
                completeness,
            ],
        )?;
        Ok(n > 0)
    }

    /// Latest-cumulative upsert for OTLP data points (spec §③): re-exports of
    /// the same series overwrite in place; an older point never wins.
    pub fn upsert_otel_metric(
        &self,
        metric: &str,
        session_id: &str,
        attr_sig: &str,
        value: f64,
        ts_ms: i64,
        attrs_json: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT INTO otel_metrics(metric,session_id,attr_sig,value,ts_ms,received_at,attrs_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(metric,session_id,attr_sig) DO UPDATE SET
               value=excluded.value, ts_ms=excluded.ts_ms,
               received_at=excluded.received_at, attrs_json=excluded.attrs_json
             WHERE excluded.ts_ms >= otel_metrics.ts_ms",
            params![metric, session_id, attr_sig, value, ts_ms, now_ms(), attrs_json],
        )?;
        Ok(n > 0)
    }

    /// Delete every event owned by one adapter. Used when an adapter changes
    /// its event granularity/dedup-key scheme — stale rows under the old
    /// scheme would double-count against the new ones.
    pub fn delete_app_events(&self, app: &str) -> Result<u64> {
        let n = self
            .conn
            .execute("DELETE FROM usage_events WHERE app=?1", params![app])?;
        Ok(n as u64)
    }

    /// Insert a quota snapshot — skipped when identical in every user-visible
    /// field to the latest row for the same (app, account, window_kind), so
    /// unchanged subscription windows don't pile up duplicate history rows.
    /// `raw_json`/`captured_at` deltas alone never resurrect a skipped value.
    /// Returns `true` when a row was actually inserted.
    pub fn insert_quota(&self, q: &crate::model::QuotaSnapshot) -> Result<bool> {
        type QVals = (Option<f64>, Option<f64>, Option<f64>, Option<i64>);
        let latest: Option<QVals> = self
            .conn
            .query_row(
                "SELECT used, limit_value, used_percent, resets_at
                 FROM quota_snapshots
                 WHERE app = ?1 AND account IS ?2 AND window_kind = ?3
                 ORDER BY captured_at DESC, id DESC LIMIT 1",
                params![q.app, q.account, q.window_kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if latest == Some((q.used, q.limit_value, q.used_percent, q.resets_at)) {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO quota_snapshots(app,account,captured_at,window_kind,used,limit_value,used_percent,resets_at,raw_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![q.app, q.account, q.captured_at, q.window_kind, q.used,
                    q.limit_value, q.used_percent, q.resets_at, q.raw_json],
        )?;
        Ok(true)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// app_state 小 KV：价格抓取节流等跨会话状态。
    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        let mut st = self
            .conn
            .prepare("SELECT value FROM app_state WHERE key=?1")?;
        let mut rows = st.query(params![key])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    pub fn set_state(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO app_state(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostSource, Provenance, UsageEvent, apps};

    fn ev(key: &str, output: u64, cost: Option<f64>) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            app: apps::CLAUDE.into(),
            output_tokens: output,
            input_tokens: 100,
            cost_usd: cost,
            cost_source: cost.map(|_| CostSource::Computed),
            provenance: Provenance::LocalJsonl,
            ..Default::default()
        }
    }

    #[test]
    fn duration_rescan_drops_claude_and_codex_cursors_once() {
        let s = Store::open_memory().unwrap();
        let cursors = |s: &Store| -> Vec<String> {
            let mut st = s
                .conn()
                .prepare("SELECT source FROM sync_cursors ORDER BY file_path")
                .unwrap();
            st.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        let put = |s: &Store| {
            for (src, path) in [
                (apps::CLAUDE, "a"),
                (apps::CODEX, "b"),
                (apps::OPENCODE, "c"),
                (apps::CLAUDE, "d"),
            ] {
                s.conn()
                    .execute(
                        "INSERT INTO sync_cursors(source, file_path, last_byte_offset, adapter_state)
                         VALUES (?1, ?2, 10, '{}')",
                        [src, path],
                    )
                    .unwrap();
            }
        };
        put(&s);
        // A ledger from before the migration existed.
        s.conn()
            .execute(
                "DELETE FROM app_state WHERE key='mig_rescan_durations_v1'",
                [],
            )
            .unwrap();
        s.migrate().unwrap();
        assert_eq!(cursors(&s), [apps::OPENCODE]);
        // Second run is a no-op: cursors written since are kept.
        s.conn().execute("DELETE FROM sync_cursors", []).unwrap();
        put(&s);
        s.migrate().unwrap();
        assert_eq!(cursors(&s).len(), 4);
    }

    #[test]
    fn opencode_output_gains_reasoning_exactly_once() {
        let dir = std::env::temp_dir().join(format!("gtt_mig_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger.db");
        let out = |s: &Store, key: &str| -> i64 {
            s.conn()
                .query_row(
                    "SELECT output_tokens FROM usage_events WHERE dedup_key=?1",
                    [key],
                    |r| r.get(0),
                )
                .unwrap()
        };
        {
            let s = Store::open(&path).unwrap();
            let mut oc = ev("oc", 5, Some(0.0));
            oc.app = apps::OPENCODE.into();
            oc.reasoning_tokens = 30;
            let mut other = ev("cl", 5, Some(0.0));
            other.reasoning_tokens = 30;
            s.upsert_event(&oc).unwrap();
            s.upsert_event(&other).unwrap();
            // Simulate a ledger written before the migration existed.
            s.conn()
                .execute("DELETE FROM app_state WHERE key LIKE 'mig_opencode%'", [])
                .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(out(&s, "oc"), 35);
        assert_eq!(out(&s, "cl"), 5); // other tools already count it in output
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(out(&s, "oc"), 35); // no double add
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn upsert_upgrades_snapshot_to_terminal() {
        let s = Store::open_memory().unwrap();
        assert!(s.upsert_event(&ev("k1", 0, None)).unwrap()); // interim: no output/cost
        assert!(s.upsert_event(&ev("k1", 500, Some(0.01))).unwrap()); // terminal wins
        let total: i64 = s
            .conn()
            .query_row(
                "SELECT output_tokens FROM usage_events WHERE dedup_key='k1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 500);
    }

    /// Re-emitting a row unchanged must not count as a write (idle ticks would
    /// otherwise look like new data), while any real change still lands.
    #[test]
    fn identical_reupsert_is_not_a_write() {
        let s = Store::open_memory().unwrap();
        let e = ev("same", 300, Some(0.02));
        assert!(s.upsert_event(&e).unwrap()); // first insert
        assert!(!s.upsert_event(&e).unwrap()); // identical → nothing to do
        assert!(!s.upsert_event(&e).unwrap());
        // a genuine change (a cost the price book just learned) still writes
        let mut repriced = e.clone();
        repriced.cost_usd = Some(0.03);
        assert!(s.upsert_event(&repriced).unwrap());
        // and NULL-vs-NULL columns compare equal (IS NOT, not <>)
        let mut nul = ev("nul", 10, None);
        nul.project = None;
        assert!(s.upsert_event(&nul).unwrap());
        assert!(!s.upsert_event(&nul).unwrap());
        let cost: f64 = s
            .conn()
            .query_row(
                "SELECT cost_usd FROM usage_events WHERE dedup_key='same'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cost, 0.03);
    }

    #[test]
    fn upsert_never_downgrades() {
        let s = Store::open_memory().unwrap();
        assert!(s.upsert_event(&ev("k2", 500, Some(0.01))).unwrap()); // terminal first
        // Interim re-scan arrives late — must NOT overwrite richer row.
        assert!(!s.upsert_event(&ev("k2", 0, None)).unwrap());
        let total: i64 = s
            .conn()
            .query_row(
                "SELECT output_tokens FROM usage_events WHERE dedup_key='k2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 500);
    }

    #[test]
    fn completeness_counts_dims() {
        assert_eq!(ev("x", 0, None).completeness(), 1); // input only
        assert_eq!(ev("x", 1, Some(0.1)).completeness(), 3);
    }

    // 2025-01-15T23:30:00Z — UTC date 15th, but +08:00 rolls to the 16th.
    const BOUNDARY_MS: i64 = 1_736_983_800_000;

    fn ev_ts(key: &str, ts_ms: i64) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            app: apps::CLAUDE.into(),
            ts_start: Some(ts_ms),
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }
    }

    #[test]
    fn rollups_respect_local_date_boundary() {
        let s = Store::open_memory().unwrap();
        s.upsert_event(&ev_ts("b1", BOUNDARY_MS)).unwrap();
        s.upsert_event(&ev_ts("b2", BOUNDARY_MS + 3_600_000))
            .unwrap(); // 00:30Z
        s.rebuild_rollups("+00:00").unwrap();
        let (d15, d16): (i64, i64) = s
            .conn()
            .query_row(
                "SELECT (SELECT events FROM daily_rollups WHERE date='2025-01-15'),
                        (SELECT events FROM daily_rollups WHERE date='2025-01-16')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((d15, d16), (1, 1)); // 23:30Z stays on 15th; 00:30Z lands 16th
        // +08:00 pushes both events into the 16th — old-dated rows must go.
        s.rebuild_rollups("+08:00").unwrap();
        let (d16, leftover): (i64, i64) = s
            .conn()
            .query_row(
                "SELECT (SELECT events FROM daily_rollups WHERE date='2025-01-16'),
                        (SELECT COUNT(*) FROM daily_rollups WHERE date='2025-01-15')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((d16, leftover), (2, 0));
    }

    #[test]
    fn rollups_idempotent() {
        let s = Store::open_memory().unwrap();
        s.upsert_event(&ev_ts("r1", BOUNDARY_MS)).unwrap();
        let n1 = s.rebuild_rollups("+00:00").unwrap();
        let n2 = s.rebuild_rollups("+00:00").unwrap();
        assert_eq!((n1, n2), (1, 1));
        let total: i64 = s
            .conn()
            .query_row("SELECT SUM(events) FROM daily_rollups", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1);
    }

    /// Partial rebuild of only the touched days must yield byte-identical
    /// `daily_rollups` rows to a full rebuild — the scan path relies on it.
    #[test]
    fn rollups_partial_rebuild_equals_full() {
        let dump = |s: &Store| -> Vec<String> {
            let mut st = s
                .conn()
                .prepare(
                    "SELECT date||'|'||app||'|'||provider||'|'||request_model||'|'||
                            pricing_model||'|'||events||'|'||input_tokens||'|'||
                            output_tokens||'|'||COALESCE(cost_usd,-1)
                     FROM daily_rollups ORDER BY 1",
                )
                .unwrap();
            st.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        let s = Store::open_memory().unwrap();
        // BOUNDARY_MS = 2025-01-15 23:30Z; +1h lands on the 16th (UTC frame).
        s.upsert_event(&ev_ts("p1", BOUNDARY_MS)).unwrap();
        s.upsert_event(&ev_ts("p2", BOUNDARY_MS + 3_600_000))
            .unwrap();
        s.rebuild_rollups("+00:00").unwrap();

        // A write lands on the 15th only — day index of that ts in the
        // +00:00 frame is what the engine feeds rebuild_rollup_days.
        let mut extra = ev_ts("p3", BOUNDARY_MS);
        extra.output_tokens = 99;
        s.upsert_event(&extra).unwrap();
        let day = (BOUNDARY_MS).div_euclid(86_400_000);
        let days = std::collections::BTreeSet::from([day]);
        let n = s.rebuild_rollup_days(&days, "+00:00").unwrap();
        assert_eq!(n, 1); // one (date,app,…) group rewritten
        let partial = dump(&s);

        s.rebuild_rollups("+00:00").unwrap();
        assert_eq!(partial, dump(&s));

        // Untouched days survive a partial rebuild verbatim.
        let d16: i64 = s
            .conn()
            .query_row(
                "SELECT events FROM daily_rollups WHERE date='2025-01-16'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(d16, 1);
    }

    #[test]
    fn rollups_partial_rebuild_offset_frame() {
        let s = Store::open_memory().unwrap();
        // 23:30Z on the 15th is 07:30 on the 16th in +08:00 — the day index
        // must be computed in that same shifted frame or the DELETE misses.
        s.upsert_event(&ev_ts("o1", BOUNDARY_MS)).unwrap();
        let off = crate::viewmodel::utc_offset_ms("+08:00").unwrap();
        let day = (BOUNDARY_MS + off).div_euclid(86_400_000);
        let n = s
            .rebuild_rollup_days(&std::collections::BTreeSet::from([day]), "+08:00")
            .unwrap();
        assert_eq!(n, 1);
        let d16: i64 = s
            .conn()
            .query_row(
                "SELECT events FROM daily_rollups WHERE date='2025-01-16'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(d16, 1);
        // Empty set is a no-op, never wipes the table.
        assert_eq!(
            s.rebuild_rollup_days(&std::collections::BTreeSet::new(), "+00:00")
                .unwrap(),
            0
        );
        assert_eq!(d16, 1);
    }

    #[test]
    fn prune_preserves_rollups() {
        let s = Store::open_memory().unwrap();
        s.upsert_event(&ev_ts("old", BOUNDARY_MS)).unwrap();
        s.upsert_event(&ev_ts("new", BOUNDARY_MS + 86_400_000 * 200))
            .unwrap();
        s.rebuild_rollups("+00:00").unwrap();
        let pruned = s.prune_events(BOUNDARY_MS + 86_400_000 * 90).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(s.event_count(None, None).unwrap(), 1);
        // Both rollup rows survive — the pruned day's aggregate included.
        let kept: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM daily_rollups", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 2);
    }

    #[test]
    fn hourly_buckets_by_local_hour() {
        let s = Store::open_memory().unwrap();
        // 2025-01-15 10:30Z + 10:59Z → same "10:00" bucket; 11:05Z → next.
        s.upsert_event(&ev_ts("h1", 1_736_937_000_000)).unwrap();
        s.upsert_event(&ev_ts("h2", 1_736_937_000_000 + 1_700_000))
            .unwrap();
        s.upsert_event(&ev_ts("h3", 1_736_937_000_000 + 3_000_000))
            .unwrap();
        let rows = s.hourly(0, "+00:00", None, None).unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.date.as_str()).collect();
        // h1+h2 fold into one 10:00 bucket; h3 lands in 11:00.
        assert_eq!(labels, ["10:00", "11:00"]);
        assert_eq!(rows[0].events, 2);
    }

    #[test]
    fn app_filter_scopes_queries() {
        let s = Store::open_memory().unwrap();
        let mut a = ev_ts("a1", BOUNDARY_MS);
        a.app = "claude".into();
        a.model = Some("opus".into());
        let mut b = ev_ts("b1", BOUNDARY_MS);
        b.app = "codex".into();
        b.model = Some("gpt-x".into());
        s.upsert_event(&a).unwrap();
        s.upsert_event(&b).unwrap();
        let only = |name: &str| vec![name.to_string()];
        // None = all; Some(subset) scopes; Some(empty) = nothing.
        assert_eq!(s.event_count(None, None).unwrap(), 2);
        assert_eq!(
            s.event_count(Some(only("claude").as_slice()), None)
                .unwrap(),
            1
        );
        assert_eq!(s.event_count(Some(&[]), None).unwrap(), 0);
        let t = s
            .totals(None, None, Some(only("codex").as_slice()), None)
            .unwrap();
        assert_eq!(t.events, 1);
        assert_eq!(
            s.by_app(None, None, Some(only("claude").as_slice()), None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.app_names().unwrap(),
            vec!["claude".to_string(), "codex".to_string()]
        );
        let rows = s
            .events_page(10, 0, Some(only("codex").as_slice()), None)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].app, "codex");
    }

    #[test]
    fn model_filter_scopes_queries() {
        let s = Store::open_memory().unwrap();
        let mut a = ev_ts("a1", BOUNDARY_MS);
        a.model = Some("opus".into());
        let mut b = ev_ts("b1", BOUNDARY_MS);
        b.app = "codex".into();
        b.model = Some("gpt-x".into());
        let mut c = ev_ts("c1", BOUNDARY_MS); // no model → falls into "?"
        c.app = "codex".into();
        s.upsert_event(&a).unwrap();
        s.upsert_event(&b).unwrap();
        s.upsert_event(&c).unwrap();
        let only = |name: &str| vec![name.to_string()];
        // Distinct display-model list: named models + the "?" bucket.
        assert_eq!(
            s.model_names(None).unwrap(),
            vec!["?".to_string(), "gpt-x".to_string(), "opus".to_string()]
        );
        // Named model scopes; "?" is filterable too.
        assert_eq!(
            s.event_count(None, Some(only("opus").as_slice())).unwrap(),
            1
        );
        assert_eq!(s.event_count(None, Some(only("?").as_slice())).unwrap(), 1);
        assert_eq!(s.event_count(None, Some(&[])).unwrap(), 0);
        // Filters combine: app × model intersects honestly.
        let t = s
            .totals(
                None,
                None,
                Some(only("claude").as_slice()),
                Some(only("gpt-x").as_slice()),
            )
            .unwrap();
        assert_eq!(t.events, 0);
        // model_names cascades off the app filter: scoping to an app that
        // only emits "opus" drops "gpt-x" from the checklist.
        assert_eq!(
            s.model_names(Some(only("claude").as_slice())).unwrap(),
            vec!["opus".to_string()]
        );
        // Detail rows honor the model filter.
        let rows = s
            .events_page(10, 0, None, Some(only("gpt-x").as_slice()))
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model.as_deref(), Some("gpt-x"));
    }

    fn quota_snap(app: &str, kind: &str, pct: Option<f64>, at: i64) -> crate::model::QuotaSnapshot {
        crate::model::QuotaSnapshot {
            app: app.into(),
            account: None,
            captured_at: at,
            window_kind: kind.into(),
            used: Some(10.0),
            limit_value: Some(100.0),
            used_percent: pct,
            resets_at: None,
            raw_json: None,
        }
    }

    fn tmp_db(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gtt-backup-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("ledger.db")
    }

    #[test]
    fn backup_restores_missing_ledger() {
        let db = tmp_db("missing");
        {
            let s = Store::open(&db).unwrap();
            s.upsert_event(&ev("keep", 7, Some(0.01))).unwrap();
            s.backup_now().unwrap();
            assert!(db.parent().unwrap().join("backups/ledger.db").exists());
        } // drop the open connection before deleting the file
        std::fs::remove_file(&db).unwrap();
        let s = Store::open(&db).unwrap();
        let n: i64 = s
            .conn()
            .query_row(
                "SELECT output_tokens FROM usage_events WHERE dedup_key='keep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 7);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn backup_restores_corrupt_ledger() {
        let db = tmp_db("corrupt");
        {
            let s = Store::open(&db).unwrap();
            s.upsert_event(&ev("safe", 9, None)).unwrap();
            s.backup_now().unwrap();
        }
        std::fs::write(&db, b"not a sqlite file at all").unwrap();
        let s = Store::open(&db).unwrap();
        let n: i64 = s
            .conn()
            .query_row(
                "SELECT output_tokens FROM usage_events WHERE dedup_key='safe'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 9);
        // The broken original is kept aside for forensics, not deleted.
        assert!(db.with_extension("db.corrupt").exists());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn backup_rotates_two_generations_and_throttles() {
        let db = tmp_db("rotate");
        let s = Store::open(&db).unwrap();
        s.upsert_event(&ev("g1", 1, None)).unwrap();
        s.backup_now().unwrap();
        s.upsert_event(&ev("g2", 2, None)).unwrap();
        s.backup_now().unwrap();
        let dir = db.parent().unwrap().join("backups");
        assert!(dir.join("ledger.db").exists());
        assert!(dir.join("ledger.prev.db").exists());
        // Newest snapshot carries the second event.
        let prev = Store::open(&dir.join("ledger.prev.db")).unwrap();
        assert_eq!(prev.event_count(None, None).unwrap(), 1);
        // maybe_backup honors the 24h throttle stamp.
        s.set_state("backup_last_at", &now_ms().to_string())
            .unwrap();
        assert!(!s.maybe_backup().unwrap());
        // In-memory stores never attempt file work.
        assert!(!Store::open_memory().unwrap().maybe_backup().unwrap());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn insert_quota_dedups_identical_snapshots() {
        let s = Store::open_memory().unwrap();
        let q = quota_snap("workbuddy", "session_ctx", Some(21.5), 1000);
        s.insert_quota(&q).unwrap();
        // Same values re-polled later — must not append.
        let mut q2 = q.clone();
        q2.captured_at = 2000;
        s.insert_quota(&q2).unwrap();
        let n: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM quota_snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        // A real change lands.
        let mut q3 = q.clone();
        q3.captured_at = 3000;
        q3.used_percent = Some(42.0);
        s.insert_quota(&q3).unwrap();
        let n: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM quota_snapshots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn latest_quotas_one_row_per_key() {
        let s = Store::open_memory().unwrap();
        // Flood: many snapshots sharing the MAX captured_at (WorkBuddy's
        // batch-updated session watermarks did exactly this).
        for i in 0..50 {
            let mut q = quota_snap("workbuddy", "session_ctx", Some(21.5), 999);
            q.raw_json = Some(format!("dup{i}"));
            s.conn()
                .execute(
                    "INSERT INTO quota_snapshots(app,account,captured_at,window_kind,used,limit_value,used_percent,resets_at,raw_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![q.app, q.account, q.captured_at, q.window_kind, q.used,
                            q.limit_value, q.used_percent, q.resets_at, q.raw_json],
                )
                .unwrap();
        }
        s.insert_quota(&quota_snap("codex", "weekly", Some(3.0), 500))
            .unwrap();
        s.insert_quota(&quota_snap("codex", "5h_block", Some(80.0), 600))
            .unwrap();
        let rows = s.latest_quotas().unwrap();
        assert_eq!(rows.len(), 3); // exactly one per (app, kind)
        assert_eq!(rows.iter().filter(|r| r.app == "workbuddy").count(), 1);
    }

    /// The indexed rewrite must agree with the original window-function query
    /// on awkward data: NULL accounts (`IS`, not `=`), snapshots tied on
    /// `captured_at` (id breaks the tie) and several windows per app.
    #[test]
    fn latest_quotas_matches_the_window_function_oracle() {
        let s = Store::open_memory().unwrap();
        let mut n = 0i64;
        for (app, account, kind) in [
            ("claude", None, "5h_block"),
            ("claude", None, "weekly"),
            ("claude", Some("acct-a"), "weekly"),
            ("codex", Some("acct-a"), "weekly"),
            ("codex", None, "credits"),
            ("cursor", Some("acct-b"), "monthly"),
        ] {
            for captured_at in [100i64, 200, 200, 300, 300, 300, 150] {
                n += 1;
                s.conn()
                    .execute(
                        "INSERT INTO quota_snapshots(app,account,captured_at,window_kind,used,limit_value,used_percent,resets_at,raw_json)
                         VALUES (?1,?2,?3,?4,?5,NULL,?6,NULL,?7)",
                        params![app, account, captured_at, kind, n as f64, n as f64 / 7.0, "x".repeat(50)],
                    )
                    .unwrap();
            }
        }
        type Row = (String, Option<String>, i64, String, Option<f64>);
        let oracle: Vec<Row> = s
            .conn()
            .prepare(
                "SELECT app, account, captured_at, window_kind, used
                 FROM (SELECT *, ROW_NUMBER() OVER (
                           PARTITION BY app, account, window_kind
                           ORDER BY captured_at DESC, id DESC) rn
                       FROM quota_snapshots)
                 WHERE rn = 1 ORDER BY app, window_kind, account",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let got: Vec<_> = s
            .latest_quotas()
            .unwrap()
            .into_iter()
            .map(|q| (q.app, q.account, q.captured_at, q.window_kind, q.used))
            .collect();
        assert_eq!(got.len(), 6);
        assert_eq!(got, oracle);
    }

    #[test]
    fn prune_quotas_ages_history_but_keeps_latest_per_key() {
        let s = Store::open_memory().unwrap();
        let now = now_ms();
        let old = now - 40 * 86_400_000; // beyond the 30d retention window
        // Two stale rows + one fresh row for one key; a stale-only key too.
        for (kind, ts) in [
            ("session_ctx", old),
            ("session_ctx", old + 1),
            ("session_ctx", now),
            ("weekly", old),
        ] {
            let mut q = quota_snap("wb", kind, Some(50.0), ts);
            q.raw_json = Some(format!("ts{ts}"));
            s.conn()
                .execute(
                    "INSERT INTO quota_snapshots(app,account,captured_at,window_kind,used,limit_value,used_percent,resets_at,raw_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![q.app, q.account, q.captured_at, q.window_kind, q.used,
                            q.limit_value, q.used_percent, q.resets_at, q.raw_json],
                )
                .unwrap();
        }
        let n = s.prune_quotas(30 * 86_400_000).unwrap();
        assert_eq!(n, 2); // two stale session_ctx rows die
        let rows = s.latest_quotas().unwrap();
        // Fresh session_ctx + the kept-latest stale weekly both survive.
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .find(|r| r.window_kind == "weekly")
                .unwrap()
                .captured_at,
            old
        );
    }
}
