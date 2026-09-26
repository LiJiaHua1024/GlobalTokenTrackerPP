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

#[derive(Debug, Clone)]
pub struct DailyRow {
    pub date: String, // YYYY-MM-DD in `tz`
    pub app: String,
    pub events: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub credits: f64,
}

impl super::Store {
    /// Totals over `[from_ms, to_ms)`; `None,None` = all time.
    pub fn totals(&self, from_ms: Option<i64>, to_ms: Option<i64>) -> Result<Totals> {
        let (w, p) = time_where(from_ms, to_ms);
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

    pub fn by_app(&self, from_ms: Option<i64>, to_ms: Option<i64>) -> Result<Vec<AppSummary>> {
        let (w, p) = time_where(from_ms, to_ms);
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

    /// Per-day aggregation. `utc_offset` is a fixed `+HH:MM`/`-HH:MM` string
    /// (validated) folded into the strftime modifier — local-midnight aligned.
    pub fn daily(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        utc_offset: &str,
    ) -> Result<Vec<DailyRow>> {
        let b = utc_offset.as_bytes();
        anyhow::ensure!(
            b.len() == 6
                && matches!(b[0], b'+' | b'-')
                && b[3] == b':'
                && [1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit()),
            "invalid utc_offset: {utc_offset}"
        );
        let (w, p) = time_where(from_ms, to_ms);
        let mut st = self.conn().prepare(&format!(
            "SELECT strftime('%Y-%m-%d', ts_start/1000, 'unixepoch', '{utc_offset}') AS d, app,
                    COUNT(*), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cost_usd),0), COALESCE(SUM(credits),0)
             FROM usage_events {w}
             GROUP BY d, app ORDER BY d"
        ))?;
        let rows = st.query_map(rusqlite::params_from_iter(p.iter()), |r| {
            Ok(DailyRow {
                date: r.get(0)?,
                app: r.get(1)?,
                events: r.get::<_, i64>(2)? as u64,
                output_tokens: r.get::<_, i64>(3)? as u64,
                cache_read_tokens: r.get::<_, i64>(4)? as u64,
                cost_usd: r.get(5)?,
                credits: r.get(6)?,
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

    pub fn event_count(&self) -> Result<u64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_events", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }
}

/// Positional `?1/?2` params bound in order — never mix with other
/// parameter styles in one statement.
fn time_where(from_ms: Option<i64>, to_ms: Option<i64>) -> (String, Vec<rusqlite::types::Value>) {
    match (from_ms, to_ms) {
        (Some(f), Some(t)) => (
            "WHERE ts_start >= ?1 AND ts_start < ?2".into(),
            vec![f.into(), t.into()],
        ),
        (Some(f), None) => ("WHERE ts_start >= ?1".into(), vec![f.into()]),
        (None, Some(t)) => ("WHERE ts_start < ?1".into(), vec![t.into()]),
        (None, None) => (String::new(), vec![]),
    }
}
