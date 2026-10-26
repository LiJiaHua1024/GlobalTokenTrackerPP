//! Historical speed ranking using weighted token/time sums.
use super::query::{scope_where, speed_exprs, speed_from_row};
use super::{SpeedSums, Store};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SpeedQuery {
    pub range_key: String,
    pub custom_start_ms: Option<i64>,
    pub custom_end_ms: Option<i64>,
    pub apps: Option<Vec<String>>,
    pub models: Option<Vec<String>>,
    pub group_by: String,
    pub metric: String,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Serialize)]
pub struct SpeedRow {
    pub name: String,
    pub events: u64,
    pub speed: SpeedSums,
    pub output_tps: Option<f64>,
    pub total_tps: Option<f64>,
    pub input_tps: Option<f64>,
    pub prefill_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub avg_ttft_ms: Option<f64>,
}

impl SpeedRow {
    fn value(&self, metric: &str) -> Option<f64> {
        match metric {
            "total" => self.total_tps,
            "input" => self.input_tps,
            "prefill" => self.prefill_tps,
            "decode" => self.decode_tps,
            _ => self.output_tps,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SpeedRanking {
    pub rows: Vec<SpeedRow>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

impl Store {
    pub fn speed_ranking(&self, q: &SpeedQuery) -> Result<SpeedRanking> {
        anyhow::ensure!(
            ["", "model", "app"].contains(&q.group_by.as_str()),
            "invalid speed group"
        );
        anyhow::ensure!(
            ["", "output", "total", "input", "prefill", "decode"].contains(&q.metric.as_str()),
            "invalid speed metric"
        );
        let range =
            crate::viewmodel::Range::from_config(&q.range_key, q.custom_start_ms, q.custom_end_ms);
        let (w, p) = scope_where(
            range.start_ms(),
            range.end_ms(),
            q.apps.as_deref(),
            q.models.as_deref(),
        );
        let group = if q.group_by == "app" {
            "app"
        } else {
            super::MODEL_EXPR
        };
        let mut statement = self.conn().prepare(&format!(
            "SELECT {group} AS name, COUNT(*), {} FROM usage_events {w} GROUP BY name",
            speed_exprs()
        ))?;
        let mut rows = statement
            .query_map(rusqlite::params_from_iter(p.iter()), |r| {
                let speed = speed_from_row(r, 2)?;
                Ok(SpeedRow {
                    name: r.get(0)?,
                    events: r.get::<_, i64>(1)? as u64,
                    output_tps: speed.output_tps(),
                    total_tps: (speed.speed_ms > 0).then(|| {
                        (speed.speed_in + speed.speed_out) as f64 / speed.speed_ms as f64 * 1000.0
                    }),
                    input_tps: speed.input_tps(),
                    prefill_tps: speed.prefill_tps(),
                    decode_tps: speed.decode_tps(),
                    avg_ttft_ms: speed.avg_ttft_ms(),
                    speed,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.sort_by(|a, b| {
            b.value(&q.metric)
                .partial_cmp(&a.value(&q.metric))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.name.cmp(&b.name))
        });
        let total = rows.len();
        let size = if q.page_size == 0 {
            25
        } else {
            q.page_size.clamp(1, 200)
        };
        let page = q.page.min(total.saturating_sub(1) / size);
        let rows = rows.into_iter().skip(page * size).take(size).collect();
        Ok(SpeedRanking {
            rows,
            total,
            page,
            page_size: size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranking_weights_times_filters_pages_and_keeps_untimed_unknown() {
        let s = Store::open_memory().unwrap();
        for (key, app, model, out, ms) in [
            ("1", "a", "fast", 100, 1000),
            ("2", "a", "fast", 100, 9000),
            ("3", "b", "slow", 10, 1000),
            ("4", "c", "unknown", 500, 0),
        ] {
            s.upsert_event(&crate::UsageEvent {
                dedup_key: key.into(),
                app: app.into(),
                model: Some(model.into()),
                input_tokens: 20,
                output_tokens: out,
                duration_ms: (ms > 0).then_some(ms),
                ttft_ms: (ms > 0).then_some(200),
                ..Default::default()
            })
            .unwrap();
        }
        s.upsert_event(&crate::UsageEvent {
            dedup_key: "unclassified".into(),
            app: "a".into(),
            model: Some("fast".into()),
            unclassified_tokens: 1000,
            duration_ms: Some(100000),
            ..Default::default()
        })
        .unwrap();
        let mut q = SpeedQuery {
            range_key: "all".into(),
            page_size: 1,
            ..Default::default()
        };
        let r = s.speed_ranking(&q).unwrap();
        assert_eq!(r.total, 3);
        assert_eq!(r.rows[0].name, "fast");
        assert_eq!(r.rows[0].output_tps, Some(20.0));
        q.page = 99;
        let r = s.speed_ranking(&q).unwrap();
        assert_eq!(r.page, 2);
        assert!(r.rows[0].output_tps.is_none());
        q.group_by = "app".into();
        q.apps = Some(vec!["b".into()]);
        let r = s.speed_ranking(&q).unwrap();
        assert_eq!(r.total, 1);
        assert_eq!(r.rows[0].name, "b");
        q.apps = Some(vec![]);
        assert_eq!(s.speed_ranking(&q).unwrap().total, 0);
    }
}
