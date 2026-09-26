//! ViewModels — plain, serializable structs the UI shells render verbatim.
//! All SQL/formatting lives here so shells stay dumb (Mac port = same VMs).

use crate::store::{AppSummary, EventRow, QuotaRow, Store, Totals};
use anyhow::Result;

/// Statistics window selected on the overview page. Persisted as `key` in
/// ui.json so the choice survives restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Range {
    Today,
    #[default]
    Week,
    Month,
    All,
}

impl Range {
    pub const LIST: [Range; 4] = [Self::Today, Self::Week, Self::Month, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今日",
            Self::Week => "近 7 天",
            Self::Month => "近 30 天",
            Self::All => "全部",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "week",
            Self::Month => "month",
            Self::All => "all",
        }
    }

    pub fn from_key(s: &str) -> Self {
        match s {
            "today" => Self::Today,
            "month" => Self::Month,
            "all" => Self::All,
            _ => Self::Week,
        }
    }

    pub fn from_label(s: &str) -> Self {
        Self::LIST
            .iter()
            .find(|r| r.label() == s)
            .copied()
            .unwrap_or_default()
    }

    /// Window start (epoch ms); `None` = unbounded ("all").
    fn start_ms(self) -> Option<i64> {
        match self {
            Self::Today => Some(day_start_ms(0)),
            Self::Week => Some(day_start_ms(6)),
            Self::Month => Some(day_start_ms(29)),
            Self::All => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OverviewVm {
    /// Always today — the tray tooltip and badges stay day-scoped regardless
    /// of the selected range.
    pub today: Totals,
    /// Aggregates for the selected `range`.
    pub span: Totals,
    pub all: Totals,
    pub range: Range,
    pub by_app: Vec<AppSummary>,
    /// Trend series for the selected range: ("YYYY-MM-DD"|"HH:00", tokens, cost).
    /// Today → hourly buckets; other ranges → per local day.
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
    pub fn overview(&self, range: Range) -> Result<OverviewVm> {
        let t0 = day_start_ms(0);
        let start = range.start_ms();
        let tz = local_utc_offset();
        // Fold apps into one series per bucket for the trend strip.
        let raw = if range == Range::Today {
            self.hourly(t0, &tz)?
        } else {
            self.daily(start, None, &tz)?
        };
        let mut series: std::collections::BTreeMap<String, (u64, f64)> =
            std::collections::BTreeMap::new();
        for d in raw {
            let e = series.entry(d.date).or_default();
            e.0 += d.output_tokens + d.cache_read_tokens + d.input_tokens;
            e.1 += d.cost_usd;
        }
        let span = self.totals(start, None)?;
        Ok(OverviewVm {
            today: self.totals(Some(t0), None)?,
            span,
            all: self.totals(None, None)?,
            range,
            by_app: self.by_app(start, None)?,
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
    /// Exact token count with thousands separators — never abbreviated,
    /// users reconcile these numbers against vendor dashboards.
    pub fn tokens_exact(n: u64) -> String {
        let s = n.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, c) in s.bytes().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(char::from(c));
        }
        out
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

#[cfg(test)]
mod tests {
    use super::fmt;

    #[test]
    fn tokens_exact_groups() {
        assert_eq!(fmt::tokens_exact(0), "0");
        assert_eq!(fmt::tokens_exact(999), "999");
        assert_eq!(fmt::tokens_exact(1_000), "1,000");
        assert_eq!(fmt::tokens_exact(1_730_848_235), "1,730,848,235");
        assert_eq!(fmt::tokens_exact(13_101_054_884), "13,101,054,884");
    }

    #[test]
    fn range_roundtrip() {
        for r in super::Range::LIST {
            assert_eq!(super::Range::from_key(r.key()), r);
            assert_eq!(super::Range::from_label(r.label()), r);
        }
    }
}
