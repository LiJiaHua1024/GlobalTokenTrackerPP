//! Page views — dumb renderers over core ViewModels. All colors/metrics come
//! from `Theme`; overview blocks are config-ordered `widgets` so users can
//! reorder/hide them (persisted in ui.json) without touching code.

use crate::config::{REFRESH_OPTIONS, UiConfig, refresh_label};
use crate::i18n::{self, tr};
use crate::theme::Theme;
use crate::widgets as w;
use crate::{DETAIL_PAGE_SIZE, MenuKind, Msg, Shell, Snapshot};
use crate::{t, tf};
use globaltokentracker_core::store::EventRow;
use globaltokentracker_core::viewmodel::Range;
use globaltokentracker_core::viewmodel::{app_display, fmt};
use windows_reactor::*;

pub fn keyed(views: Vec<View>) -> impl Iterator<Item = KeyedView> {
    views
        .into_iter()
        .enumerate()
        .map(|(i, v)| KeyedView::new(i as u64, v))
}

fn cell(col: i32, v: View) -> View {
    Border::new().grid_column(col).content(v)
}

/// Grid cell with an explicit row — reflow grids need both coordinates.
fn cell_rc(col: i32, row: i32, v: View) -> View {
    Border::new().grid_column(col).grid_row(row).content(v)
}

/// Minimum cell width for the reflow grids (stat cards + share donuts).
const REFLOW_MIN_CELL: f64 = 250.0;

/// How many `min_cell`-wide columns fit in `width` DIPs of content
/// (clamped 1..=max; unknown width → `max`, matching the default window).
fn fit_cols(width: f64, min_cell: f64, gap: f64, max: usize) -> usize {
    if width <= 0.0 {
        return max;
    }
    (((width + gap) / (min_cell + gap)).floor() as usize).clamp(1, max)
}

/// N items into `cols` STAR columns × Auto rows — cells stretch to fill
/// the measured width, so wide windows get wide cards, narrow reflow.
fn reflow_grid(theme: &Theme, items: Vec<View>, cols: usize) -> View {
    let cols = cols.clamp(1, items.len().max(1));
    let rows = items.len().div_ceil(cols);
    Grid::new()
        .columns(vec![GridLength::STAR; cols])
        .rows(vec![GridLength::Auto; rows])
        .column_spacing(theme.gap)
        .row_spacing(theme.gap)
        .keyed_children(items.into_iter().enumerate().map(|(i, v)| {
            KeyedView::new(i as u64, cell_rc((i % cols) as i32, (i / cols) as i32, v))
        }))
}

/// Quota row where the percent earns a tone badge (>=50% only — sparingly).
fn quota_row_badged(
    theme: &Theme,
    label: &str,
    pct: f64,
    tone: w::BadgeTone,
    reset: String,
) -> View {
    Grid::new()
        .columns([GridLength::STAR, GridLength::Auto, GridLength::Auto])
        .column_spacing(10.0)
        .children([
            cell(
                0,
                TextBlock::new()
                    .text(label)
                    .font_size(theme.body_size)
                    .into(),
            ),
            cell(1, w::badge(theme, format!("{pct:.0}%"), tone)),
            cell(
                2,
                TextBlock::new()
                    .text(format!("reset {reset}"))
                    .font_size(theme.body_size)
                    .foreground(theme.subtle)
                    .into(),
            ),
        ])
}

fn vstack(spacing: f64, children: Vec<View>) -> View {
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .spacing(spacing)
        .keyed_children(keyed(children))
}

fn loading(theme: &Theme, scanning: bool) -> View {
    let mut children: Vec<View> = Vec::new();
    if scanning {
        children.push(
            ProgressRing::new()
                .is_indeterminate(true)
                .is_active(true)
                .into(),
        );
    }
    children.push(
        TextBlock::new()
            .text(t!("正在扫描数据源…"))
            .foreground(theme.subtle)
            .into(),
    );
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .spacing(12.0)
        .horizontal_alignment(HorizontalAlignment::Center)
        .keyed_children(keyed(children))
}

/// Top band of a page: title left, actions right.
fn header(theme: &Theme, title: &str, actions: Vec<View>) -> View {
    Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .children([
            cell(
                0,
                TextBlock::new()
                    .text(title)
                    .font_size(theme.title_size)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .foreground(theme.text)
                    .into(),
            ),
            cell(
                1,
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(10.0)
                    .keyed_children(keyed(actions)),
            ),
        ])
}

/// WinUI `CalendarDatePicker` reports the picked day as UTC-midnight
/// `DateTime` (100ns ticks since 1601); translate to the local day's
/// start-of-day epoch ms so the window follows local calendar dates.
fn picked_day_ms(d: Option<windows_time::DateTime>) -> Option<i64> {
    let ms = (d?.universal_time - 116_444_736_000_000_000) / 10_000;
    Some(globaltokentracker_core::viewmodel::utc_day_to_local_start(
        ms,
    ))
}

/// `自定义` range chrome: two calendar pickers (start day / last day) plus a
/// text echo of the resolved window — CalendarDatePicker has no `date`
/// setter in reactor 0.100, so the picked value is shown alongside.
fn custom_range_row(
    theme: &Theme,
    ctx: &mut ViewContext<Shell>,
    start_ms: i64,
    end_ms: i64,
) -> View {
    let day_label = |v: &str| {
        TextBlock::new()
            .text(v)
            .font_size(theme.body_size)
            .foreground(theme.subtle)
            .vertical_alignment(VerticalAlignment::Center)
            .into()
    };
    // end_ms is exclusive — echo the last INCLUDED day.
    let echo = format!(
        "{} → {}",
        fmt::day(Some(start_ms)),
        fmt::day(Some(end_ms - 1))
    );
    let from: View = CalendarDatePicker::new()
        .placeholder_text(tr("起始日期"))
        .on_date_changed(
            ctx.callback(|d: Option<windows_time::DateTime>| match picked_day_ms(d) {
                Some(ms) => Msg::SetCustomStart(ms),
                None => Msg::Noop,
            }),
        )
        .into();
    let to: View = CalendarDatePicker::new()
        .placeholder_text(tr("截止日期"))
        .on_date_changed(
            ctx.callback(|d: Option<windows_time::DateTime>| match picked_day_ms(d) {
                Some(ms) => Msg::SetCustomEnd(ms),
                None => Msg::Noop,
            }),
        )
        .into();
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(10.0)
        .children([
            day_label(tr("从")),
            from,
            day_label(tr("至")),
            to,
            day_label(&echo),
        ])
}

fn page_frame(theme: &Theme, body: View) -> View {
    let mut frame = Border::new().padding(Thickness::xy(24.0, 16.0));
    if let Some(bg) = &theme.page_bg {
        frame = frame.background(*bg);
    }
    ScrollViewer::new()
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .content(frame.content(body))
}

// ---------------------------------------------------------------- overview

fn edit_chrome(theme: &Theme, page: &str, id: &'static str, ctx: &mut ViewContext<Shell>) -> View {
    let page = page.to_string();
    let icon = w::widget_icon(id);
    let title = w::widget_title(id);
    Border::new()
        .background(theme.card_border)
        .padding(Thickness::xy(8.0, 3.0))
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    SymbolIcon::new().symbol(icon),
                    TextBlock::new()
                        .text(title)
                        .font_size(theme.label_size)
                        .vertical_alignment(VerticalAlignment::Center),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::MoveWidget(p.clone(), id.to_string(), -1)
                        }))
                        .content(t!("上移")),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::MoveWidget(p.clone(), id.to_string(), 1)
                        }))
                        .content(t!("下移")),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::HideWidget(p.clone(), id.to_string(), true)
                        }))
                        .content(t!("隐藏")),
                )),
        )
}

fn hidden_chip(theme: &Theme, page: &str, id: &'static str, ctx: &mut ViewContext<Shell>) -> View {
    let page = page.to_string();
    Border::new().padding(Thickness::xy(10.0, 4.0)).content(
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                TextBlock::new()
                    .text(tf!("已隐藏：{}", w::widget_title(id)))
                    .font_size(theme.label_size)
                    .foreground(theme.subtle)
                    .vertical_alignment(VerticalAlignment::Center),
                Button::new()
                    .on_click(ctx.callback({
                        let p = page.clone();
                        move |_| Msg::HideWidget(p.clone(), id.to_string(), false)
                    }))
                    .content(t!("恢复")),
            )),
    )
}

/// Render one overview widget (without edit chrome).
fn overview_widget(
    id: &str,
    s: &Snapshot,
    theme: &Theme,
    args: &OverviewArgs,
    ctx: &mut ViewContext<Shell>,
) -> Option<View> {
    let trend = args.trend;
    let vm = &s.vm;
    let rl = tr(vm.range.label());
    match id {
        // Reflow grid: columns stretch to the measured content width —
        // 4 across at normal width, 3/2/1 as the window narrows.
        "stats" => Some(reflow_grid(
            theme,
            vec![
                w::stat_card(
                    theme,
                    Symbol::Calculator,
                    &tf!("{rl} Tokens", rl),
                    fmt::tokens_exact(fmt::tokens_total(&vm.span)),
                    tf!("事件 {}", fmt::tokens_exact(vm.span.events)),
                    false,
                    None,
                ),
                w::stat_card(
                    theme,
                    Symbol::Tag,
                    &tf!("{rl}估算成本", rl),
                    fmt::usd(vm.span.cost_usd),
                    tf!("全部 {}", fmt::usd(vm.all.cost_usd)),
                    true,
                    Some((t!("估算"), w::BadgeTone::Accent)),
                ),
                w::stat_card(
                    theme,
                    Symbol::SyncFolder,
                    &tf!("{rl}缓存读", rl),
                    fmt::tokens_exact(vm.span.cache_read_tokens),
                    tf!("输入 {}", fmt::tokens_exact(vm.span.input_tokens)),
                    false,
                    None,
                ),
                w::stat_card(
                    theme,
                    Symbol::CalendarWeek,
                    &tf!("{rl}事件", rl),
                    fmt::tokens_exact(vm.span.events),
                    tf!(
                        "活跃 {}",
                        if vm.span.active_ms > 0 {
                            fmt::duration(Some(vm.span.active_ms as i64))
                        } else {
                            "—".into()
                        }
                    ),
                    false,
                    None,
                ),
            ],
            args.cols,
        )),
        "trend" => {
            let trend_title: String = match vm.range {
                Range::Today => t!("今日 · 按小时").into(),
                Range::All => t!("全部 · 按天（近 60 桶）").into(),
                Range::Custom { .. } => t!("自定义 · 按天").into(),
                _ => tf!("{rl}趋势", rl),
            };
            Some(w::card(
                theme,
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(10.0)
                    .children((
                        w::section_header(theme, Symbol::FourBars, &trend_title),
                        w::trend_strip(theme, &vm.daily, trend, ctx),
                    )),
            ))
        }
        "share" => {
            /// Top-`keep` slices by value + 其他 fold; zero-value rows can't
            /// draw a wedge so they're dropped honestly before folding.
            fn fold(mut items: Vec<(String, f64)>, keep: usize) -> Vec<(String, f64)> {
                items.retain(|(_, v)| *v > 0.0);
                items.sort_by(|a, b| b.1.total_cmp(&a.1));
                let rest: f64 = items.iter().skip(keep).map(|i| i.1).sum();
                items.truncate(keep);
                if rest > 0.0 {
                    items.push((t!("其他").to_string(), rest));
                }
                items
            }
            let app_tok = |a: &globaltokentracker_core::store::AppSummary| {
                a.input_tokens + a.output_tokens + a.cache_read_tokens
            };
            let columns = [
                (
                    tf!("{} · {}", tr("按工具"), tr("费用")),
                    fold(
                        vm.by_app
                            .iter()
                            .map(|a| (app_display(&a.app).to_string(), a.cost_usd))
                            .collect(),
                        4,
                    ),
                    fmt::usd as fn(f64) -> String,
                ),
                (
                    tf!("{} · {}", tr("按模型"), tr("费用")),
                    fold(
                        vm.by_model
                            .iter()
                            .map(|r| (r.name.clone(), r.cost_usd))
                            .collect(),
                        4,
                    ),
                    fmt::usd,
                ),
                (
                    tf!("{} · Tokens", tr("按工具")),
                    fold(
                        vm.by_app
                            .iter()
                            .map(|a| (app_display(&a.app).to_string(), app_tok(a) as f64))
                            .collect(),
                        4,
                    ),
                    |v| fmt::tokens_compact(v as u64),
                ),
                (
                    tf!("{} · Tokens", tr("按模型")),
                    fold(
                        vm.by_model
                            .iter()
                            .map(|r| (r.name.clone(), r.tokens as f64))
                            .collect(),
                        4,
                    ),
                    |v| fmt::tokens_compact(v as u64),
                ),
            ];
            let mut cells: Vec<View> = Vec::with_capacity(columns.len());
            for (i, (title, slices, f)) in columns.iter().enumerate() {
                let total: f64 = slices.iter().map(|s| s.1).sum();
                let center = f(total);
                cells.push(w::donut_cell(
                    theme,
                    title.clone(),
                    w::DonutSpec {
                        slices,
                        center,
                        fmt_v: *f,
                        key: i as u8,
                        handle: &args.donuts[i],
                    },
                    ctx,
                ));
            }
            // Donuts stretch across the measured width: 4→3→2→1 columns.
            Some(w::card(
                theme,
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(10.0)
                    .children((
                        w::section_header(theme, Symbol::Target, &tf!("{rl} · 占比分布", rl)),
                        reflow_grid(theme, cells, args.cols),
                    )),
            ))
        }
        "apps" => {
            let mut rows: Vec<View> = Vec::new();
            for a in vm.by_app.iter().take(8) {
                rows.push(w::key_value_row(
                    theme,
                    tf!("{}  ·  {} 事件", a.app, a.events),
                    tf!(
                        "{} tok  ·  {}",
                        fmt::tokens_exact(
                            a.input_tokens
                                + a.output_tokens
                                + a.cache_read_tokens
                                + a.cache_write_tokens
                        ),
                        fmt::usd(a.cost_usd)
                    ),
                ));
            }
            if rows.is_empty() {
                rows.push(
                    TextBlock::new()
                        .text(t!("暂无数据"))
                        .font_size(theme.body_size)
                        .foreground(theme.subtle)
                        .into(),
                );
            }
            Some(w::card(
                theme,
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(8.0)
                    .children((
                        w::section_header(theme, Symbol::List, &tf!("{rl} · 按工具", rl)),
                        vstack(2.0, rows),
                    )),
            ))
        }
        "quotas" => {
            let mut rows: Vec<View> = Vec::new();
            for q in vm.quotas.iter().take(6) {
                let label = tf!("{} · {}", q.app, tr(&q.window_kind));
                match q.used_percent {
                    Some(p) if p >= 50.0 => {
                        let tone = if p >= 80.0 {
                            w::BadgeTone::Danger
                        } else {
                            w::BadgeTone::Warn
                        };
                        rows.push(quota_row_badged(
                            theme,
                            &label,
                            p,
                            tone,
                            fmt::until(q.resets_at),
                        ));
                    }
                    _ => rows.push(w::key_value_row(
                        theme,
                        label,
                        tf!(
                            "{}  ·  reset {}",
                            q.used_percent
                                .map(|p| format!("{p:.0}%"))
                                .unwrap_or_else(|| "—".into()),
                            tr(&fmt::until(q.resets_at))
                        ),
                    )),
                }
            }
            if rows.is_empty() {
                rows.push(
                    TextBlock::new()
                        .text(t!("暂无配额信号"))
                        .font_size(theme.body_size)
                        .foreground(theme.subtle)
                        .into(),
                );
            }
            Some(w::card(
                theme,
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(8.0)
                    .children((
                        w::section_header(theme, Symbol::Clock, t!("订阅配额")),
                        vstack(2.0, rows),
                    )),
            ))
        }
        "unpriced" => {
            if vm.unpriced.is_empty() {
                return None;
            }
            Some(
                InfoBar::new()
                    .severity(InfoBarSeverity::Warning)
                    .is_open(true)
                    .title(t!("未计价模型").to_string())
                    .message(tf!(
                        "{} — 请在价格页补充覆写",
                        vm.unpriced
                            .iter()
                            .take(6)
                            .map(|(m, n)| format!("{m}×{n}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .into(),
            )
        }
        _ => None,
    }
}

/// Chrome-strip geometry — labels are width-pinned so the overlay panel can
/// sit under its button without measuring. Strip order: 工具 → 模型 → 刷新
/// (data filters first, the cadence setting last).
const CHROME_LEFT: f64 = 24.0;
const LABEL_W: f64 = 28.0;
const GROUP_GAP: f64 = 8.0;
const PICKER_GAP: f64 = 20.0;
const TOOLS_BTN_W: f64 = 104.0;
const MODELS_BTN_W: f64 = 104.0;
const REFRESH_BTN_W: f64 = 120.0;
const TOOLS_PANEL_X: f64 = CHROME_LEFT + LABEL_W + GROUP_GAP;
const MODELS_PANEL_X: f64 = TOOLS_PANEL_X + TOOLS_BTN_W + PICKER_GAP + LABEL_W + GROUP_GAP;
const REFRESH_PANEL_X: f64 = MODELS_PANEL_X + MODELS_BTN_W + PICKER_GAP + LABEL_W + GROUP_GAP;

/// Filter-strip state bundle — keeps `filter_chrome`/`dropdown_overlay`
/// signatures tidy as more pickers join.
pub struct ChromeState<'a> {
    pub apps: &'a Option<Vec<String>>,
    pub models: &'a Option<Vec<String>>,
    pub refresh_secs: u64,
    pub open: Option<MenuKind>,
}

/// Pill summary for a multi-select checklist: `None` = all, `Some([])` =
/// deliberately empty, `Some(f)` = partial coverage.
fn check_summary(filter: &Option<Vec<String>>, total: usize) -> String {
    match filter {
        None => t!("全部").to_string(),
        Some(f) if f.is_empty() => t!("未选").into(),
        Some(f) => tf!("已选 {}/{total}", f.len(), total),
    }
}

/// Filter strip shared by overview + detail: tool-scope, model-scope, and
/// refresh-cadence pickers on one row that cannot overflow. Buttons toggle an
/// in-content overlay (`dropdown_overlay`) instead of a system Flyout — see
/// `Shell::open_menu`.
pub fn filter_chrome(
    s: &Snapshot,
    theme: &Theme,
    state: &ChromeState,
    ctx: &mut ViewContext<Shell>,
) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(PICKER_GAP)
        .margin(Thickness::xy(CHROME_LEFT, 4.0))
        .children((
            picker_button(
                theme,
                check_summary(state.apps, s.vm.apps.len()),
                TOOLS_BTN_W,
                state.open == Some(MenuKind::Tools),
                MenuKind::Tools,
                ctx,
            ),
            picker_button(
                theme,
                check_summary(state.models, s.vm.models.len()),
                MODELS_BTN_W,
                state.open == Some(MenuKind::Models),
                MenuKind::Models,
                ctx,
            ),
            picker_button(
                theme,
                tr(refresh_label(state.refresh_secs)).to_string(),
                REFRESH_BTN_W,
                state.open == Some(MenuKind::Refresh),
                MenuKind::Refresh,
                ctx,
            ),
        ))
}

/// Label + fixed-width pill whose click toggles `kind` in `open_menu`.
/// The chevron flips while the panel is open.
fn picker_button(
    theme: &Theme,
    value: String,
    width: f64,
    open: bool,
    kind: MenuKind,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let (label, a11y) = match kind {
        MenuKind::Tools => (t!("工具"), t!("工具筛选")),
        MenuKind::Models => (t!("模型"), t!("模型筛选")),
        MenuKind::Refresh => (t!("刷新"), t!("刷新频率")),
    };
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(GROUP_GAP)
        .children((
            TextBlock::new()
                .text(label)
                .font_size(theme.label_size)
                .foreground(theme.subtle)
                .width(LABEL_W)
                .vertical_alignment(VerticalAlignment::Center),
            Button::new()
                .automation_name(a11y)
                .width(width)
                .on_click(ctx.callback(move |_| Msg::ToggleMenu(kind)))
                .content(
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(6.0)
                        .children((
                            TextBlock::new()
                                .text(value)
                                .font_size(theme.body_size)
                                .vertical_alignment(VerticalAlignment::Center),
                            TextBlock::new()
                                .text(if open { "▴" } else { "▾" })
                                .font_size(theme.label_size)
                                .foreground(theme.subtle)
                                .vertical_alignment(VerticalAlignment::Center),
                        )),
                ),
        ))
}

/// Tool-checkbox rows plus the select-all / clear footer — extracted from the
/// old flyout so the overlay and chrome stay in sync. `filter` `None` = all
/// checked; `Some(vec![])` = deliberately empty view.
fn tools_menu_items(
    s: &Snapshot,
    theme: &Theme,
    filter: &Option<Vec<String>>,
    ctx: &mut ViewContext<Shell>,
) -> Vec<View> {
    let mut col: Vec<View> = Vec::with_capacity(s.vm.apps.len() + 2);
    for name in &s.vm.apps {
        let checked = filter.as_ref().is_none_or(|f| f.contains(name));
        let n = name.clone();
        col.push(
            CheckBox::new()
                .is_checked(checked)
                .on_is_checked_changed(ctx.callback(move |on: bool| Msg::ToggleApp(n.clone(), on)))
                .content(
                    TextBlock::new()
                        .text(name.clone())
                        .font_size(theme.body_size),
                ),
        );
    }
    col.push(
        Border::new()
            .height(1.0)
            .background(theme.divider)
            .margin(Thickness::xy(0.0, 6.0))
            .into(),
    );
    col.push(
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                Button::new()
                    .on_click(ctx.callback(|_| Msg::SetApps(None)))
                    .content(t!("全选")),
                Button::new()
                    .on_click(ctx.callback(|_| Msg::SetApps(Some(Vec::new()))))
                    .content(t!("清空")),
            )),
    );
    col
}

/// Model-checkbox rows plus the select-all / clear footer. The list is
/// capped + scrollable — dozens of distinct models land here and a fixed
/// height keeps the card from running off the window. `s.vm.models` is
/// already scoped by the tool filter (cascade), not by the model filter.
fn models_menu_items(
    s: &Snapshot,
    theme: &Theme,
    filter: &Option<Vec<String>>,
    ctx: &mut ViewContext<Shell>,
) -> Vec<View> {
    let mut checks: Vec<View> = Vec::with_capacity(s.vm.models.len());
    for name in &s.vm.models {
        let checked = filter.as_ref().is_none_or(|f| f.contains(name));
        let n = name.clone();
        checks.push(
            CheckBox::new()
                .is_checked(checked)
                .on_is_checked_changed(
                    ctx.callback(move |on: bool| Msg::ToggleModel(n.clone(), on)),
                )
                .content(
                    TextBlock::new()
                        .text(truncate(name, 36))
                        .font_size(theme.body_size)
                        .tooltip(name.clone()),
                ),
        );
    }
    vec![
        ScrollViewer::new()
            .max_height(300.0)
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .content(
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(10.0)
                    .keyed_children(keyed(checks)),
            ),
        Border::new()
            .height(1.0)
            .background(theme.divider)
            .margin(Thickness::xy(0.0, 6.0))
            .into(),
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                Button::new()
                    .on_click(ctx.callback(|_| Msg::SetModels(None)))
                    .content(t!("全选")),
                Button::new()
                    .on_click(ctx.callback(|_| Msg::SetModels(Some(Vec::new()))))
                    .content(t!("清空")),
            )),
    ]
}

/// Refresh-cadence radio rows — single-select, so a pick light-dismisses the
/// panel (`SetRefreshSecs` clears `open_menu`). `0` seconds = file-watch only.
fn refresh_menu_items(theme: &Theme, secs: u64, ctx: &mut ViewContext<Shell>) -> Vec<View> {
    let mut rows: Vec<View> = Vec::with_capacity(REFRESH_OPTIONS.len());
    for (s, label) in REFRESH_OPTIONS {
        rows.push(
            RadioButton::new()
                .group_name("refresh-cadence")
                .is_checked(s == secs)
                .on_checked(ctx.callback(move |on: bool| {
                    if on {
                        Msg::SetRefreshSecs(s)
                    } else {
                        Msg::Noop
                    }
                }))
                .content(TextBlock::new().text(tr(label)).font_size(theme.body_size)),
        );
    }
    rows
}

/// In-content dropdown layer, rendered as the last child of the content cell
/// so it paints over the page. The card carries our own border + background —
/// the system FlyoutPresenter (which draws the pale surface stroke that read
/// as a bright halo over dark chrome) is gone entirely. A full-area
/// transparent border underneath swallows outside clicks to light-dismiss;
/// the trigger row above stays live (overlay only covers the content row).
pub fn dropdown_overlay(
    s: &Snapshot,
    theme: &Theme,
    state: &ChromeState,
    open: MenuKind,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let (x, items) = match open {
        MenuKind::Tools => (TOOLS_PANEL_X, tools_menu_items(s, theme, state.apps, ctx)),
        MenuKind::Models => (
            MODELS_PANEL_X,
            models_menu_items(s, theme, state.models, ctx),
        ),
        MenuKind::Refresh => (
            REFRESH_PANEL_X,
            refresh_menu_items(theme, state.refresh_secs, ctx),
        ),
    };
    // Two-layer fill: `card_bg` is a *translucent* Fluent layer brush — over
    // page content it lets the rows beneath bleed through (reads as a broken
    // transparent popup). An opaque base under the tint composites to the
    // same elevated-card look the cards get over the window surface.
    let base = theme
        .page_bg
        .unwrap_or(Brush::Theme(ThemeBrush::SolidBackground));
    // Margin positions the card inside the content cell — plain Grid, no
    // Canvas (its empty-area hits can swallow presses meant for siblings).
    Border::new()
        .grid_row(2)
        .horizontal_alignment(HorizontalAlignment::Left)
        .vertical_alignment(VerticalAlignment::Top)
        .margin(Thickness::new(x, 4.0, 0.0, 0.0))
        .background(base)
        .border_brush(theme.card_border)
        .border_thickness(theme.card_border_thickness())
        .corner_radius(CornerRadius::uniform(theme.radius))
        .content(
            Border::new()
                .background(theme.card_bg)
                .corner_radius(CornerRadius::uniform(theme.radius - 1.0))
                .padding(Thickness::xy(12.0, 8.0))
                .content(
                    StackPanel::new()
                        .orientation(Orientation::Vertical)
                        .spacing(10.0)
                        .keyed_children(keyed(items)),
                ),
        )
}

/// Bundle for the overview page — keeps the signature under the arg limit
/// and makes the shell→page handoff explicit.
pub struct OverviewArgs<'a> {
    pub scanning: bool,
    pub config: &'a UiConfig,
    pub editing: bool,
    pub trend: &'a w::TrendHandle,
    /// Reflow column count from the width ruler (stats + share grids).
    pub cols: usize,
    /// Bound to the 1-DIP ruler panel mounted on this page.
    pub ruler: &'a ElementRef<SwapChainPanel>,
    /// Hover state per share-donut column (pointer → Msg → invalidation).
    pub donuts: &'a [w::DonutHandle; 4],
}

pub fn overview_page(
    snap: Option<&Snapshot>,
    theme: &Theme,
    ctx: &mut ViewContext<Shell>,
    args: &OverviewArgs,
) -> View {
    let (config, editing) = (args.config, args.editing);
    let Some(s) = snap else {
        return loading(theme, args.scanning);
    };
    let scanning = args.scanning;

    let registry = w::registry_ids();
    let order = config.order_for("overview", &registry);
    let hidden = config.hidden("overview");

    let range_item = |label: &'static str, r: Range| {
        SelectorBarItem::new()
            .text(tr(label))
            .is_selected(s.vm.range == r)
    };
    let is_custom = matches!(s.vm.range, Range::Custom { .. });
    let range_sel: View = SelectorBar::new()
        .on_selected_text_changed(ctx.callback(|t: Option<String>| {
            // SelectorBarItem carries only text — map the localized label
            // back to the semantic value (works in either UI language).
            let want = t.unwrap_or_default();
            if want == tr("自定义") {
                return Msg::PickCustomRange;
            }
            let r = Range::LIST
                .iter()
                .find(|r| tr(r.label()) == want)
                .copied()
                .unwrap_or_default();
            Msg::SetRange(r)
        }))
        .collection_slot(
            SelectorBarSlot::Items,
            [
                KeyedView::new("today", range_item("今日", Range::Today)),
                KeyedView::new("week", range_item("近 7 天", Range::Week)),
                KeyedView::new("month", range_item("近 30 天", Range::Month)),
                KeyedView::new("all", range_item("全部", Range::All)),
                KeyedView::new(
                    "custom",
                    SelectorBarItem::new()
                        .text(tr("自定义"))
                        .is_selected(is_custom),
                ),
            ],
        );

    let mut col: Vec<View> = vec![header(
        theme,
        t!("总览"),
        vec![
            range_sel,
            if scanning {
                ProgressRing::new()
                    .is_indeterminate(true)
                    .is_active(true)
                    .width(18.0)
                    .height(18.0)
                    .vertical_alignment(VerticalAlignment::Center)
                    .into()
            } else {
                Border::new().width(0.0).into()
            },
            Button::new()
                .on_click(ctx.callback(|_| Msg::ToggleEdit))
                .content(if editing { t!("完成") } else { t!("布局") }),
            Button::new()
                .on_click(ctx.callback(|_| Msg::Rescan))
                .content(t!("刷新")),
        ]
        .into_iter()
        .collect(),
    )];

    // 1-DIP full-width ruler: its surface Metrics report the real content
    // width → column-count changes are quantized here so a resize drag
    // only rebuilds when a column boundary is crossed.
    let on_width = ctx.callback(|cols: usize| Msg::SetOverviewCols(cols));
    ctx.use_effect("overview-ruler", (), {
        let ruler = args.ruler.clone();
        move || {
            let last = std::cell::Cell::new(0usize);
            let obs = ruler.observe_surface(move |event| {
                if let SwapChainPanelEvent::Metrics { width, .. } = event {
                    let cols = fit_cols(width, REFLOW_MIN_CELL, 12.0, 4);
                    if cols != last.get() {
                        last.set(cols);
                        let _ = on_width.call(cols);
                    }
                }
            });
            Some(Box::new(move || drop(obs)))
        }
    });
    col.push(
        SwapChainPanel::new()
            .element_ref(args.ruler)
            .height(1.0)
            .into(),
    );

    if let Range::Custom { start_ms, end_ms } = s.vm.range {
        col.push(custom_range_row(theme, ctx, start_ms, end_ms));
    }

    for id in order.iter() {
        let id_static: &'static str = match registry.iter().find(|r| **r == id) {
            Some(r) => r,
            None => continue,
        };
        if hidden.contains(id) {
            if editing {
                col.push(hidden_chip(theme, "overview", id_static, ctx));
            }
            continue;
        }
        if let Some(v) = overview_widget(id_static, s, theme, args, ctx) {
            if editing {
                col.push(vstack(
                    4.0,
                    vec![edit_chrome(theme, "overview", id_static, ctx), v],
                ));
            } else {
                col.push(v);
            }
        }
    }

    page_frame(theme, vstack(theme.section_gap, col))
}

// ---------------------------------------------------------------- detail

/// Shared column shape for header + every data row — identical widths on each
/// per-row Grid keep columns aligned without one giant 200-row measure pass.
const DETAIL_COLS: [GridLength; 8] = [
    GridLength::Pixel(96.0), // 时间
    GridLength::Pixel(72.0), // 工具
    GridLength::STAR,        // 模型
    GridLength::Pixel(96.0), // 输入
    GridLength::Pixel(96.0), // 输出
    GridLength::Pixel(96.0), // 缓存
    GridLength::Pixel(92.0), // 成本
    GridLength::Pixel(72.0), // 时长
];

fn dcell(col: i32, v: View) -> View {
    Border::new().grid_column(col).content(v)
}

fn dtext(theme: &Theme, text: String, right: bool) -> TextBlock {
    let t = TextBlock::new()
        .text(text)
        .font_size(theme.body_size)
        .vertical_alignment(VerticalAlignment::Center);
    if right {
        t.horizontal_alignment(HorizontalAlignment::Right)
    } else {
        t.horizontal_alignment(HorizontalAlignment::Left)
    }
}

fn detail_header(theme: &Theme) -> View {
    let h = |theme: &Theme, text: &str, col: i32, right: bool| -> View {
        dcell(
            col,
            dtext(theme, text.into(), right)
                .font_size(theme.label_size)
                .font_weight(FontWeight::SEMI_BOLD)
                .foreground(theme.subtle)
                .into(),
        )
    };
    Border::new()
        .padding(Thickness::xy(10.0, 6.0))
        .border_brush(theme.divider)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(Grid::new().columns(DETAIL_COLS).children([
            h(theme, t!("时间"), 0, false),
            h(theme, t!("工具"), 1, false),
            h(theme, t!("模型"), 2, false),
            h(theme, t!("输入"), 3, true),
            h(theme, t!("输出"), 4, true),
            h(theme, t!("缓存"), 5, true),
            h(theme, t!("成本"), 6, true),
            h(theme, t!("时长"), 7, true),
        ]))
}

fn event_row(theme: &Theme, r: &EventRow, zebra: bool) -> View {
    let model = r
        .model
        .clone()
        .or_else(|| r.pricing_model.clone())
        .unwrap_or_else(|| "—".into());
    let mut cost = r
        .cost_usd
        .map(fmt::usd)
        .or_else(|| r.credits.map(|c| format!("{}cr", fmt::trim_f(c, 1))))
        .unwrap_or_else(|| "—".into());
    match r.cost_source.as_deref() {
        Some("estimated") => cost.push_str(" ≈"),
        Some("provider_reported") => cost.push_str(" ↺"),
        _ => {}
    }
    let mut cost_children: Vec<View> = vec![dtext(theme, cost, true).into()];
    if r.cost_source.as_deref() == Some("unpriced") {
        cost_children.push(w::badge(theme, "unpriced".into(), w::BadgeTone::Warn));
    }
    let cells: [View; 8] = [
        dcell(0, dtext(theme, fmt::ts_short(r.ts_start), false).into()),
        dcell(1, dtext(theme, r.app.clone(), false).into()),
        dcell(
            2,
            dtext(theme, truncate(&model, 40), false)
                .foreground(theme.subtle)
                .into(),
        ),
        dcell(
            3,
            dtext(theme, fmt::tokens_exact(r.input_tokens), true).into(),
        ),
        dcell(
            4,
            dtext(theme, fmt::tokens_exact(r.output_tokens), true).into(),
        ),
        dcell(
            5,
            dtext(
                theme,
                fmt::tokens_exact(r.cache_read_tokens + r.cache_write_tokens),
                true,
            )
            .foreground(theme.subtle)
            .into(),
        ),
        dcell(
            6,
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .horizontal_alignment(HorizontalAlignment::Right)
                .keyed_children(keyed(cost_children)),
        ),
        dcell(7, dtext(theme, fmt::duration(r.duration_ms), true).into()),
    ];
    let mut row = Border::new().padding(Thickness::xy(10.0, 5.0));
    if theme.line_separators {
        row = row
            .border_brush(theme.divider)
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0));
    }
    if zebra {
        // ~4% gray reads on both light and dark Fluent surfaces.
        row = row.background(Brush::Solid(Color::argb(10, 128, 128, 128)));
    }
    row.content(Grid::new().columns(DETAIL_COLS).children(cells))
        .tooltip(r.raw_ref.clone().unwrap_or_default())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

pub fn detail_page(snap: Option<&Snapshot>, theme: &Theme, ctx: &mut ViewContext<Shell>) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let d = &s.detail;
    let list: Vec<View> = d
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| event_row(theme, r, i % 2 == 1))
        .collect();
    let pages = (d.total as i64 + DETAIL_PAGE_SIZE - 1) / DETAIL_PAGE_SIZE;
    let page = d.page;
    let mut nav: Vec<View> = Vec::new();
    if page > 0 {
        nav.push(
            Button::new()
                .on_click(ctx.callback(move |_| Msg::DetailPage(page - 1)))
                .content(t!("← 上一页")),
        );
    }
    nav.push(
        TextBlock::new()
            .text(tf!(
                "第 {} / {} 页 · 共 {} 条",
                page + 1,
                pages.max(1),
                d.total
            ))
            .font_size(theme.body_size)
            .foreground(theme.subtle)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
    );
    if page + 1 < pages {
        nav.push(
            Button::new()
                .on_click(ctx.callback(move |_| Msg::DetailPage(page + 1)))
                .content(t!("下一页 →")),
        );
    }

    page_frame(
        theme,
        vstack(
            10.0,
            vec![
                header(
                    theme,
                    t!("明细"),
                    vec![
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(8.0)
                            .keyed_children(keyed(nav)),
                    ],
                ),
                w::card(
                    theme,
                    vstack(
                        0.0,
                        std::iter::once(detail_header(theme)).chain(list).collect(),
                    ),
                ),
            ],
        ),
    )
}

// ---------------------------------------------------------------- quota

/// One quota window row inside an app group card — distinct labels per
/// window_kind, percent badge + bar when known, usage/limit and reset on the
/// meta line.
fn quota_row(theme: &Theme, q: &globaltokentracker_core::store::QuotaRow) -> View {
    let label = tr(globaltokentracker_core::viewmodel::quota_kind_label(
        &q.window_kind,
    ));
    let mut title: Vec<View> = vec![
        TextBlock::new()
            .text(label)
            .font_size(theme.body_size)
            .font_weight(FontWeight::SEMI_BOLD)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
    ];
    if let Some(acc) = &q.account {
        title.push(w::badge(theme, acc.clone(), w::BadgeTone::Muted));
    }
    let mut head: Vec<View> = vec![cell(
        0,
        StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .keyed_children(keyed(title)),
    )];
    if let Some(pct) = q.used_percent {
        head.push(cell(
            1,
            w::badge(
                theme,
                format!("{}%", fmt::trim_f(pct, 1)),
                if pct > 80.0 {
                    w::BadgeTone::Danger
                } else if pct > 50.0 {
                    w::BadgeTone::Warn
                } else {
                    w::BadgeTone::Muted
                },
            ),
        ));
    }
    let mut body: Vec<View> = vec![
        Grid::new()
            .columns([GridLength::STAR, GridLength::Auto])
            .keyed_children(keyed(head)),
    ];
    if let Some(pct) = q.used_percent {
        body.push(
            ProgressBar::new()
                .value(pct)
                .maximum(100.0)
                .minimum(0.0)
                .into(),
        );
    }
    // `credits` rows carry remaining balance in `used` (see quota.rs) — never
    // dress it as "已用". Compact 万/亿 keeps the meta line scannable; exact
    // counts go on the tooltip for reconciliation.
    let (usage, usage_exact) = if q.window_kind == "credits" {
        (
            q.used.map(|u| tf!("余额 {}", i18n::compact(u as u64))),
            q.used.map(|u| tf!("余额 {}", fmt::tokens_exact(u as u64))),
        )
    } else {
        let pair = |(u, l): (f64, f64)| {
            (
                tf!(
                    "已用 {} / 上限 {}",
                    i18n::compact(u as u64),
                    i18n::compact(l as u64)
                ),
                tf!(
                    "已用 {} / 上限 {}",
                    fmt::tokens_exact(u as u64),
                    fmt::tokens_exact(l as u64)
                ),
            )
        };
        match (q.used, q.limit_value) {
            (Some(u), Some(l)) => {
                let (c, e) = pair((u, l));
                (Some(c), Some(e))
            }
            (Some(u), None) => (
                Some(tf!("用量 {}", i18n::compact(u as u64))),
                Some(tf!("用量 {}", fmt::tokens_exact(u as u64))),
            ),
            (None, Some(l)) => (
                Some(tf!("上限 {}", i18n::compact(l as u64))),
                Some(tf!("上限 {}", fmt::tokens_exact(l as u64))),
            ),
            (None, None) => (None, None),
        }
    };
    let mut meta = tf!(
        "重置 {} · 采集 {}",
        tr(&fmt::until(q.resets_at)),
        fmt::ts_short(Some(q.captured_at))
    );
    if let Some(u) = usage {
        meta = tf!("{u} · {meta}", u, meta);
    }
    let mut meta_v: View = TextBlock::new()
        .text(meta)
        .font_size(theme.label_size)
        .foreground(theme.subtle)
        .into();
    if let Some(e) = usage_exact {
        meta_v = meta_v.tooltip(e);
    }
    body.push(meta_v);
    vstack(6.0, body)
}

/// Quota page — one collapsible card per tool (a vendor can expose several
/// windows; a flat list once flooded the page with WorkBuddy session marks).
pub fn quota_page(
    snap: Option<&Snapshot>,
    theme: &Theme,
    collapsed: &std::collections::BTreeSet<String>,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let mut list: Vec<View> = Vec::new();
    for g in &s.vm.quota_groups {
        let open = !collapsed.contains(&g.app);
        let worst = g.worst_pct.unwrap_or(0.0);
        let app_key = g.app.clone();
        let header_btn = Button::new()
            .automation_name(tf!("配额组 {}", g.display))
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .horizontal_content_alignment(HorizontalAlignment::Stretch)
            .on_click(ctx.callback(move |_| Msg::ToggleQuotaGroup(app_key.clone())))
            .content(
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto, GridLength::Auto])
                    .column_spacing(8.0)
                    .children([
                        cell(
                            0,
                            StackPanel::new()
                                .orientation(Orientation::Horizontal)
                                .spacing(8.0)
                                .children((
                                    SymbolIcon::new().symbol(Symbol::Clock),
                                    TextBlock::new()
                                        .text(tf!("{} · {} 项配额", g.display, g.rows.len()))
                                        .font_weight(FontWeight::SEMI_BOLD)
                                        .vertical_alignment(VerticalAlignment::Center),
                                )),
                        ),
                        cell(
                            1,
                            w::badge(
                                theme,
                                format!("{}%", fmt::trim_f(worst, 1)),
                                if worst > 80.0 {
                                    w::BadgeTone::Danger
                                } else if worst > 50.0 {
                                    w::BadgeTone::Warn
                                } else {
                                    w::BadgeTone::Muted
                                },
                            ),
                        ),
                        cell(
                            2,
                            TextBlock::new()
                                .text(if open { "▴" } else { "▾" })
                                .font_size(theme.label_size)
                                .foreground(theme.subtle)
                                .vertical_alignment(VerticalAlignment::Center)
                                .into(),
                        ),
                    ]),
            );
        let mut inner: Vec<View> = vec![header_btn];
        if open {
            inner.extend(g.rows.iter().map(|q| quota_row(theme, q)));
        }
        list.push(w::card(theme, vstack(10.0, inner)));
    }
    if list.is_empty() {
        list.push(
            TextBlock::new()
                .text(t!("暂无配额数据"))
                .foreground(theme.subtle)
                .into(),
        );
    }
    page_frame(
        theme,
        vstack(
            10.0,
            vec![header(theme, t!("配额"), vec![]), vstack(8.0, list)],
        ),
    )
}

// ---------------------------------------------------------------- sources

pub fn sources_page(snap: Option<&Snapshot>, theme: &Theme) -> View {
    let Some(list_src) = snap.and_then(|s| s.sources.as_ref()) else {
        return loading(theme, true);
    };
    let mut list: Vec<View> = Vec::new();
    for h in list_src {
        let state = h
            .last_error
            .as_ref()
            .map(|e| tf!("⚠ {e}", e))
            .unwrap_or_else(|| t!("正常").into());
        let err = h.last_error.is_some();
        list.push(w::card(
            theme,
            Grid::new()
                .columns([GridLength::STAR, GridLength::Auto])
                .children([
                    cell(
                        0,
                        StackPanel::new()
                            .orientation(Orientation::Vertical)
                            .spacing(4.0)
                            .children((
                                StackPanel::new()
                                    .orientation(Orientation::Horizontal)
                                    .spacing(8.0)
                                    .children((
                                        SymbolIcon::new().symbol(Symbol::World),
                                        TextBlock::new()
                                            .text(h.source.clone())
                                            .font_weight(FontWeight::SEMI_BOLD)
                                            .vertical_alignment(VerticalAlignment::Center),
                                    )),
                                TextBlock::new()
                                    .text(tf!(
                                        "{} 文件 · 累计 {} 行 · 游标 {} · 上次 {}",
                                        h.files_seen,
                                        h.rows_ingested,
                                        h.cursors,
                                        fmt::ts_short(h.last_synced_at)
                                    ))
                                    .font_size(theme.label_size)
                                    .foreground(theme.subtle),
                            )),
                    ),
                    cell(
                        1,
                        if err {
                            w::badge(theme, state, w::BadgeTone::Danger)
                        } else {
                            TextBlock::new()
                                .text(state)
                                .font_size(theme.label_size)
                                .foreground(theme.ok)
                                .into()
                        },
                    ),
                ]),
        ));
    }
    if list.is_empty() {
        list.push(
            TextBlock::new()
                .text(t!("尚未扫描"))
                .foreground(theme.subtle)
                .into(),
        );
    }
    page_frame(
        theme,
        vstack(
            10.0,
            vec![header(theme, t!("数据源"), vec![]), vstack(8.0, list)],
        ),
    )
}

// ---------------------------------------------------------------- prices

// -------------------------------------------------------------- prices
// Column grid: model | input | output | cache-read | cache-write | source.
const PRICE_COLS: [GridLength; 6] = [
    GridLength::STAR,
    GridLength::Pixel(96.0),
    GridLength::Pixel(96.0),
    GridLength::Pixel(96.0),
    GridLength::Pixel(96.0),
    GridLength::Pixel(110.0),
];
/// $/1M values vary from 0.0001 to thousands — trim, don't pad.
fn price_num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

fn price_head(theme: &Theme) -> View {
    let head_cell = |i: i32, h: &str| {
        dcell(
            i,
            dtext(theme, h.into(), i > 0)
                .font_size(theme.label_size)
                .font_weight(FontWeight::SEMI_BOLD)
                .foreground(theme.subtle)
                .into(),
        )
    };
    let cells: [View; 6] = [
        head_cell(0, t!("模型")),
        head_cell(1, t!("输入")),
        head_cell(2, t!("输出")),
        head_cell(3, t!("缓存读")),
        head_cell(4, t!("缓存写")),
        head_cell(5, t!("来源")),
    ];
    Border::new()
        .padding(Thickness::xy(10.0, 6.0))
        .border_brush(theme.divider)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
        .content(Grid::new().columns(PRICE_COLS).children(cells))
}

fn price_row(theme: &Theme, p: &globaltokentracker_core::store::PriceRow, zebra: bool) -> View {
    let tone = if p.source == "seed" {
        w::BadgeTone::Muted
    } else {
        w::BadgeTone::Accent
    };
    let cells: [View; 6] = [
        dcell(
            0,
            TextBlock::new()
                .text(truncate(&p.model, 56))
                .font_size(theme.body_size)
                .vertical_alignment(VerticalAlignment::Center)
                .into(),
        ),
        dcell(1, dtext(theme, price_num(p.input), true).into()),
        dcell(2, dtext(theme, price_num(p.output), true).into()),
        dcell(
            3,
            dtext(theme, price_num(p.cache_read), true)
                .foreground(theme.subtle)
                .into(),
        ),
        dcell(
            4,
            dtext(theme, price_num(p.cache_write), true)
                .foreground(theme.subtle)
                .into(),
        ),
        dcell(
            5,
            Border::new()
                .horizontal_alignment(HorizontalAlignment::Right)
                .content(w::badge(theme, p.source.clone(), tone)),
        ),
    ];
    // Same chrome as detail rows: separators gated by theme, alternating
    // ~4% zebra, full model id on hover.
    let mut row = Border::new().padding(Thickness::xy(10.0, 5.0));
    if theme.line_separators {
        row = row
            .border_brush(theme.divider)
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0));
    }
    if zebra {
        row = row.background(Brush::Solid(Color::argb(10, 128, 128, 128)));
    }
    row.content(Grid::new().columns(PRICE_COLS).children(cells))
        .tooltip(p.model.clone())
}

pub fn prices_page(snap: Option<&Snapshot>, theme: &Theme) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let Some(rows) = s.prices.as_ref() else {
        return loading(theme, true);
    };
    let list: Vec<View> = std::iter::once(price_head(theme))
        .chain(
            rows.iter()
                .take(400)
                .enumerate()
                .map(|(i, p)| price_row(theme, p, i % 2 == 1)),
        )
        .collect();
    page_frame(
        theme,
        vstack(
            10.0,
            vec![
                header(theme, t!("价目表（$/1M tokens）"), vec![]),
                TextBlock::new()
                    .text(tf!(
                        "{} 个模型 · 前 400 条 · {}",
                        rows.len(),
                        match s.prices_synced_at {
                            Some(t) => tf!(
                                "联网同步于 {} 小时前",
                                (globaltokentracker_core::store::now_ms() - t) / 3_600_000
                            ),
                            None => t!("仅本地种子，尚未联网同步").to_string(),
                        }
                    ))
                    .font_size(theme.body_size)
                    .foreground(theme.subtle)
                    .into(),
                // Card chrome around the table — same as the detail page.
                w::card(theme, vstack(0.0, list)),
            ],
        ),
    )
}

// ---------------------------------------------------------------- settings

/// (semantic value, zh label) — labels go through `tr` at render so the
/// emitted text always matches the active locale.
const THEME_OPTIONS: &[(&str, &str)] =
    &[("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")];
const ACCENT_OPTIONS: &[(&str, &str)] = &[
    ("", "默认"),
    ("#4d8ae8", "蓝色"),
    ("#10a874", "绿色"),
    ("#8b6fd8", "紫色"),
    ("#e8853d", "橙色"),
    ("#d84d5b", "红色"),
];
const LANG_OPTIONS: &[(&str, &str)] = &[("zh", "中文"), ("en", "English")];

/// Dropdown picker for one setting row. ComboBox reports the selected
/// index — semantic value lookup is by position, no label matching.
fn setting_dropdown(
    options: &'static [(&'static str, &'static str)],
    current: &str,
    ctx: &mut ViewContext<Shell>,
    map: impl Fn(&'static str) -> Msg + 'static + Clone,
) -> View {
    let selected = options.iter().position(|(value, _)| *value == current);
    ComboBox::new()
        .items_source(options.iter().map(|(_, label)| tr(label)))
        .selected_index(selected)
        .min_width(220.0)
        .on_selection_changed(ctx.callback(move |idx: Option<usize>| {
            map(idx
                .and_then(|i| options.get(i))
                .map(|(value, _)| *value)
                .unwrap_or_default())
        }))
        .into()
}

/// Label column + control + optional subtle note — consistent with the
/// pages' two-column chrome.
fn setting_row(
    theme: &Theme,
    label: &'static str,
    note: Option<&'static str>,
    control: View,
) -> View {
    let mut left: Vec<View> = vec![
        TextBlock::new()
            .text(tr(label))
            .font_size(theme.body_size)
            .font_weight(FontWeight::SEMI_BOLD)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
    ];
    if let Some(n) = note {
        left.push(
            TextBlock::new()
                .text(tr(n))
                .font_size(theme.label_size)
                .foreground(theme.subtle)
                .into(),
        );
    }
    Grid::new()
        .columns([GridLength::Pixel(300.0), GridLength::STAR])
        .children([
            cell(
                0,
                StackPanel::new().spacing(2.0).keyed_children(keyed(left)),
            ),
            cell(
                1,
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .horizontal_alignment(HorizontalAlignment::Left)
                    .vertical_alignment(VerticalAlignment::Center)
                    .children([control]),
            ),
        ])
}

/// Font picker listing every DirectWrite family installed on the machine.
/// Index 0 is the default entry; `i` maps to `families()[i-1]`. Names are
/// shown as-is — they're already localized family names, not zh UI strings.
fn font_dropdown(current: &str, ctx: &mut ViewContext<Shell>) -> View {
    let families = crate::fonts::families();
    let selected = if current.is_empty() {
        Some(0)
    } else {
        families.iter().position(|f| f == current).map(|i| i + 1)
    };
    let labels: Vec<String> = std::iter::once(tr("默认（Segoe UI）").to_string())
        .chain(families.iter().cloned())
        .collect();
    ComboBox::new()
        .items_source(labels)
        .selected_index(selected)
        .min_width(220.0)
        .on_selection_changed(ctx.callback(|idx: Option<usize>| {
            Msg::SetFontFamily(
                idx.and_then(|i| crate::fonts::families().get(i.wrapping_sub(1)))
                    .filter(|_| idx.is_some_and(|i| i > 0))
                    .cloned()
                    .unwrap_or_default(),
            )
        }))
        .into()
}

/// Body-size slider — title/h2/label keep their offsets (see `SetFontSize`),
/// so one drag rescales the whole interface. Live-previews as it moves.
fn size_control(theme: &Theme, ctx: &mut ViewContext<Shell>) -> View {
    let slider: View = Slider::new()
        .minimum(9.0)
        .maximum(18.0)
        .step_frequency(0.5)
        .value(theme.body_size.clamp(9.0, 18.0))
        .width(220.0)
        .vertical_alignment(VerticalAlignment::Center)
        .on_value_changed(ctx.callback(Msg::SetFontSize))
        .into();
    let readout: View = TextBlock::new()
        .text(format!("{:.1}", theme.body_size))
        .font_size(theme.body_size)
        .min_width(30.0)
        .vertical_alignment(VerticalAlignment::Center)
        .into();
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children([slider, readout])
}

pub fn settings_page(config: &UiConfig, theme: &Theme, ctx: &mut ViewContext<Shell>) -> View {
    let theme_row = setting_row(
        theme,
        "主题模式",
        None,
        setting_dropdown(
            THEME_OPTIONS,
            match config.window_theme.as_str() {
                "" => "system",
                v => v,
            },
            ctx,
            Msg::SetThemeMode,
        ),
    );
    let accent_row = setting_row(
        theme,
        "主题色",
        None,
        setting_dropdown(
            ACCENT_OPTIONS,
            config.theme.accent.as_deref().unwrap_or(""),
            ctx,
            Msg::SetAccent,
        ),
    );
    let font_row = setting_row(
        theme,
        "字体",
        Some("仅作用于图表文字；界面控件字体跟随系统"),
        font_dropdown(config.theme.font_family.as_deref().unwrap_or(""), ctx),
    );
    let scale_row = setting_row(theme, "界面字号", None, size_control(theme, ctx));
    let lang_row = setting_row(
        theme,
        "语言",
        None,
        setting_dropdown(
            LANG_OPTIONS,
            match config.lang.as_str() {
                "" => "zh",
                v => v,
            },
            ctx,
            Msg::SetLang,
        ),
    );
    let autostart_row = setting_row(
        theme,
        "开机自启动",
        Some("登录 Windows 后自动启动（最小化到托盘）"),
        ToggleSwitch::new()
            .is_on(config.autostart)
            .on_toggled(ctx.callback(Msg::SetAutostart))
            .into(),
    );

    page_frame(
        theme,
        vstack(
            theme.section_gap,
            vec![
                header(theme, tr("设置"), vec![]),
                w::section_header(theme, Symbol::FontColor, tr("外观")),
                w::card(
                    theme,
                    vstack(theme.gap, vec![theme_row, accent_row, font_row, scale_row]),
                ),
                w::section_header(theme, Symbol::Setting, tr("通用")),
                w::card(theme, vstack(theme.gap, vec![lang_row, autostart_row])),
            ],
        ),
    )
}
