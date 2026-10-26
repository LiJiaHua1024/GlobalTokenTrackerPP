//! Trend-card math shared by every shell: the trailing moving-average
//! overlay and the drawn-bucket cap. (Moved from the WinUI shell's
//! `ui/src/trend.rs` so the macOS shell renders the identical series.)

use crate::viewmodel::TrendBucket;

/// Bars drawn at most — "All" ranges show the latest 60 buckets.
pub const MAX_BUCKETS: usize = 60;

/// Window of the moving average, in buckets' own unit (days or hours).
pub const WINDOW: i64 = 7;

/// Hourly buckets are labelled `HH:00`, daily ones `YYYY-MM-DD`.
pub fn is_hourly(buckets: &[TrendBucket]) -> bool {
    buckets.first().is_some_and(|b| b.date.len() == 5)
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Position of a bucket on its calendar axis: day number for `YYYY-MM-DD`,
/// hour of day for `HH:00`.
fn position(date: &str) -> Option<i64> {
    if date.len() == 5 {
        return date.get(..2)?.parse().ok();
    }
    let y = date.get(..4)?.parse().ok()?;
    let m = date.get(5..7)?.parse().ok()?;
    let d = date.get(8..10)?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

/// Trailing [`WINDOW`]-bucket moving average of `tokens`, aligned to
/// `buckets`. Calendar-correct: days (hours) without a bucket count as 0, and
/// the divisor is the number of calendar days (hours) from the first bucket to
/// this one, capped at the window — so the series start is not diluted.
pub fn moving_average(buckets: &[TrendBucket]) -> Vec<f64> {
    moving_average_inner(buckets)
}

/// The drawn series: the last [`MAX_BUCKETS`] buckets plus their moving
/// average, computed over everything *before* truncation so the first drawn
/// points carry their true history.
pub fn drawn(buckets: &[TrendBucket]) -> (&[TrendBucket], Vec<f64>) {
    let avg = moving_average(buckets);
    let skip = buckets.len().saturating_sub(MAX_BUCKETS);
    (&buckets[skip..], avg[skip..].to_vec())
}

fn moving_average_inner(buckets: &[TrendBucket]) -> Vec<f64> {
    let pos: Vec<i64> = buckets
        .iter()
        .map(|b| position(&b.date).unwrap_or(0))
        .collect();
    let Some(&first) = pos.first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(buckets.len());
    let (mut lo, mut sum) = (0usize, 0u64);
    for (i, b) in buckets.iter().enumerate() {
        sum += b.tokens;
        while pos[lo] < pos[i] - (WINDOW - 1) {
            sum -= buckets[lo].tokens;
            lo += 1;
        }
        let w = (pos[i] - first + 1).clamp(1, WINDOW);
        out.push(sum as f64 / w as f64);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(date: &str, tokens: u64) -> TrendBucket {
        TrendBucket {
            date: date.into(),
            events: 0,
            tokens,
            cost_usd: 0.0,
            top: Vec::new(),
        }
    }

    fn series(dates: &[&str], tokens: &[u64]) -> Vec<TrendBucket> {
        dates.iter().zip(tokens).map(|(d, t)| b(d, *t)).collect()
    }

    fn near(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-9, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn drawn_caps_at_sixty_with_full_history_average() {
        let dates: Vec<String> = (0..70)
            .map(|i| {
                let d = jiff::civil::date(2026, 1, 1).saturating_add(jiff::Span::new().days(i));
                d.to_string()
            })
            .collect();
        let refs: Vec<&str> = dates.iter().map(String::as_str).collect();
        let s = series(&refs, &[7; 70]);
        let (bars, avg) = drawn(&s);
        assert_eq!(bars.len(), MAX_BUCKETS);
        assert_eq!(bars[0].date, "2026-01-11");
        // History before the cut keeps the first drawn point at a full week.
        near(&avg, &[7.0; MAX_BUCKETS]);
        let short = series(&["2026-03-01"], &[5]);
        assert_eq!(drawn(&short).0.len(), 1);
    }

    #[test]
    fn empty_series_is_empty() {
        assert_eq!(moving_average(&[]), Vec::<f64>::new());
        assert!(!is_hourly(&[]));
    }

    #[test]
    fn series_start_divides_by_the_days_so_far() {
        let s = series(&["2026-03-01", "2026-03-02", "2026-03-03"], &[10, 20, 30]);
        near(&moving_average(&s), &[10.0, 15.0, 20.0]);
    }

    #[test]
    fn contiguous_days_use_a_full_week_then_slide() {
        let dates: Vec<String> = (1..=9).map(|d| format!("2026-03-{d:02}")).collect();
        let refs: Vec<&str> = dates.iter().map(String::as_str).collect();
        let s = series(&refs, &[7; 9]);
        near(&moving_average(&s), &[7.0; 9]);
        // A spike leaves the window after 7 days.
        let s = series(&refs, &[70, 0, 0, 0, 0, 0, 0, 0, 0]);
        let ma = moving_average(&s);
        assert!((ma[6] - 10.0).abs() < 1e-9);
        assert!((ma[7] - 0.0).abs() < 1e-9);
    }

    #[test]
    fn missing_days_count_as_zero_not_as_adjacent() {
        // Bars for Mar 1 and Mar 5 only: day 5's window holds both, w = 5.
        let s = series(&["2026-03-01", "2026-03-05"], &[50, 100]);
        near(&moving_average(&s), &[50.0, 30.0]);
        // Mar 10 is > 6 days after Mar 1 → Mar 1 has dropped out; w = 7.
        let s = series(&["2026-03-01", "2026-03-10"], &[50, 70]);
        near(&moving_average(&s), &[50.0, 10.0]);
    }

    #[test]
    fn month_and_year_boundaries_are_calendar_correct() {
        let s = series(&["2025-12-30", "2026-01-01", "2026-01-02"], &[30, 60, 90]);
        // Dec 30 → Jan 2 is 4 calendar days.
        near(&moving_average(&s), &[30.0, 30.0, 45.0]);
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28),
            2
        );
    }

    #[test]
    fn hourly_buckets_use_a_seven_hour_window() {
        let s = series(&["09:00", "10:00", "14:00", "17:00"], &[70, 70, 140, 7]);
        assert!(is_hourly(&s));
        // 17:00: window 11..=17 holds only 14:00 and 17:00; w = 7 (09→17 is 9 h).
        near(
            &moving_average(&s),
            &[70.0, 70.0, 140.0 / 6.0 + 140.0 / 6.0, (140.0 + 7.0) / 7.0],
        );
    }
}
