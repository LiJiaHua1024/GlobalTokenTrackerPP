//! SQLite storage layer (spec §5). WAL mode, single writer, UPSERT-by-completeness.

mod cursor;
mod query;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub use cursor::{CursorAction, FileCursor, tail_fingerprint};
pub use query::{
    AppSummary, DailyRow, EventRow, PriceRow, QuotaRow, SourceHealth, Totals,
};

const SCHEMA: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 1;

/// Default database location: `~/.globaltokentracker/ledger.db`.
pub fn default_db_path() -> std::path::PathBuf {
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

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (and migrate) the ledger at `path`. Use `:memory:` in tests.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("create db dir {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("open ledger {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
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
               WHERE excluded.completeness >= usage_events.completeness"#,
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
        s.upsert_event(&ev_ts("b2", BOUNDARY_MS + 3_600_000)).unwrap(); // 00:30Z
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

    #[test]
    fn prune_preserves_rollups() {
        let s = Store::open_memory().unwrap();
        s.upsert_event(&ev_ts("old", BOUNDARY_MS)).unwrap();
        s.upsert_event(&ev_ts("new", BOUNDARY_MS + 86_400_000 * 200))
            .unwrap();
        s.rebuild_rollups("+00:00").unwrap();
        let pruned = s
            .prune_events(BOUNDARY_MS + 86_400_000 * 90)
            .unwrap();
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
        assert_eq!(
            s.event_count(None, Some(only("?").as_slice())).unwrap(),
            1
        );
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
        assert_eq!(
            rows.iter().filter(|r| r.app == "workbuddy").count(),
            1
        );
    }
}

