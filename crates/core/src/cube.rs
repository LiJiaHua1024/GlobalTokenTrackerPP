//! Aggregation cube — the overview's data source.
//!
//! The overview used to re-aggregate the raw `usage_events` table (wide rows,
//! `strftime` per row) with ten separate queries on every range/filter change
//! — 100–270ms on a 63k-event ledger. Here the table is scanned **once** into a
//! few thousand `(local day × tool × model)` groups held in memory; after that
//! any range or filter combination is an in-memory fold (well under a
//! millisecond), and ingestion only recomputes the days it touched.
//!
//! Semantics are pinned to the SQL path (`Store::overview`) by the equivalence
//! tests below: the same fixed-offset day frame, `MODEL_EXPR` model identity,
//! `input+output+cache_read` headline tokens, and NULL-timestamp rows counting
//! in unbounded totals only.

use crate::store::{AppSummary, MODEL_EXPR, ShareRow, Store, Totals};
use crate::viewmodel::{
    HeatDay, Range, TrendBucket, day_start_ms, local_utc_offset, utc_offset_ms,
};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;
/// Day index of rows without `ts_start` — counted by unbounded totals only.
const NO_DAY: i32 = i32::MIN;
/// More touched days than this → cheaper (and safer) to rebuild everything.
const MAX_INCREMENTAL_DAYS: usize = 60;

#[derive(Clone, Debug)]
struct Group {
    /// Local day index (days since 1970-01-01 in the cube's fixed-offset frame).
    day: i32,
    app: u16,
    model: u16,
    events: u64,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    credits: f64,
    cost: f64,
    active_ms: u64,
    duration_ms: u64,
}

impl Group {
    /// Headline token count of the trend/pie convention: input+output+cache_read.
    fn headline(&self) -> u64 {
        self.input + self.output + self.cache_read
    }
}

/// Today's hour × tool × model groups — the `Today` range's trend bars.
#[derive(Clone, Debug)]
struct HourGroup {
    hour: u8,
    app: u16,
    model: u16,
    events: u64,
    tokens: u64,
    cost: f64,
}

#[derive(Clone, Debug)]
pub struct Cube {
    /// Fixed local UTC offset (ms) the day frame was built with.
    off_ms: i64,
    apps: Vec<String>,
    app_ix: HashMap<String, u16>,
    models: Vec<String>,
    model_ix: HashMap<String, u16>,
    groups: Vec<Group>,
    /// Day index the `hours` belong to — a midnight rollover makes them stale.
    today_day: i32,
    hours: Vec<HourGroup>,
}

/// The range/filter-dependent part of an overview.
#[derive(Debug, Clone)]
pub struct OverviewParts {
    pub today: Totals,
    pub span: Totals,
    pub all: Totals,
    pub by_app: Vec<AppSummary>,
    pub by_model: Vec<ShareRow>,
    pub daily: Vec<TrendBucket>,
    pub apps: Vec<String>,
    pub models: Vec<String>,
}

/// One row of the group query.
struct Raw {
    day: Option<i64>,
    app: String,
    model: String,
    events: u64,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    credits: f64,
    cost: f64,
    active_ms: u64,
    duration_ms: u64,
}

impl Cube {
    /// Scan the whole ledger once.
    pub fn build(store: &Store) -> Result<Self> {
        let off_ms = current_offset_ms();
        let mut cube = Self::empty(off_ms);
        let raws = query_groups(store, off_ms, None)?;
        cube.absorb(raws);
        cube.reload_hours(store)?;
        cube.sort();
        Ok(cube)
    }

    /// A copy with only the given local days recomputed — what an ingestion
    /// pass that touched those days needs. Falls back to a full build when
    /// the timezone offset changed or too many days are dirty.
    pub fn refreshed(&self, store: &Store, days: &BTreeSet<i64>) -> Result<Self> {
        let off_ms = current_offset_ms();
        if off_ms != self.off_ms || days.len() > MAX_INCREMENTAL_DAYS {
            return Self::build(store);
        }
        let mut next = self.clone();
        // Today's hours (and yesterday/today's groups) are always recomputed:
        // writers this process doesn't hear about (the OTLP receiver) only
        // ever add recent events.
        let now_day = day_index(off_ms, now_ms());
        let mut dirty: BTreeSet<i64> = days.clone();
        dirty.insert(i64::from(now_day));
        dirty.insert(i64::from(now_day) - 1);
        next.groups
            .retain(|g| g.day == NO_DAY || !dirty.contains(&i64::from(g.day)));
        let raws = query_groups(store, off_ms, Some(&dirty))?;
        next.absorb(raws);
        next.reload_hours(store)?;
        next.sort();
        Ok(next)
    }

    fn empty(off_ms: i64) -> Self {
        Self {
            off_ms,
            apps: Vec::new(),
            app_ix: HashMap::new(),
            models: Vec::new(),
            model_ix: HashMap::new(),
            groups: Vec::new(),
            today_day: day_index(off_ms, now_ms()),
            hours: Vec::new(),
        }
    }

    fn intern_app(&mut self, name: &str) -> u16 {
        if let Some(&i) = self.app_ix.get(name) {
            return i;
        }
        let i = self.apps.len() as u16;
        self.apps.push(name.to_string());
        self.app_ix.insert(name.to_string(), i);
        i
    }

    fn intern_model(&mut self, name: &str) -> u16 {
        if let Some(&i) = self.model_ix.get(name) {
            return i;
        }
        let i = self.models.len() as u16;
        self.models.push(name.to_string());
        self.model_ix.insert(name.to_string(), i);
        i
    }

    fn absorb(&mut self, raws: Vec<Raw>) {
        for r in raws {
            let app = self.intern_app(&r.app);
            let model = self.intern_model(&r.model);
            self.groups.push(Group {
                day: r.day.map_or(NO_DAY, |d| d as i32),
                app,
                model,
                events: r.events,
                input: r.input,
                output: r.output,
                reasoning: r.reasoning,
                cache_read: r.cache_read,
                cache_write: r.cache_write,
                credits: r.credits,
                cost: r.cost,
                active_ms: r.active_ms,
                duration_ms: r.duration_ms,
            });
        }
    }

    fn sort(&mut self) {
        self.groups.sort_by_key(|g| (g.day, g.app, g.model));
    }

    fn reload_hours(&mut self, store: &Store) -> Result<()> {
        self.today_day = day_index(self.off_ms, now_ms());
        let today_start = i64::from(self.today_day) * DAY_MS - self.off_ms;
        let mut st = store.conn().prepare(&format!(
            "SELECT ((ts_start + ?1) % {DAY_MS}) / {HOUR_MS} AS h, app, {MODEL_EXPR},
                    COUNT(*),
                    COALESCE(SUM(input_tokens+output_tokens+cache_read_tokens),0),
                    COALESCE(SUM(cost_usd),0)
             FROM usage_events WHERE ts_start >= ?2 GROUP BY h, app, 3"
        ))?;
        let rows = st.query_map([self.off_ms, today_start], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, f64>(5)?,
            ))
        })?;
        let mut hours = Vec::new();
        for row in rows {
            let (h, app, model, events, tokens, cost) = row?;
            hours.push((h, app, model, events, tokens, cost));
        }
        self.hours = hours
            .into_iter()
            .map(|(h, app, model, events, tokens, cost)| HourGroup {
                hour: h.clamp(0, 23) as u8,
                app: self.intern_app(&app),
                model: self.intern_model(&model),
                events: events as u64,
                tokens: tokens as u64,
                cost,
            })
            .collect();
        Ok(())
    }

    // ---------------------------------------------------------------- reads

    /// Distinct tool names in the ledger, sorted.
    pub fn app_names(&self) -> Vec<String> {
        let live: BTreeSet<u16> = self.groups.iter().map(|g| g.app).collect();
        let mut v: Vec<String> = live
            .iter()
            .map(|&i| self.apps[usize::from(i)].clone())
            .collect();
        v.sort();
        v
    }

    /// Distinct display-model names, scoped by `apps` only — the model
    /// checklist cascades from the tool selection but is never narrowed by
    /// the model filter itself.
    pub fn model_names(&self, apps: Option<&[String]>) -> Vec<String> {
        let app_ok = self.app_mask(apps);
        let live: BTreeSet<u16> = self
            .groups
            .iter()
            .filter(|g| app_ok[usize::from(g.app)])
            .map(|g| g.model)
            .collect();
        let mut v: Vec<String> = live
            .iter()
            .map(|&i| self.models[usize::from(i)].clone())
            .collect();
        v.sort();
        v
    }

    /// Event count under the tool/model filters (all time, NULL-ts included).
    pub fn event_count(&self, apps: Option<&[String]>, models: Option<&[String]>) -> u64 {
        let (app_ok, model_ok) = (self.app_mask(apps), self.model_mask(models));
        self.groups
            .iter()
            .filter(|g| app_ok[usize::from(g.app)] && model_ok[usize::from(g.model)])
            .map(|g| g.events)
            .sum()
    }

    /// The activity heatmap's days: from the Monday 52 weeks before the
    /// current week through today, one entry per local day (zero-filled),
    /// under the tool/model filters. Independent of the selected range; rows
    /// without a timestamp and future-dated rows are outside the window.
    pub fn heat(&self, apps: Option<&[String]>, models: Option<&[String]>) -> Vec<HeatDay> {
        let (start, today) = heat_bounds(self.off_ms, now_ms());
        let mut out = blank_heat(start, today);
        let (app_ok, model_ok) = (self.app_mask(apps), self.model_mask(models));
        for g in &self.groups {
            if g.day == NO_DAY
                || g.day < start
                || g.day > today
                || !app_ok[usize::from(g.app)]
                || !model_ok[usize::from(g.model)]
            {
                continue;
            }
            let h = &mut out[(g.day - start) as usize];
            h.tokens += g.headline();
            h.cost_usd += g.cost;
            h.events += g.events;
            h.duration_ms += g.duration_ms;
        }
        out
    }

    /// Everything on the overview that depends on the range/filters. `None`
    /// when the cube cannot answer exactly (a range edge that isn't a day
    /// boundary in the cube's frame, or `Today`'s hours are from yesterday) —
    /// the caller then falls back to the SQL path.
    pub fn overview_parts(
        &self,
        range: Range,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) -> Option<OverviewParts> {
        let lo = self.bound(range.start_ms())?;
        let hi = self.bound(range.end_ms())?;
        let today_idx = self.aligned_day(day_start_ms(0))?;
        if today_idx != self.today_day {
            return None;
        }
        let (app_ok, model_ok) = (self.app_mask(apps), self.model_mask(models));
        let keep = |g: &&Group| app_ok[usize::from(g.app)] && model_ok[usize::from(g.model)];
        let bounded = lo.is_some() || hi.is_some();
        let in_span = |g: &Group| {
            if g.day == NO_DAY {
                return !bounded;
            }
            lo.is_none_or(|l| g.day >= l) && hi.is_none_or(|h| g.day < h)
        };

        let mut today = Totals::default();
        let mut span = Totals::default();
        let mut all = Totals::default();
        let mut by_app: BTreeMap<u16, AppSummary> = BTreeMap::new();
        let mut by_model: BTreeMap<u16, ShareRow> = BTreeMap::new();
        for g in self.groups.iter().filter(keep) {
            add(&mut all, g);
            if g.day != NO_DAY && g.day >= self.today_day {
                add(&mut today, g);
            }
            if !in_span(g) {
                continue;
            }
            add(&mut span, g);
            let a = by_app.entry(g.app).or_insert_with(|| AppSummary {
                app: self.apps[usize::from(g.app)].clone(),
                events: 0,
                input_tokens: 0,
                output_tokens: 0,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                credits: 0.0,
                cost_usd: 0.0,
            });
            a.events += g.events;
            a.input_tokens += g.input;
            a.output_tokens += g.output;
            a.reasoning_tokens += g.reasoning;
            a.cache_read_tokens += g.cache_read;
            a.cache_write_tokens += g.cache_write;
            a.credits += g.credits;
            a.cost_usd += g.cost;
            let m = by_model.entry(g.model).or_insert_with(|| ShareRow {
                name: self.models[usize::from(g.model)].clone(),
                events: 0,
                tokens: 0,
                cost_usd: 0.0,
            });
            m.events += g.events;
            m.tokens += g.headline();
            m.cost_usd += g.cost;
        }
        let mut by_app: Vec<AppSummary> = by_app.into_values().collect();
        by_app.sort_by(|a, b| cost_desc(a.cost_usd, b.cost_usd).then_with(|| a.app.cmp(&b.app)));
        let mut by_model: Vec<ShareRow> = by_model.into_values().collect();
        by_model
            .sort_by(|a, b| cost_desc(a.cost_usd, b.cost_usd).then_with(|| a.name.cmp(&b.name)));

        let daily = if range == Range::Today {
            self.hourly_trend(&app_ok, &model_ok)
        } else {
            self.daily_trend(
                self.groups
                    .iter()
                    .filter(keep)
                    .filter(|g| g.day != NO_DAY && in_span(g)),
            )
        };
        Some(OverviewParts {
            today,
            span,
            all,
            by_app,
            by_model,
            daily,
            apps: self.app_names(),
            models: self.model_names(apps),
        })
    }

    fn daily_trend<'a>(&self, groups: impl Iterator<Item = &'a Group>) -> Vec<TrendBucket> {
        let mut out: Vec<TrendBucket> = Vec::new();
        let mut cur_day = NO_DAY;
        let mut models: BTreeMap<u16, u64> = BTreeMap::new();
        let finish = |out: &mut Vec<TrendBucket>, models: &mut BTreeMap<u16, u64>| {
            if let Some(b) = out.last_mut() {
                b.top = top3(models, &self.models);
            }
            models.clear();
        };
        for g in groups {
            if g.day != cur_day {
                finish(&mut out, &mut models);
                cur_day = g.day;
                out.push(TrendBucket {
                    date: civil_date(i64::from(g.day)),
                    events: 0,
                    tokens: 0,
                    cost_usd: 0.0,
                    top: Vec::new(),
                });
            }
            let b = out.last_mut().expect("bucket pushed above");
            b.events += g.events;
            b.tokens += g.headline();
            b.cost_usd += g.cost;
            *models.entry(g.model).or_default() += g.headline();
        }
        finish(&mut out, &mut models);
        out
    }

    fn hourly_trend(&self, app_ok: &[bool], model_ok: &[bool]) -> Vec<TrendBucket> {
        let mut by_hour: BTreeMap<u8, (TrendBucket, BTreeMap<u16, u64>)> = BTreeMap::new();
        for h in &self.hours {
            if !app_ok[usize::from(h.app)] || !model_ok[usize::from(h.model)] {
                continue;
            }
            let (b, models) = by_hour.entry(h.hour).or_insert_with(|| {
                (
                    TrendBucket {
                        date: format!("{:02}:00", h.hour),
                        events: 0,
                        tokens: 0,
                        cost_usd: 0.0,
                        top: Vec::new(),
                    },
                    BTreeMap::new(),
                )
            });
            b.events += h.events;
            b.tokens += h.tokens;
            b.cost_usd += h.cost;
            *models.entry(h.model).or_default() += h.tokens;
        }
        by_hour
            .into_values()
            .map(|(mut b, models)| {
                b.top = top3(&models, &self.models);
                b
            })
            .collect()
    }

    // -------------------------------------------------------------- helpers

    /// `None` filter → everyone; `Some(list)` → exactly those names
    /// (`Some(empty)` = nobody, an honest empty view).
    fn app_mask(&self, apps: Option<&[String]>) -> Vec<bool> {
        mask(&self.apps, apps)
    }

    fn model_mask(&self, models: Option<&[String]>) -> Vec<bool> {
        mask(&self.models, models)
    }

    /// A range edge as a day index; the outer `None` = not a day boundary in
    /// this cube's frame, the inner `None` = unbounded.
    fn bound(&self, ms: Option<i64>) -> Option<Option<i32>> {
        match ms {
            None => Some(None),
            Some(ms) => self.aligned_day(ms).map(Some),
        }
    }

    fn aligned_day(&self, ms: i64) -> Option<i32> {
        let v = ms + self.off_ms;
        (v.rem_euclid(DAY_MS) == 0).then(|| v.div_euclid(DAY_MS) as i32)
    }

    /// A local midnight has passed since the cube's "today" was loaded — its
    /// hourly bars and today totals are yesterday's; refresh before trusting it.
    pub fn is_day_stale(&self) -> bool {
        day_index(current_offset_ms(), now_ms()) != self.today_day
    }

    /// Approximate resident size — for the memory report.
    pub fn approx_bytes(&self) -> usize {
        self.groups.capacity() * std::mem::size_of::<Group>()
            + self.hours.capacity() * std::mem::size_of::<HourGroup>()
            + self
                .apps
                .iter()
                .chain(&self.models)
                .map(|s| s.capacity() + 24)
                .sum::<usize>()
                * 2
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }
}

fn mask(names: &[String], filter: Option<&[String]>) -> Vec<bool> {
    match filter {
        None => vec![true; names.len()],
        Some(list) => names.iter().map(|n| list.contains(n)).collect(),
    }
}

fn add(t: &mut Totals, g: &Group) {
    t.events += g.events;
    t.input_tokens += g.input;
    t.output_tokens += g.output;
    t.reasoning_tokens += g.reasoning;
    t.cache_read_tokens += g.cache_read;
    t.cache_write_tokens += g.cache_write;
    t.credits += g.credits;
    t.cost_usd += g.cost;
    t.active_ms += g.active_ms;
}

fn cost_desc(a: f64, b: f64) -> std::cmp::Ordering {
    b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
}

/// Top models by tokens (ties: name ascending, like the SQL group order).
/// This fork's `Store::overview` keeps up to 20 per bucket for the Flutter
/// trend tooltip, so the cube must match that, not upstream's top-3.
fn top3(models: &BTreeMap<u16, u64>, names: &[String]) -> Vec<(String, u64)> {
    const TOP_N: usize = 20;
    let mut v: Vec<(String, u64)> = models
        .iter()
        .map(|(&i, &t)| (names[usize::from(i)].clone(), t))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(TOP_N);
    v
}

pub(crate) fn current_offset_ms() -> i64 {
    utc_offset_ms(&local_utc_offset()).unwrap_or(0)
}

fn now_ms() -> i64 {
    crate::store::now_ms()
}

fn day_index(off_ms: i64, ms: i64) -> i32 {
    (ms + off_ms).div_euclid(DAY_MS) as i32
}

/// `(first day, today)` day indices of the heatmap window: the Monday that
/// starts the week 52 weeks before the current week, through today — 365–371
/// days. 1970-01-01 was a Thursday, hence the `+ 3` for a Monday-first week.
pub(crate) fn heat_bounds(off_ms: i64, now: i64) -> (i32, i32) {
    let today = day_index(off_ms, now);
    let weekday = (today + 3).rem_euclid(7);
    (today - weekday - 52 * 7, today)
}

/// Epoch ms where the heatmap window starts (local midnight of its Monday).
pub(crate) fn heat_since_ms() -> i64 {
    let off_ms = current_offset_ms();
    heat_day_start_ms(off_ms, heat_bounds(off_ms, now_ms()).0)
}

/// Epoch ms of the local midnight that opens `day` in the `off_ms` frame.
pub(crate) fn heat_day_start_ms(off_ms: i64, day: i32) -> i64 {
    i64::from(day) * DAY_MS - off_ms
}

/// The window's days with nothing in them yet.
pub(crate) fn blank_heat(start: i32, today: i32) -> Vec<HeatDay> {
    (start..=today)
        .map(|d| HeatDay {
            date: civil_date(i64::from(d)),
            ..Default::default()
        })
        .collect()
}

/// Days since 1970-01-01 → "YYYY-MM-DD" (proleptic Gregorian).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", y + i64::from(m <= 2), m, d)
}

/// The one scan: group the events by (local day, tool, model). `days`
/// restricts it to those local days (index frame of `off_ms`); `None` = all
/// rows including the timestamp-less ones.
fn query_groups(store: &Store, off_ms: i64, days: Option<&BTreeSet<i64>>) -> Result<Vec<Raw>> {
    let (filter, params) = match days {
        None => (String::new(), Vec::new()),
        Some(set) => {
            // Coalesce consecutive days into [from, to) ts ranges.
            let mut ranges: Vec<(i64, i64)> = Vec::new();
            for &d in set {
                let (a, b) = (d * DAY_MS - off_ms, (d + 1) * DAY_MS - off_ms);
                match ranges.last_mut() {
                    Some(last) if last.1 == a => last.1 = b,
                    _ => ranges.push((a, b)),
                }
            }
            let mut params: Vec<i64> = Vec::new();
            let conds: Vec<String> = ranges
                .iter()
                .map(|&(a, b)| {
                    params.push(a);
                    params.push(b);
                    format!(
                        "(ts_start >= ?{} AND ts_start < ?{})",
                        params.len() - 1,
                        params.len()
                    )
                })
                .collect();
            (format!("WHERE {}", conds.join(" OR ")), params)
        }
    };
    let mut st = store.conn().prepare(&format!(
        "SELECT (ts_start + {off_ms}) / {DAY_MS} AS d, app, {MODEL_EXPR} AS m, COUNT(*),
                COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(reasoning_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                COALESCE(SUM(cache_write_5m_tokens+cache_write_1h_tokens),0),
                COALESCE(SUM(credits),0), COALESCE(SUM(cost_usd),0), COALESCE(SUM(active_ms),0),
                COALESCE(SUM(duration_ms),0)
         FROM usage_events {filter} GROUP BY d, app, m"
    ))?;
    let rows = st.query_map(rusqlite::params_from_iter(params.iter()), |r| {
        Ok(Raw {
            day: r.get::<_, Option<i64>>(0)?,
            app: r.get(1)?,
            model: r.get(2)?,
            events: r.get::<_, i64>(3)? as u64,
            input: r.get::<_, i64>(4)? as u64,
            output: r.get::<_, i64>(5)? as u64,
            reasoning: r.get::<_, i64>(6)? as u64,
            cache_read: r.get::<_, i64>(7)? as u64,
            cache_write: r.get::<_, i64>(8)? as u64,
            credits: r.get(9)?,
            cost: r.get(10)?,
            active_ms: r.get::<_, i64>(11)? as u64,
            duration_ms: r.get::<_, i64>(12)? as u64,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CostSource, Provenance, UsageEvent};

    fn ev(
        key: &str,
        app: &str,
        model: Option<&str>,
        ts: Option<i64>,
        out: u64,
        cost: f64,
    ) -> UsageEvent {
        UsageEvent {
            dedup_key: key.into(),
            app: app.into(),
            model: model.map(str::to_string),
            ts_start: ts,
            input_tokens: 100 + out,
            output_tokens: out,
            cache_read_tokens: out / 2,
            cache_write_5m_tokens: out / 3,
            cost_usd: Some(cost),
            cost_source: Some(CostSource::Computed),
            provenance: Provenance::LocalJsonl,
            credits: out.is_multiple_of(3).then_some(out as f64 / 10.0),
            active_ms: Some(out as i64),
            // Every 4th call has no duration (Cursor-like), the rest do.
            duration_ms: (!out.is_multiple_of(4)).then_some(out as i64 * 11),
            ..Default::default()
        }
    }

    /// A synthetic ledger spread over ~45 local days, three tools, several
    /// models (incl. empty names falling back through MODEL_EXPR), events today,
    /// and one row without a timestamp.
    fn ledger() -> Store {
        let s = Store::open_memory().unwrap();
        let mut n = 0u64;
        let tools = ["claude", "codex", "cursor"];
        let models = [Some("opus"), Some("sonnet"), Some("gpt"), None, Some("")];
        for d in 0..45i64 {
            for (i, app) in tools.iter().enumerate() {
                for (j, m) in models.iter().enumerate() {
                    if (d + i as i64 + j as i64) % 3 == 0 {
                        continue; // sparse, like real usage
                    }
                    n += 1;
                    let ts = day_start_ms(d) + 3_600_000 * ((n % 20) as i64 + 1);
                    let mut e = ev(&format!("k{n}"), app, *m, Some(ts), n * 7, n as f64 * 0.013);
                    if m.is_none() {
                        e.request_model = Some("aliasy".into());
                    }
                    s.upsert_event(&e).unwrap();
                }
            }
        }
        // no timestamp: unbounded totals only
        s.upsert_event(&ev("nots", "claude", Some("opus"), None, 999, 1.5))
            .unwrap();
        // future-dated event still counts in Today (ts_start >= day start)
        s.upsert_event(&ev(
            "fut",
            "claude",
            Some("sonnet"),
            Some(now_ms() + 3_600_000),
            42,
            0.2,
        ))
        .unwrap();
        s
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    fn assert_totals(a: &Totals, b: &Totals, what: &str) {
        assert_eq!(a.events, b.events, "{what} events");
        assert_eq!(a.input_tokens, b.input_tokens, "{what} input");
        assert_eq!(a.output_tokens, b.output_tokens, "{what} output");
        assert_eq!(a.reasoning_tokens, b.reasoning_tokens, "{what} reasoning");
        assert_eq!(
            a.cache_read_tokens, b.cache_read_tokens,
            "{what} cache_read"
        );
        assert_eq!(
            a.cache_write_tokens, b.cache_write_tokens,
            "{what} cache_write"
        );
        assert_eq!(a.active_ms, b.active_ms, "{what} active_ms");
        assert!(
            close(a.cost_usd, b.cost_usd),
            "{what} cost {} vs {}",
            a.cost_usd,
            b.cost_usd
        );
        assert!(close(a.credits, b.credits), "{what} credits");
    }

    /// The whole point: for every range × filter combination the cube's
    /// numbers equal the SQL path's.
    fn assert_equivalent(
        store: &Store,
        cube: &Cube,
        range: Range,
        apps: Option<&[String]>,
        models: Option<&[String]>,
    ) {
        let what = format!("{range:?} apps={apps:?} models={models:?}");
        let sql = store.overview(range, apps, models).unwrap();
        let c = cube
            .overview_parts(range, apps, models)
            .unwrap_or_else(|| panic!("cube declined {what}"));
        assert_totals(&sql.today, &c.today, &format!("today [{what}]"));
        assert_totals(&sql.span, &c.span, &format!("span [{what}]"));
        assert_totals(&sql.all, &c.all, &format!("all [{what}]"));
        assert_eq!(sql.apps, c.apps, "app names [{what}]");
        assert_eq!(sql.models, c.models, "model names [{what}]");
        let ch = cube.heat(apps, models);
        assert_eq!(sql.heat.len(), ch.len(), "heat days [{what}]");
        for (x, y) in sql.heat.iter().zip(&ch) {
            assert_eq!(x.date, y.date, "heat date [{what}]");
            assert_eq!(
                (x.tokens, x.events, x.duration_ms),
                (y.tokens, y.events, y.duration_ms),
                "heat {} [{what}]",
                x.date
            );
            assert!(
                close(x.cost_usd, y.cost_usd),
                "heat cost {} [{what}]",
                x.date
            );
        }

        let by_app = |v: &[AppSummary]| -> BTreeMap<String, (u64, u64, u64, u64)> {
            v.iter()
                .map(|a| {
                    (
                        a.app.clone(),
                        (
                            a.events,
                            a.input_tokens,
                            a.output_tokens,
                            (a.cost_usd * 1e6).round() as u64,
                        ),
                    )
                })
                .collect()
        };
        assert_eq!(by_app(&sql.by_app), by_app(&c.by_app), "by_app [{what}]");
        let by_model = |v: &[ShareRow]| -> BTreeMap<String, (u64, u64, u64)> {
            v.iter()
                .map(|m| {
                    (
                        m.name.clone(),
                        (m.events, m.tokens, (m.cost_usd * 1e6).round() as u64),
                    )
                })
                .collect()
        };
        assert_eq!(
            by_model(&sql.by_model),
            by_model(&c.by_model),
            "by_model [{what}]"
        );

        assert_eq!(
            sql.daily.len(),
            c.daily.len(),
            "trend bucket count [{what}]"
        );
        for (x, y) in sql.daily.iter().zip(&c.daily) {
            assert_eq!(x.date, y.date, "bucket key [{what}]");
            assert_eq!(
                (x.events, x.tokens),
                (y.events, y.tokens),
                "bucket {} [{what}]",
                x.date
            );
            assert!(
                close(x.cost_usd, y.cost_usd),
                "bucket cost {} [{what}]",
                x.date
            );
            // top-3 by tokens — compare token values (ties may order names differently)
            let toks = |t: &[(String, u64)]| t.iter().map(|m| m.1).collect::<Vec<_>>();
            assert_eq!(
                toks(&x.top),
                toks(&y.top),
                "bucket top tokens {} [{what}]",
                x.date
            );
        }
        assert_eq!(
            store.event_count(apps, models).unwrap(),
            cube.event_count(apps, models),
            "event_count [{what}]"
        );
    }

    #[test]
    fn cube_matches_sql_across_ranges_and_filters() {
        let store = ledger();
        let cube = Cube::build(&store).unwrap();
        let some = |v: &[&str]| Some(v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let app_filters = [
            None,
            some(&["claude"]),
            some(&["codex", "cursor"]),
            some(&[]),
        ];
        let model_filters = [None, some(&["opus", "gpt"]), some(&["aliasy"]), some(&[])];
        let mut custom = Vec::new();
        custom.push(Range::custom(day_start_ms(20), day_start_ms(10) + DAY_MS)); // inverted ends get swapped
        custom.push(Range::custom(day_start_ms(3), day_start_ms(0)));
        for range in [Range::Today, Range::Week, Range::Month, Range::All]
            .into_iter()
            .chain(custom)
        {
            for a in &app_filters {
                for m in &model_filters {
                    assert_equivalent(&store, &cube, range, a.as_deref(), m.as_deref());
                }
            }
        }
    }

    #[test]
    fn heat_window_is_monday_aligned_contiguous_and_zero_filled() {
        let store = ledger();
        let heat = Cube::build(&store).unwrap().heat(None, None);
        assert!((365..=371).contains(&heat.len()), "{}", heat.len());
        let first: jiff::civil::Date = heat[0].date.parse().unwrap();
        assert_eq!(first.weekday(), jiff::civil::Weekday::Monday);
        let today = jiff::Zoned::now().date();
        assert_eq!(heat.last().unwrap().date, today.to_string());
        // Today's week column is the 53rd, with today at weekday-index row.
        let wd = i64::from(today.weekday().to_monday_zero_offset());
        assert_eq!(heat.len() as i64, 52 * 7 + wd + 1);
        let mut d = first;
        for h in &heat {
            assert_eq!(h.date, d.to_string());
            d = d.tomorrow().unwrap();
        }
        // The fixture spans 45 days back; older days are present but empty.
        let old = &heat[0];
        assert_eq!((old.events, old.tokens, old.duration_ms), (0, 0, 0));
        assert!(heat.iter().filter(|h| h.events > 0).count() >= 40);
        // Rows without a timestamp never land in a day.
        let all_events: u64 = heat.iter().map(|h| h.events).sum();
        assert!(all_events < store.event_count(None, None).unwrap());
    }

    #[test]
    fn heat_follows_tool_and_model_filters_and_sums_durations() {
        let store = ledger();
        let cube = Cube::build(&store).unwrap();
        let sum = |h: &[HeatDay]| -> (u64, u64, u64) {
            (
                h.iter().map(|d| d.events).sum(),
                h.iter().map(|d| d.tokens).sum(),
                h.iter().map(|d| d.duration_ms).sum(),
            )
        };
        let all = sum(&cube.heat(None, None));
        let claude = sum(&cube.heat(Some(&["claude".to_string()]), None));
        let none = sum(&cube.heat(Some(&[]), None));
        assert!(claude.0 > 0 && claude.0 < all.0);
        assert_eq!(none, (0, 0, 0));
        let opus = sum(&cube.heat(None, Some(&["opus".to_string()])));
        assert!(opus.0 > 0 && opus.0 < all.0);
        // Durations come straight from the events (NULL counts 0): the window
        // covers every timestamped, non-future fixture row.
        let sql: i64 = store
            .conn()
            .query_row(
                "SELECT COALESCE(SUM(duration_ms),0) FROM usage_events
                 WHERE ts_start IS NOT NULL AND ts_start < ?1",
                [heat_day_start_ms(
                    cube.off_ms,
                    heat_bounds(cube.off_ms, now_ms()).1 + 1,
                )],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(all.2 as i64, sql);
        assert!(all.2 > 0);
    }

    #[test]
    fn incremental_refresh_equals_a_full_build() {
        let store = ledger();
        let cube = Cube::build(&store).unwrap();
        // ingestion lands on day 5 (and a brand-new tool/model) plus today
        let d5 = day_start_ms(5) + 5_000;
        store
            .upsert_event(&ev("new1", "claude", Some("opus"), Some(d5), 500, 0.9))
            .unwrap();
        store
            .upsert_event(&ev(
                "new2",
                "windsurf",
                Some("brand-new-model"),
                Some(d5),
                70,
                0.1,
            ))
            .unwrap();
        store
            .upsert_event(&ev("new3", "codex", Some("gpt"), Some(now_ms()), 33, 0.05))
            .unwrap();
        let day5 = day_index(cube.off_ms, d5);
        let inc = cube
            .refreshed(&store, &BTreeSet::from([i64::from(day5)]))
            .unwrap();
        let full = Cube::build(&store).unwrap();
        for range in [Range::Today, Range::Week, Range::Month, Range::All] {
            for app in [None, Some(vec!["windsurf".to_string()])] {
                let a = inc.overview_parts(range, app.as_deref(), None).unwrap();
                let b = full.overview_parts(range, app.as_deref(), None).unwrap();
                assert_totals(&a.span, &b.span, &format!("span {range:?}"));
                assert_totals(&a.all, &b.all, &format!("all {range:?}"));
                assert_eq!(a.apps, b.apps);
                assert_eq!(a.models, b.models);
                assert_eq!(a.daily.len(), b.daily.len());
            }
        }
        // and both still agree with SQL
        assert_equivalent(&store, &inc, Range::Month, None, None);
        assert_equivalent(&store, &inc, Range::Today, None, None);
    }

    #[test]
    fn a_range_edge_off_the_day_boundary_falls_back() {
        let store = ledger();
        let cube = Cube::build(&store).unwrap();
        let off_boundary = Range::Custom {
            start_ms: day_start_ms(3) + 1_000,
            end_ms: day_start_ms(1),
        };
        assert!(cube.overview_parts(off_boundary, None, None).is_none());
    }

    #[test]
    fn stale_today_hours_decline_instead_of_answering_wrong() {
        let store = ledger();
        let mut cube = Cube::build(&store).unwrap();
        cube.today_day -= 1; // a midnight passed since the hours were loaded
        assert!(cube.overview_parts(Range::Today, None, None).is_none());
    }

    #[test]
    fn civil_dates_round_trip_known_days() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_103), "2025-01-15"); // BOUNDARY_MS.div_euclid(day) in store tests
        assert_eq!(civil_date(19_782), "2024-02-29"); // leap day
        assert_eq!(civil_date(-1), "1969-12-31");
        // cross-check against SQLite's own date arithmetic on a spread of days
        let s = Store::open_memory().unwrap();
        for d in [
            -800i64, 0, 59, 60, 365, 11_017, 19_782, 20_103, 20_361, 47_482,
        ] {
            let sql: String = s
                .conn()
                .query_row(
                    "SELECT strftime('%Y-%m-%d', ?1*86400, 'unixepoch')",
                    [d],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(civil_date(d), sql, "day {d}");
        }
    }
}
