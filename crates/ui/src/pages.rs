//! Page views — dumb renderers over core ViewModels. All colors/metrics come
//! from `Theme`; overview blocks are config-ordered `widgets` so users can
//! reorder/hide them (persisted in ui.json) without touching code.

use crate::config::UiConfig;
use crate::theme::Theme;
use crate::widgets as w;
use crate::{Msg, Shell, Snapshot, DETAIL_PAGE_SIZE};
use globaltokentracker_core::store::EventRow;
use globaltokentracker_core::viewmodel::fmt;
use globaltokentracker_core::viewmodel::Range;
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

/// Quota row where the percent earns a tone badge (>=50% only — sparingly).
fn quota_row_badged(theme: &Theme, label: &str, pct: f64, tone: w::BadgeTone, reset: String) -> View {
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
            cell(
                1,
                w::badge(theme, format!("{pct:.0}%"), tone),
            ),
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
        children.push(ProgressRing::new().is_indeterminate(true).is_active(true).into());
    }
    children.push(
        TextBlock::new()
            .text("正在扫描数据源…")
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

fn page_frame(theme: &Theme, body: View) -> View {
    let mut frame = Border::new()
        .padding(Thickness::xy(24.0, 16.0));
    if let Some(bg) = &theme.page_bg {
        frame = frame.background(*bg);
    }
    ScrollViewer::new().content(frame.content(body))
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
                        .content("上移"),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::MoveWidget(p.clone(), id.to_string(), 1)
                        }))
                        .content("下移"),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::HideWidget(p.clone(), id.to_string(), true)
                        }))
                        .content("隐藏"),
                )),
        )
}

fn hidden_chip(theme: &Theme, page: &str, id: &'static str, ctx: &mut ViewContext<Shell>) -> View {
    let page = page.to_string();
    Border::new()
        .padding(Thickness::xy(10.0, 4.0))
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    TextBlock::new()
                        .text(format!("已隐藏：{}", w::widget_title(id)))
                        .font_size(theme.label_size)
                        .foreground(theme.subtle)
                        .vertical_alignment(VerticalAlignment::Center),
                    Button::new()
                        .on_click(ctx.callback({
                            let p = page.clone();
                            move |_| Msg::HideWidget(p.clone(), id.to_string(), false)
                        }))
                        .content("恢复"),
                )),
        )
}

/// Render one overview widget (without edit chrome).
fn overview_widget(
    id: &str,
    s: &Snapshot,
    theme: &Theme,
    trend: &w::TrendHandle,
    ctx: &mut ViewContext<Shell>,
) -> Option<View> {
    let vm = &s.vm;
    let rl = vm.range.label();
    match id {
        "stats" => Some(
            Grid::new()
                .columns([
                    GridLength::STAR,
                    GridLength::STAR,
                    GridLength::STAR,
                    GridLength::STAR,
                ])
                .column_spacing(theme.gap)
                .children([
                    cell(
                        0,
                        w::stat_card(
                            theme,
                            Symbol::Calculator,
                            &format!("{rl} Tokens"),
                            fmt::tokens_exact(fmt::tokens_total(&vm.span)),
                            format!("事件 {}", fmt::tokens_exact(vm.span.events)),
                            false,
                            None,
                        ),
                    ),
                    cell(
                        1,
                        w::stat_card(
                            theme,
                            Symbol::Tag,
                            &format!("{rl}估算成本"),
                            fmt::usd(vm.span.cost_usd),
                            format!("全部 {}", fmt::usd(vm.all.cost_usd)),
                            true,
                            Some(("估算", w::BadgeTone::Accent)),
                        ),
                    ),
                    cell(
                        2,
                        w::stat_card(
                            theme,
                            Symbol::SyncFolder,
                            &format!("{rl}缓存读"),
                            fmt::tokens_exact(vm.span.cache_read_tokens),
                            format!("输入 {}", fmt::tokens_exact(vm.span.input_tokens)),
                            false,
                            None,
                        ),
                    ),
                    cell(
                        3,
                        w::stat_card(
                            theme,
                            Symbol::CalendarWeek,
                            &format!("{rl}事件"),
                            fmt::tokens_exact(vm.span.events),
                            format!(
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
                    ),
                ]),
        ),
        "trend" => {
            let trend_title: String = match vm.range {
                Range::Today => "今日 · 按小时".into(),
                Range::All => "全部 · 按天（近 60 桶）".into(),
                _ => format!("{rl}趋势"),
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
        "apps" => {
            let mut rows: Vec<View> = Vec::new();
            for a in vm.by_app.iter().take(8) {
                rows.push(w::key_value_row(
                    theme,
                    format!("{}  ·  {} 事件", a.app, a.events),
                    format!(
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
                        .text("暂无数据")
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
                        w::section_header(theme, Symbol::List, &format!("{rl} · 按工具")),
                        vstack(2.0, rows),
                    )),
            ))
        }
        "quotas" => {
            let mut rows: Vec<View> = Vec::new();
            for q in vm.quotas.iter().take(6) {
                let label = format!("{} · {}", q.app, q.window_kind);
                match q.used_percent {
                    Some(p) if p >= 50.0 => {
                        let tone = if p >= 80.0 { w::BadgeTone::Danger } else { w::BadgeTone::Warn };
                        rows.push(quota_row_badged(theme, &label, p, tone, fmt::until(q.resets_at)));
                    }
                    _ => rows.push(w::key_value_row(
                        theme,
                        label,
                        format!(
                            "{}  ·  reset {}",
                            q.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into()),
                            fmt::until(q.resets_at)
                        ),
                    )),
                }
            }
            if rows.is_empty() {
                rows.push(
                    TextBlock::new()
                        .text("暂无配额信号")
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
                        w::section_header(theme, Symbol::Clock, "订阅配额"),
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
                    .title("未计价模型".to_string())
                    .message(format!(
                        "{} — 请在价格页补充覆写（绝不猜价）",
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

pub fn overview_page(
    snap: Option<&Snapshot>,
    scanning: bool,
    theme: &Theme,
    config: &UiConfig,
    editing: bool,
    ctx: &mut ViewContext<Shell>,
    trend: &w::TrendHandle,
) -> View {
    let Some(s) = snap else {
        return loading(theme, scanning);
    };

    let registry = w::registry_ids();
    let order = config.order_for("overview", &registry);
    let hidden = config.hidden("overview");

    let range_item = |label: &'static str, r: Range| {
        SelectorBarItem::new()
            .text(label)
            .is_selected(s.vm.range == r)
    };
    let range_sel: View = SelectorBar::new()
        .on_selected_text_changed(ctx.callback(|t: Option<String>| {
            Msg::SetRange(t.unwrap_or_default())
        }))
        .collection_slot(
            SelectorBarSlot::Items,
            [
                KeyedView::new("today", range_item("今日", Range::Today)),
                KeyedView::new("week", range_item("近 7 天", Range::Week)),
                KeyedView::new("month", range_item("近 30 天", Range::Month)),
                KeyedView::new("all", range_item("全部", Range::All)),
            ],
        );

    let mut col: Vec<View> = vec![header(
        theme,
        "总览",
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
                .content(if editing { "完成" } else { "布局" }),
            Button::new()
                .on_click(ctx.callback(|_| Msg::Rescan))
                .content("刷新"),
        ]
        .into_iter()
        .collect(),
    )];

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
        if let Some(v) = overview_widget(id_static, s, theme, trend, ctx) {
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
    GridLength::Pixel(96.0),  // 时间
    GridLength::Pixel(72.0),  // 工具
    GridLength::STAR,         // 模型
    GridLength::Pixel(96.0),  // 输入
    GridLength::Pixel(96.0),  // 输出
    GridLength::Pixel(96.0),  // 缓存
    GridLength::Pixel(92.0),  // 成本
    GridLength::Pixel(72.0),  // 时长
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
        .content(
            Grid::new()
                .columns(DETAIL_COLS)
                .children([
                    h(theme, "时间", 0, false),
                    h(theme, "工具", 1, false),
                    h(theme, "模型", 2, false),
                    h(theme, "输入", 3, true),
                    h(theme, "输出", 4, true),
                    h(theme, "缓存", 5, true),
                    h(theme, "成本", 6, true),
                    h(theme, "时长", 7, true),
                ]),
        )
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
        .or_else(|| r.credits.map(|c| format!("{c:.1}cr")))
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
        dcell(3, dtext(theme, fmt::tokens_exact(r.input_tokens), true).into()),
        dcell(4, dtext(theme, fmt::tokens_exact(r.output_tokens), true).into()),
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
                .content("← 上一页"),
        );
    }
    nav.push(
        TextBlock::new()
            .text(format!(
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
                .content("下一页 →"),
        );
    }

    page_frame(
        theme,
        vstack(
            10.0,
            vec![
                header(
                    theme,
                    "明细",
                    vec![StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .keyed_children(keyed(nav))],
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

pub fn quota_page(snap: Option<&Snapshot>, theme: &Theme) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let mut list: Vec<View> = Vec::new();
    for q in &s.vm.quotas {
        let pct = q.used_percent.unwrap_or(0.0);
        list.push(w::card(
            theme,
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .spacing(8.0)
                .children((
                    Grid::new()
                        .columns([GridLength::STAR, GridLength::Auto])
                        .children([
                            cell(
                                0,
                                StackPanel::new()
                                    .orientation(Orientation::Horizontal)
                                    .spacing(8.0)
                                    .children((
                                        SymbolIcon::new().symbol(Symbol::Clock),
                                        TextBlock::new()
                                            .text(format!("{} · {}", q.app, q.window_kind))
                                            .font_weight(FontWeight::SEMI_BOLD)
                                            .vertical_alignment(VerticalAlignment::Center),
                                    )),
                            ),
                            cell(
                                1,
                                w::badge(
                                    theme,
                                    format!("{pct:.1}%"),
                                    if pct > 80.0 {
                                        w::BadgeTone::Danger
                                    } else if pct > 50.0 {
                                        w::BadgeTone::Warn
                                    } else {
                                        w::BadgeTone::Muted
                                    },
                                ),
                            ),
                        ]),
                    ProgressBar::new()
                        .value(pct)
                        .maximum(100.0)
                        .minimum(0.0),
                    TextBlock::new()
                        .text(format!(
                            "reset {} · account {}",
                            fmt::until(q.resets_at),
                            q.account.clone().unwrap_or_else(|| "—".into())
                        ))
                        .font_size(theme.label_size)
                        .foreground(theme.subtle),
                )),
        ));
    }
    if list.is_empty() {
        list.push(
            TextBlock::new()
                .text("暂无配额数据")
                .foreground(theme.subtle)
                .into(),
        );
    }
    page_frame(
        theme,
        vstack(
            10.0,
            vec![header(theme, "配额", vec![]), vstack(8.0, list)],
        ),
    )
}

// ---------------------------------------------------------------- sources

pub fn sources_page(snap: Option<&Snapshot>, theme: &Theme) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let mut list: Vec<View> = Vec::new();
    for h in &s.sources {
        let state = h
            .last_error
            .as_ref()
            .map(|e| format!("⚠ {e}"))
            .unwrap_or_else(|| "正常".into());
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
                                    .text(format!(
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
                .text("尚未扫描")
                .foreground(theme.subtle)
                .into(),
        );
    }
    page_frame(
        theme,
        vstack(
            10.0,
            vec![header(theme, "数据源", vec![]), vstack(8.0, list)],
        ),
    )
}

// ---------------------------------------------------------------- prices

pub fn prices_page(snap: Option<&Snapshot>, theme: &Theme) -> View {
    let Some(s) = snap else {
        return loading(theme, true);
    };
    let mut list: Vec<View> = Vec::new();
    for p in s.prices.iter().take(400) {
        list.push(
            Border::new()
                .padding(Thickness::xy(10.0, 4.0))
                .border_brush(theme.divider)
                .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                .content(
                    TextBlock::new().text(format!(
                        "{:<42} in {:>7.2}  out {:>7.2}  cr {:>7.3}  cw {:>7.3}   {}",
                        truncate(&p.model, 42),
                        p.input,
                        p.output,
                        p.cache_read,
                        p.cache_write,
                        p.source
                    ))
                    .font_size(theme.body_size),
                ),
        );
    }
    page_frame(
        theme,
        vstack(
            10.0,
            vec![
                header(theme, "价目表（$/1M tokens）", vec![]),
                TextBlock::new()
                    .text(format!(
                        "{} 个模型 · 前 400 条 · {}",
                        s.prices.len(),
                        match s.prices_synced_at {
                            Some(t) => format!(
                                "联网同步于 {} 小时前",
                                (globaltokentracker_core::store::now_ms() - t) / 3_600_000
                            ),
                            None => "仅本地种子，尚未联网同步".to_string(),
                        }
                    ))
                    .font_size(theme.body_size)
                    .foreground(theme.subtle)
                    .into(),
                vstack(0.0, list),
            ],
        ),
    )
}
