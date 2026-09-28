//! Read-side queries — these are also the ViewModel SQL for the UI shell
//! (core owns all SQL so the shell stays dumb and portable).

use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone, Default)]
pub struct Totals {
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub credits: f64,
    pub cost_usd: f64,
    pub active_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AppSummary {
    pub app: String,
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub credits: f64,
    pub cost_usd: f64,
}

/// One share-dimension entry for the cost pie — name + the two metrics the
/// legend shows.
#[derive(Debug, Clone)]
pub struct ShareRow {
    pub name: String,
    pub events: u64,
    /// input+output+cache_read — same headline convention as `bucket_models`.
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct QuotaRow {
    pub app: String,
    pub account: Option<String>,
    pub captured_at: i64,
    pub window_kind: String,
    pub used: Option<f64>,
    pub limit_value: Option<f64>,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PriceRow {
    pub model: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct SourceHealth {
    pub source: String,
    pub enabled: bool,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
    pub files_seen: u64,
    pub rows_ingested: u64,
    pub cursors: u64,
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub app: String,
    pub model: Option<String>,
    pub pricing_model: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub ts_start: Option<i64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub credits: Option<f64>,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub duration_ms: Option<i64>,
    pub raw_ref: Option<String>,
}

/// One row of the trend tooltip query: bucket ("YYYY-MM-DD"/"HH:00") × model.
#[derive(Debug, Clone)]
pub struct BucketModelRow {
    pub bucket: String,
    pub model: String,
    pub events: u64,
    /// input+output+cache_read — the headline token count (trend convention).
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct DailyRow {
    pub date: String, // YYYY-MM-DD in `tz`
    pub app: String,
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub credits: f64,
}

impl super::Store {
    /// Totals over `[from_ms, to_ms)`; `None,None` = all time.
    /// `apps`/`models` restrict scope; `Some(empty)` = honest empty result.
    pub fn totals(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Totals> {
        let (w, p) = scope_where(from_ms, to_ms, apps, models);
        self.conn()
            .query_row(
                &format!(
                "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                        COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                        COALESCE(SUM(credits),0), COALESCE(SUM(cost_usd),0),
                        COALESCE(SUM(active_ms),0)
                 FROM usage_events {w}"
            ),
                rusqlite::params_from_iter(p.iter()),
                |r| {
                    Ok(Totals {
                        events: r.get::<_, i64>(0)? as u64,
                        input_tokens: r.get::<_, i64>(1)? as u64,
                        output_tokens: r.get::<_, i64>(2)? as u64,
                        reasoning_tokens: r.get::<_, i64>(3)? as u64,
                        cache_read_tokens: r.get::<_, i64>(4)? as u64,
                        cache_write_tokens: r.get::<_, i64>(5)? as u64,
                        credits: r.get(6)?,
                        cost_usd: r.get(7)?,
                        active_ms: r.get::<_, i64>(8)? as u64,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn by_app(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<AppSummary>> {
        let (w, p) = scope_where(from_ms, to_ms, apps, models);
        let mut st = self.conn().prepare(&format!(
            "SELECT app, COUNT(*),
                    COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                    COALESCE(SUM(credits),0), COALESCE(SUM(cost_usd),0)
             FROM usage_events {w} GROUP BY app ORDER BY cost_usd DESC"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(AppSummary {
                app: r.get(0)?,
                events: r.get::<_, i64>(1)? as u64,
                input_tokens: r.get::<_, i64>(2)? as u64,
                output_tokens: r.get::<_, i64>(3)? as u64,
                reasoning_tokens: r.get::<_, i64>(4)? as u64,
                cache_read_tokens: r.get::<_, i64>(5)? as u64,
                cache_write_tokens: r.get::<_, i64>(6)? as u64,
                credits: r.get(7)?,
                cost_usd: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per-model aggregation for the share pie — `MODEL_EXPR` identity, so
    /// slice names match the model filter's checkbox labels exactly.
    pub fn by_model(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<ShareRow>> {
        let (w, p) = scope_where(from_ms, to_ms, apps, models);
        let mut st = self.conn().prepare(&format!(
            "SELECT {MODEL_EXPR}, COUNT(*),
                    COALESCE(SUM(input_tokens+output_tokens+cache_read_tokens),0),
                    COALESCE(SUM(cost_usd),0)
             FROM usage_events {w} GROUP BY 1 ORDER BY cost_usd DESC"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(ShareRow {
                name: r.get(0)?,
                events: r.get::<_, i64>(1)? as u64,
                tokens: r.get::<_, i64>(2)? as u64,
                cost_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per-day aggregation. `utc_offset` is a fixed `+HH:MM`/`-HH:MM` string
    /// (validated) folded into the strftime modifier — local-midnight aligned.
    pub fn daily(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        utc_offset: &str,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<DailyRow>> {
        let b = utc_offset.as_bytes();
        anyhow::ensure!(
            b.len() == 6
                && matches!(b[0], b'+' | b'-')
                && b[3] == b':'
                && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit()),
            "invalid utc_offset: {utc_offset}"
        );
        let (w, p) = scope_where(from_ms, to_ms, apps, models);
        let mut st = self.conn().prepare(&format!(
            "SELECT strftime('%Y-%m-%d', ts_start/1000, 'unixepoch', '{utc_offset}') AS d, app,
                    COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                    COALESCE(SUM(cost_usd),0), COALESCE(SUM(credits),0)
             FROM usage_events {w}
             GROUP BY d, app ORDER BY d"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(DailyRow {
                date: r.get(0)?,
                app: r.get(1)?,
                events: r.get::<_, i64>(2)? as u64,
                input_tokens: r.get::<_, i64>(3)? as u64,
                output_tokens: r.get::<_, i64>(4)? as u64,
                reasoning_tokens: r.get::<_, i64>(5)? as u64,
                cache_read_tokens: r.get::<_, i64>(6)? as u64,
                cache_write_tokens: r.get::<_, i64>(7)? as u64,
                cost_usd: r.get(8)?,
                credits: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per-hour aggregation within `[from_ms, ∞)` — used for the "today"
    /// range where daily granularity collapses to a single bar. Reuses
    /// `DailyRow` with `date` = "HH:00" local label.
    pub fn hourly(
        &self,
        from_ms: i64,
        utc_offset: &str,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<DailyRow>> {
        let b = utc_offset.as_bytes();
        anyhow::ensure!(
            b.len() == 6
                && matches!(b[0], b'+' | b'-')
                && b[3] == b':'
                && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit()),
            "invalid utc_offset: {utc_offset}"
        );
        let (w, p) = scope_where(Some(from_ms), None, apps, models);
        let mut st = self.conn().prepare(&format!(
            "SELECT strftime('%H:00', ts_start/1000, 'unixepoch', '{utc_offset}') AS h, app,
                    COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                    COALESCE(SUM(cost_usd),0), COALESCE(SUM(credits),0)
             FROM usage_events {w}
             GROUP BY h, app ORDER BY h"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(DailyRow {
                date: r.get(0)?,
                app: r.get(1)?,
                events: r.get::<_, i64>(2)? as u64,
                input_tokens: r.get::<_, i64>(3)? as u64,
                output_tokens: r.get::<_, i64>(4)? as u64,
                reasoning_tokens: r.get::<_, i64>(5)? as u64,
                cache_read_tokens: r.get::<_, i64>(6)? as u64,
                cache_write_tokens: r.get::<_, i64>(7)? as u64,
                cost_usd: r.get(8)?,
                credits: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per-bucket × model aggregation feeding the trend tooltip (top-3 models
    /// per day/hour). `hourly` switches the bucket strftime to "HH:00".
    pub fn bucket_models(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        hourly: bool,
        utc_offset: &str,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<BucketModelRow>> {
        let b = utc_offset.as_bytes();
        anyhow::ensure!(
            b.len() == 6
                && matches!(b[0], b'+' | b'-')
                && b[3] == b':'
                && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit()),
            "invalid utc_offset: {utc_offset}"
        );
        let fmt = if hourly { "%H:00" } else { "%Y-%m-%d" };
        let (w, p) = scope_where(from_ms, to_ms, apps, models);
        let mut st = self.conn().prepare(&format!(
            "SELECT strftime('{fmt}', ts_start/1000, 'unixepoch', '{utc_offset}') AS k,
                    {MODEL_EXPR},
                    COUNT(*),
                    COALESCE(SUM(input_tokens+output_tokens+cache_read_tokens),0),
                    COALESCE(SUM(cost_usd),0)
             FROM usage_events {w}
             GROUP BY k, 2 ORDER BY k"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(BucketModelRow {
                bucket: r.get(0)?,
                model: r.get(1)?,
                events: r.get::<_, i64>(2)? as u64,
                tokens: r.get::<_, i64>(3)? as u64,
                cost_usd: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Reconciliation: per-app cost+token totals, for `reconcile` vs cc-switch.
    pub fn reconcile_summary(&self, app: &str) -> Result<Totals> {
        self.conn()
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                    COALESCE(SUM(credits),0), COALESCE(SUM(cost_usd),0),
                    COALESCE(SUM(active_ms),0)
             FROM usage_events WHERE app=?1",
                params![app],
                |r| {
                    Ok(Totals {
                        events: r.get::<_, i64>(0)? as u64,
                        input_tokens: r.get::<_, i64>(1)? as u64,
                        output_tokens: r.get::<_, i64>(2)? as u64,
                        reasoning_tokens: r.get::<_, i64>(3)? as u64,
                        cache_read_tokens: r.get::<_, i64>(4)? as u64,
                        cache_write_tokens: r.get::<_, i64>(5)? as u64,
                        credits: r.get(6)?,
                        cost_usd: r.get(7)?,
                        active_ms: r.get::<_, i64>(8)? as u64,
                    })
                },
            )
            .map_err(Into::into)
    }

    /// `(file_path, adapter_state)` for a source — reconcile rebuilds session
    /// watermarks from these (Codex stores each file's last cumulative counter).
    pub fn adapter_states(&self, source: &str) -> Result<Vec<(String, String)>> {
        let mut st = self.conn().prepare(
            "SELECT file_path, adapter_state FROM sync_cursors
             WHERE source=?1 AND adapter_state IS NOT NULL",
        )?;
        let rows = st.query_map(params![source], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Latest quota snapshot per (app, account, window_kind) — exactly one row
    /// per identity key (`ROW_NUMBER` top-1; a `MAX(captured_at)=` join would
    /// return every row tied at the max timestamp, which is how WorkBuddy's
    /// batch-updated `session_ctx` watermarks flooded the quota page).
    pub fn latest_quotas(&self) -> Result<Vec<QuotaRow>> {
        let mut st = self.conn().prepare(
            "SELECT app, account, captured_at, window_kind, used, limit_value,
                    used_percent, resets_at
             FROM (SELECT *, ROW_NUMBER() OVER (
                       PARTITION BY app, account, window_kind
                       ORDER BY captured_at DESC, id DESC) rn
                   FROM quota_snapshots)
             WHERE rn = 1
             ORDER BY app, window_kind, account",
        )?;
        let rows = st.query_map([], |r| {
            Ok(QuotaRow {
                app: r.get(0)?,
                account: r.get(1)?,
                captured_at: r.get(2)?,
                window_kind: r.get(3)?,
                used: r.get(4)?,
                limit_value: r.get(5)?,
                used_percent: r.get(6)?,
                resets_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Effective price book rows for the prices page: one row per model_id —
    /// the row that `PriceBook::load` precedence would actually resolve
    /// (seed < litellm < models.dev < llmpricing; ties → newest fetched_at).
    pub fn price_rows(&self, limit: i64) -> Result<Vec<PriceRow>> {
        let mut st = self.conn().prepare(
            "SELECT model_id, input, output, cache_read, cache_write, source FROM (
                 SELECT model_id, input, output, cache_read, cache_write, source,
                        ROW_NUMBER() OVER (
                          PARTITION BY model_id
                          ORDER BY CASE source
                              WHEN 'seed' THEN 0 WHEN 'litellm' THEN 1
                              WHEN 'models.dev' THEN 2 ELSE 3 END DESC,
                              fetched_at DESC
                        ) AS rn
                 FROM prices
             ) WHERE rn = 1
             ORDER BY model_id LIMIT ?1",
        )?;
        let rows = st.query_map(params![limit], |r| {
            // Columns are nullable — llmpricing rows carry NULL cache_write.
            Ok(PriceRow {
                model: r.get(0)?,
                input: r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                output: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                cache_read: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                cache_write: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                source: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Models seen in events that resolved to no price (unpriced badge list).
    pub fn unpriced_models(&self) -> Result<Vec<(String, u64)>> {
        let mut st = self.conn().prepare(
            "SELECT COALESCE(model, request_model, '?'), COUNT(*) FROM usage_events
             WHERE pricing_model IS NULL AND cost_source='unpriced'
             GROUP BY 1 ORDER BY 2 DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Per-source health rows for the data-sources page.
    pub fn source_health(&self) -> Result<Vec<SourceHealth>> {
        let mut st = self.conn().prepare(
            "SELECT s.source, s.enabled, s.last_synced_at, s.last_error,
                    s.files_seen, s.rows_ingested,
                    (SELECT COUNT(*) FROM sync_cursors c WHERE c.source=s.source) AS cursors
             FROM sources s ORDER BY s.source",
        )?;
        let rows = st.query_map([], |r| {
            Ok(SourceHealth {
                source: r.get(0)?,
                enabled: r.get::<_, i64>(1)? != 0,
                last_synced_at: r.get(2)?,
                last_error: r.get(3)?,
                files_seen: r.get::<_, i64>(4)? as u64,
                rows_ingested: r.get::<_, i64>(5)? as u64,
                cursors: r.get::<_, i64>(6)? as u64,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Source health upsert after a scan (engine calls once per adapter).
    pub fn touch_source(
        &self,
        source: &str,
        files_seen: u64,
        rows: u64,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO sources(source, enabled, last_synced_at, last_error, files_seen, rows_ingested)
             VALUES (?1,1,?2,?3,?4,?5)
             ON CONFLICT(source) DO UPDATE SET
               last_synced_at=excluded.last_synced_at, last_error=excluded.last_error,
               files_seen=excluded.files_seen,
               rows_ingested=sources.rows_ingested+excluded.rows_ingested",
            params![source, super::now_ms(), error, files_seen as i64, rows as i64],
        )?;
        Ok(())
    }

    /// Detail-page rows: newest events first. `apps`/`models` scope to the
    /// checked sets; `None` = unfiltered.
    pub fn events_page(
        &self,
        limit: i64,
        offset: i64,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<Vec<EventRow>> {
        let (w, mut p) = scope_where(None, None, apps, models);
        let (li, oi) = (p.len() + 1, p.len() + 2);
        p.push(limit.into());
        p.push(offset.into());
        let mut st = self.conn().prepare(&format!(
            "SELECT app, model, pricing_model, project, session_id, ts_start,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_read_tokens, cache_write_5m_tokens+cache_write_1h_tokens,
                    credits, cost_usd, cost_source, duration_ms, raw_ref
             FROM usage_events {w} ORDER BY ts_start DESC LIMIT ?{li} OFFSET ?{oi}"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(EventRow {
                app: r.get(0)?,
                model: r.get(1)?,
                pricing_model: r.get(2)?,
                project: r.get(3)?,
                session_id: r.get(4)?,
                ts_start: r.get(5)?,
                input_tokens: r.get::<_, i64>(6)? as u64,
                output_tokens: r.get::<_, i64>(7)? as u64,
                reasoning_tokens: r.get::<_, i64>(8)? as u64,
                cache_read_tokens: r.get::<_, i64>(9)? as u64,
                cache_write_tokens: r.get::<_, i64>(10)? as u64,
                credits: r.get(11)?,
                cost_usd: r.get(12)?,
                cost_source: r.get(13)?,
                duration_ms: r.get(14)?,
                raw_ref: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Hour-of-day histogram (UTC→local shift done by caller via offset string).
    pub fn hourly_histogram(&self, utc_offset: &str) -> Result<Vec<(u8, u64)>> {
        let mut st = self.conn().prepare(&format!(
            "SELECT CAST(strftime('%H', ts_start/1000, 'unixepoch', '{utc_offset}') AS INTEGER) h,
                    COUNT(*) FROM usage_events GROUP BY h ORDER BY h"
        ))?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, u8>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn event_count(
        &self,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Result<u64> {
        let (w, p) = scope_where(None, None, apps, models);
        Ok(self.conn().query_row(
            &format!("SELECT COUNT(*) FROM usage_events {w}"),
            rusqlite::params_from_iter(p.iter()),
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    /// Distinct tool names present in the ledger — checkbox list source.
    pub fn app_names(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn()
            .prepare("SELECT DISTINCT app FROM usage_events ORDER BY app")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Distinct display-model names (`MODEL_EXPR`), scoped by `apps` only —
    /// the model checklist cascades from the tool selection but never hides
    /// models because of the model filter itself.
    pub fn model_names(&self, apps: Option<&[String]>) -> Result<Vec<String>> {
        let (w, p) = scope_where(None, None, apps, None);
        let mut st = self.conn().prepare(&format!(
            "SELECT DISTINCT {MODEL_EXPR} FROM usage_events {w} ORDER BY 1"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

impl super::Store {
    /// Rebuild `daily_rollups` from raw events — idempotent full rebuild in
    /// one transaction (derived data, so delete+insert also self-heals an
    /// offset change), local-date aligned via `utc_offset`.
    /// Returns rows written.
    pub fn rebuild_rollups(&self, utc_offset: &str) -> Result<u64> {
        anyhow::ensure!(
            crate::viewmodel::utc_offset_ms(utc_offset).is_some(),
            "invalid utc_offset: {utc_offset}"
        );
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM daily_rollups", [])?;
        let n = tx.execute(
            &format!(
                "INSERT INTO daily_rollups
                 (date, app, provider, request_model, pricing_model, events,
                  input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                  cache_write_5m, cache_write_1h, credits, cost_usd, active_ms)
                 SELECT strftime('%Y-%m-%d', ts_start/1000, 'unixepoch', '{utc_offset}'),
                        app, COALESCE(provider_id,''), COALESCE(request_model,''),
                        COALESCE(pricing_model,''), COUNT(*),
                        COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                        COALESCE(SUM(cache_write_5m_tokens),0),
                        COALESCE(SUM(cache_write_1h_tokens),0),
                        SUM(credits), SUM(cost_usd), COALESCE(SUM(active_ms),0)
                 FROM usage_events
                 GROUP BY 1,2,3,4,5"
            ),
            [],
        )?;
        tx.commit()?;
        Ok(n as u64)
    }

    /// Same delete+insert semantics as `rebuild_rollups`, scoped to the
    /// listed day indices (`(ts + offset_ms).div_euclid(86_400_000)` — the
    /// fixed-offset frame `rebuild_rollups` aligns to). A small ingest then
    /// pays for only the days it touched instead of a full-table aggregate.
    pub fn rebuild_rollup_days(
        &self,
        days: &std::collections::BTreeSet<i64>,
        utc_offset: &str,
    ) -> Result<u64> {
        if days.is_empty() {
            return Ok(0);
        }
        let off = crate::viewmodel::utc_offset_ms(utc_offset)
            .ok_or_else(|| anyhow::anyhow!("invalid utc_offset: {utc_offset}"))?;
        const DAY: i64 = 86_400_000;
        let tx = self.conn().unchecked_transaction()?;
        // DELETE by the same date strings the INSERT will compute — the UTC
        // civil date of `d*DAY` in the epoch-ms domain is exactly what
        // `strftime(..., 'unixepoch', utc_offset)` yields for ts in that day.
        let dates: Vec<String> = days
            .iter()
            .filter_map(|&d| {
                jiff::Timestamp::from_millisecond(d * DAY)
                    .ok()
                    .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).date().to_string())
            })
            .collect();
        let marks = vec!["?"; dates.len()].join(",");
        tx.execute(
            &format!("DELETE FROM daily_rollups WHERE date IN ({marks})"),
            rusqlite::params_from_iter(dates.iter()),
        )?;
        // Index-friendly ts ranges instead of a per-row strftime filter.
        let ranges = days
            .iter()
            .map(|_| "(ts_start >= ? AND ts_start < ?)")
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut p: Vec<rusqlite::types::Value> = Vec::with_capacity(days.len() * 2);
        for &d in days {
            p.push((d * DAY - off).into());
            p.push(((d + 1) * DAY - off).into());
        }
        let n = tx.execute(
            &format!(
                "INSERT INTO daily_rollups
                 (date, app, provider, request_model, pricing_model, events,
                  input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                  cache_write_5m, cache_write_1h, credits, cost_usd, active_ms)
                 SELECT strftime('%Y-%m-%d', ts_start/1000, 'unixepoch', '{utc_offset}'),
                        app, COALESCE(provider_id,''), COALESCE(request_model,''),
                        COALESCE(pricing_model,''), COUNT(*),
                        COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                        COALESCE(SUM(cache_write_5m_tokens),0),
                        COALESCE(SUM(cache_write_1h_tokens),0),
                        SUM(credits), SUM(cost_usd), COALESCE(SUM(active_ms),0)
                 FROM usage_events WHERE {ranges}
                 GROUP BY 1,2,3,4,5"
            ),
            rusqlite::params_from_iter(p),
        )?;
        tx.commit()?;
        Ok(n as u64)
    }

    /// All detail rows in [from,to) oldest-first — the CSV export path.
    pub fn export_rows(&self, from_ms: Option<i64>, to_ms: Option<i64>) -> Result<Vec<EventRow>> {
        let (w, p) = time_where(from_ms, to_ms);
        let mut st = self.conn().prepare(&format!(
            "SELECT app, model, pricing_model, project, session_id, ts_start,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_read_tokens, cache_write_5m_tokens+cache_write_1h_tokens,
                    credits, cost_usd, cost_source, duration_ms, raw_ref
             FROM usage_events {w} ORDER BY ts_start"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(EventRow {
                app: r.get(0)?,
                model: r.get(1)?,
                pricing_model: r.get(2)?,
                project: r.get(3)?,
                session_id: r.get(4)?,
                ts_start: r.get(5)?,
                input_tokens: r.get::<_, i64>(6)? as u64,
                output_tokens: r.get::<_, i64>(7)? as u64,
                reasoning_tokens: r.get::<_, i64>(8)? as u64,
                cache_read_tokens: r.get::<_, i64>(9)? as u64,
                cache_write_tokens: r.get::<_, i64>(10)? as u64,
                credits: r.get(11)?,
                cost_usd: r.get(12)?,
                cost_source: r.get(13)?,
                duration_ms: r.get(14)?,
                raw_ref: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Drop raw events older than `before_ms`. Caller is expected to have run
    /// `rebuild_rollups` first so long-term aggregates survive the prune.
    pub fn prune_events(&self, before_ms: i64) -> Result<u64> {
        let n = self
            .conn()
            .execute("DELETE FROM usage_events WHERE ts_start < ?1", params![before_ms])?;
        Ok(n as u64)
    }

    /// Reclaim pages after a prune (blocks; run from CLI, not the UI scan path).
    pub fn vacuum(&self) -> Result<()> {
        self.conn().execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM")?;
        Ok(())
    }
}

/// Positional `?1/?2/…` params bound in order — never mix with other
/// parameter styles in one statement.
fn time_where(from_ms: Option<i64>, to_ms: Option<i64>) -> (String, Vec<rusqlite::types::Value>) {
    scope_where(from_ms, to_ms, None, None)
}

/// The display-model identity: first non-empty of model → request_model →
/// pricing_model, else "?". The model dropdown lists these names and filters
/// on the same expression, so checkbox labels, detail rows, and WHERE clauses
/// all agree on what "the model" is.
pub const MODEL_EXPR: &str =
    "COALESCE(NULLIF(model,''), NULLIF(request_model,''), NULLIF(pricing_model,''), '?')";

/// Time range + app-set + model-set filter in one WHERE. `apps`/`models`
/// `None` → no clause; `Some(list)` → `IN (?,?,…)` bound as params;
/// `Some(empty)` (user unchecked everything) → `WHERE 0`, an honest empty
/// result.
fn scope_where(
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    apps: Option<&[String]>,
    models: Option<&[String]>,
) -> (String, Vec<rusqlite::types::Value>) {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(f) = from_ms {
        params.push(f.into());
        conds.push(format!("ts_start >= ?{}", params.len()));
    }
    if let Some(t) = to_ms {
        params.push(t.into());
        conds.push(format!("ts_start < ?{}", params.len()));
    }
    if let Some(list) = apps {
        if list.is_empty() {
            conds.push("0".into());
        } else {
            let marks: Vec<String> = (0..list.len())
                .map(|i| format!("?{}", params.len() + i + 1))
                .collect();
            conds.push(format!("app IN ({})", marks.join(",")));
            params.extend(list.iter().map(|a| a.clone().into()));
        }
    }
    if let Some(list) = models {
        if list.is_empty() {
            conds.push("0".into());
        } else {
            let marks: Vec<String> = (0..list.len())
                .map(|i| format!("?{}", params.len() + i + 1))
                .collect();
            conds.push(format!("{MODEL_EXPR} IN ({})", marks.join(",")));
            params.extend(list.iter().map(|m| m.clone().into()));
        }
    }
    if conds.is_empty() {
        (String::new(), params)
    } else {
        (format!("WHERE {}", conds.join(" AND ")), params)
    }
}
