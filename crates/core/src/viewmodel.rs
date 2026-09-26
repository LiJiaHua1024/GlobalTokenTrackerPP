//! ViewModels — plain, serializable structs the UI shells render verbatim.
//! All SQL/formatting lives here so shells stay dumb (Mac port = same VMs).

use crate::store::{AppSummary, EventRow, QuotaRow, Store, Totals};
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct OverviewVm {
    pub today: Totals,
    pub week: Totals,
    pub month: Totals,
    pub all: Totals,
    pub by_app: Vec<AppSummary>,
    /// (YYYY-MM-DD, total_tokens, cost_usd) for the last `days` local days.
    pub daily: Vec<(String, u64, f64)>,
    pub quotas: Vec<QuotaRow>,
    pub unpriced: Vec<(String, u64)>,
    /// Local UTC offset "+HH:MM" for display.
    pub tz_offset: String,
}

#[derive(Debug, Clone)]
pub struct DetailVm {
    pub rows: Vec<EventRow>,
    pub total_events: u64,
}

pub fn local_utc_offset() -> String {
    let secs = jiff::Zoned::now().offset().seconds();
    let sign = if secs < 0 { '-' } else { '+' };
    let a = secs.unsigned_abs();
    format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
}

fn day_start_ms(days_ago: i64) -> i64 {
    let now = jiff::Zoned::now();
    now.start_of_day()
        .and_then(|d| d.checked_sub(jiff::SignedDuration::from_hours(days_ago * 24)))
        .map(|d| d.timestamp().as_millisecond())
        .unwrap_or_default()
}

impl Store {
    pub fn overview(&self) -> Result<OverviewVm> {
        let t0 = day_start_ms(0);
        let w0 = day_start_ms(6);
        let m0 = day_start_ms(29);
        let tz = local_utc_offset();
        let daily_raw = self.daily(Some(m0), None, &tz)?;
        // Fold apps into one series per day for the trend strip.
        let mut series: std::collections::BTreeMap<String, (u64, f64)> =
            std::collections::BTreeMap::new();
        for d in daily_raw {
            let e = series.entry(d.date).or_default();
            e.0 += d.output_tokens + d.cache_read_tokens + d.input_tokens;
            e.1 += d.cost_usd;
        }
        Ok(OverviewVm {
            today: self.totals(Some(t0), None)?,
            week: self.totals(Some(w0), None)?,
            month: self.totals(Some(m0), None)?,
            all: self.totals(None, None)?,
            by_app: self.by_app(Some(w0), None)?,
            daily: series.into_iter().map(|(d, (t, c))| (d, t, c)).collect(),
            quotas: self.latest_quotas()?,
            unpriced: self.unpriced_models()?,
            tz_offset: tz,
        })
    }

    pub fn detail(&self, page: i64, page_size: i64) -> Result<DetailVm> {
        Ok(DetailVm {
            rows: self.events_page(page_size, page * page_size)?,
            total_events: self.event_count()?,
        })
    }
}

/// Human formatting helpers shared by CLI and UI.
pub mod fmt {
    pub fn tokens(n: u64) -> String {
        if n >= 1_000_000_000 {
            format!("{:.2}B", n as f64 / 1e9)
        } else if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1e6)
        } else if n >= 10_000 {
            format!("{:.0}K", n as f64 / 1e3)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1e3)
        } else {
            n.to_string()
        }
    }

    pub fn usd(v: f64) -> String {
        if v >= 100.0 {
            format!("${:.0}", v)
        } else if v >= 1.0 {
            format!("${:.2}", v)
        } else {
            format!("${:.4}", v)
        }
    }

    pub fn tokens_total(t: &crate::store::Totals) -> u64 {
        t.input_tokens
            + t.output_tokens
            + t.cache_read_tokens
            + t.cache_write_tokens
    }

    /// epoch ms → "MM-DD HH:MM" local.
    pub fn ts_short(ms: Option<i64>) -> String {
        let Some(ms) = ms else { return "—".into() };
        let Ok(t) = jiff::Timestamp::from_millisecond(ms) else {
            return "—".into();
        };
        t.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%m-%d %H:%M")
            .to_string()
    }

    /// epoch ms → "YYYY-MM-DD HH:MM:SS" local.
    pub fn ts_long(ms: Option<i64>) -> String {
        let Some(ms) = ms else { return "—".into() };
        let Ok(t) = jiff::Timestamp::from_millisecond(ms) else {
            return "—".into();
        };
        t.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    /// epoch ms → human "3h12m" countdown, "过期" when the instant has passed.
    pub fn until(ms: Option<i64>) -> String {
        let Some(ms) = ms else { return "—".into() };
        let diff = ms - crate::store::now_ms();
        if diff < 0 {
            return "已过期".into();
        }
        let m = diff / 60_000;
        if m >= 1440 {
            format!("{}d{}h", m / 1440, (m % 1440) / 60)
        } else if m >= 60 {
            format!("{}h{}m", m / 60, m % 60)
        } else {
            format!("{m}m")
        }
    }

    pub fn duration(ms: Option<i64>) -> String {
        let Some(ms) = ms else { return "—".into() };
        if ms >= 60_000 {
            format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
        } else if ms >= 1000 {
            format!("{:.1}s", ms as f64 / 1000.0)
        } else {
            format!("{ms}ms")
        }
    }
}
