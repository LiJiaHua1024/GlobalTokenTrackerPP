//! Widget registry — each dashboard block is an addressable widget with an id,
//! title and icon. `UiConfig` orders/hides them; pages render them via `render`.
//! Adding a widget = one registry entry + one match arm.

use crate::theme::Theme;
use crate::{Msg, Shell};
use codeledger_core::viewmodel::fmt;
use std::cell::Cell;
use std::rc::Rc;
use windows_canvas::Invalidator;
use windows_reactor::*;

/// (id, title, icon) — stable ids persisted in ui.json.
pub const OVERVIEW_WIDGETS: &[(&str, &str, Symbol)] = &[
    ("stats", "统计卡", Symbol::Calculator),
    ("trend", "近 30 天趋势", Symbol::FourBars),
    ("apps", "本周 · 按工具", Symbol::List),
    ("quotas", "订阅配额", Symbol::Clock),
    ("unpriced", "未计价提示", Symbol::Important),
];

pub fn widget_title(id: &str) -> &'static str {
    OVERVIEW_WIDGETS
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, t, _)| *t)
        .unwrap_or("部件")
}

pub fn widget_icon(id: &str) -> Symbol {
    OVERVIEW_WIDGETS
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, _, s)| *s)
        .unwrap_or(Symbol::Placeholder)
}

pub fn registry_ids() -> Vec<&'static str> {
    OVERVIEW_WIDGETS.iter().map(|(id, _, _)| *id).collect()
}

/// Card chrome: border + background + optional accent edge. All skin values
/// come from `theme` — a skin swap restyles every card at once.
pub fn card(theme: &Theme, content: View) -> View {
    Border::new()
        .background(theme.card_bg)
        .border_brush(theme.card_border)
        .border_thickness(theme.card_border_thickness())
        .corner_radius(CornerRadius::uniform(theme.radius))
        .padding(Thickness::uniform(theme.pad))
        .content(content)
}

/// Icon + title + hairline rule — the line is the visual divider the design
/// asks for; it stretches to fill remaining width.
pub fn section_header(theme: &Theme, icon: Symbol, title: &str) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children((
            SymbolIcon::new().symbol(icon),
            TextBlock::new()
                .text(title)
                .font_size(theme.h2_size)
                .font_weight(FontWeight::SEMI_BOLD)
                .vertical_alignment(VerticalAlignment::Center),
            Border::new()
                .height(1.0)
                .width(600.0)
                .background(theme.divider)
                .vertical_alignment(VerticalAlignment::Center),
        ))
}

/// Emphasis tones — used sparingly: only state signals earn color.
#[derive(Clone, Copy)]
pub enum BadgeTone {
    Accent,
    Warn,
    Danger,
    Muted,
}

/// Outline-style pill: 1px tone border, tinted text, transparent fill.
/// Compact and readable in both light/dark skins.
pub fn badge(theme: &Theme, text: String, tone: BadgeTone) -> View {
    let brush = match tone {
        BadgeTone::Accent => theme.accent,
        BadgeTone::Warn => theme.warn,
        BadgeTone::Danger => theme.danger,
        BadgeTone::Muted => theme.subtle,
    };
    Border::new()
        .corner_radius(CornerRadius::uniform(10.0))
        .border_brush(brush)
        .border_thickness(Thickness::uniform(1.0))
        .padding(Thickness::xy(7.0, 1.0))
        .content(
            TextBlock::new()
                .text(text)
                .font_size(theme.label_size)
                .foreground(brush),
        )
}

/// A single stat tile: icon + label + big number + sub-line.
/// `emph` paints the value in accent and `tag` pins a pill next to the label —
/// reserve both for the metric that carries the page (today's estimated cost).
pub fn stat_card(
    theme: &Theme,
    icon: Symbol,
    label: &str,
    value: String,
    sub: String,
    emph: bool,
    tag: Option<(&str, BadgeTone)>,
) -> View {
    let mut label_children: Vec<View> = vec![
        SymbolIcon::new().symbol(icon).into(),
        TextBlock::new()
            .text(label)
            .font_size(theme.label_size)
            .foreground(theme.subtle)
            .vertical_alignment(VerticalAlignment::Center)
            .into(),
    ];
    if let Some((t, tone)) = tag {
        label_children.push(badge(theme, t.to_string(), tone));
    }
    let mut value_tb = TextBlock::new()
        .text(value)
        .font_size(26.0)
        .font_weight(FontWeight::SEMI_BOLD);
    if emph {
        value_tb = value_tb.foreground(theme.accent);
    }
    card(
        theme,
        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(4.0)
            .children((
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(6.0)
                    .keyed_children(crate::pages::keyed(label_children)),
                value_tb,
                TextBlock::new()
                    .text(sub)
                    .font_size(theme.label_size)
                    .foreground(theme.accent_soft),
            )),
    )
}

/// Shared trend-hover state: pointer callbacks write `hover` via a Msg round
/// trip; the D2D draw closure reads it every invalidated frame. `width`/`count`
/// are written by the draw pass so hover math uses the real surface size.
#[derive(Default)]
pub struct TrendShared {
    pub hover: Cell<Option<usize>>,
    pub width: Cell<f32>,
    pub count: Cell<usize>,
}

/// Owned by `Shell`; cloned handles flow into the widget each render.
#[derive(Clone)]
pub struct TrendHandle {
    pub shared: Rc<TrendShared>,
    pub inv: Invalidator,
}

impl Default for TrendHandle {
    fn default() -> Self {
        Self {
            shared: Rc::new(TrendShared::default()),
            inv: Invalidator::new(),
        }
    }
}

/// 30-day token trend — Direct2D demand canvas: rounded bars (today at full
/// alpha, history softened), faint mid gridline + hairline baseline, sparse
/// date ticks and a max label drawn by DirectWrite — `theme.font_family` is
/// honored here (the one place family config takes effect on 0.100.0).
/// Pointer hover lifts the bar to full alpha and prints its date·tokens.
pub fn trend_strip(
    theme: &Theme,
    daily: &[(String, u64, f64)],
    trend: &TrendHandle,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let days: Vec<(String, u64)> = daily
        .iter()
        .rev()
        .take(30)
        .rev()
        .map(|(d, v, _)| (d.clone(), *v))
        .collect();
    let accent = theme.accent_cf;
    let subtle = theme.subtle_cf;
    let divider = theme.divider_cf;
    let family = theme.font_family.clone();
    let label_pt = theme.label_size as f32;
    let shared = trend.shared.clone();
    Border::new()
        .height(160.0)
        .on_pointer_moved(ctx.callback(|e: PointerEventInfo| Msg::TrendHover(e.x)))
        .on_pointer_exited(ctx.callback(|_| Msg::TrendLeave))
        .content(windows_canvas::canvas_invalidated(
            &trend.inv,
            move |ctx| {
            let (w, h) = (ctx.width, ctx.height);
            if w < 16.0 || h < 24.0 || days.is_empty() {
                return Ok(());
            }
            use windows_canvas::{ColorF, Rect, TextAlignment, TextFormat, Vector2};
            ctx.clear(ColorF::TRANSPARENT);

            let tf = TextFormat::new(&family, label_pt)?;
            let tf_r = tf.clone().with_alignment(TextAlignment::Trailing);
            let ink = ctx.create_solid_brush(subtle)?;
            let line = ctx.create_solid_brush(divider)?;

            // Layout: 18px top label strip, plot area, 16px bottom ticks.
            let top = 18.0f32;
            let bottom = h - 16.0;
            let plot_h = (bottom - top).max(1.0);
            let max = days.iter().map(|d| d.1).max().unwrap_or(1).max(1) as f32;

            // Max label (top-left) + faint mid gridline.
            ctx.draw_text(
                &fmt::tokens(max as u64),
                &tf,
                &Rect::new(0.0, 0.0, 120.0, top),
                &ink,
            );
            let mid_y = top + plot_h * 0.5;
            ctx.draw_line(Vector2::new(0.0, mid_y), Vector2::new(w, mid_y), &line, 1.0);
            ctx.draw_line(
                Vector2::new(0.0, bottom),
                Vector2::new(w, bottom),
                &line,
                1.0,
            );

            let n = days.len() as f32;
            let slot = w / n;
            let bar_w = (slot * 0.62).clamp(3.0, 20.0);
            let last = days.len() - 1;
            let hover = shared.hover.get().filter(|&i| i <= last);
            shared.width.set(w);
            shared.count.set(days.len());
            for (i, (d, v)) in days.iter().enumerate() {
                let bh = ((*v as f32) / max * plot_h).max(if *v > 0 { 3.0 } else { 1.5 });
                let x = slot * i as f32 + (slot - bar_w) * 0.5;
                let lit = i == last || hover == Some(i);
                let brush = ctx.create_solid_brush(ColorF::new(
                    accent.r,
                    accent.g,
                    accent.b,
                    accent.a * if lit { 1.0 } else { 0.45 },
                ))?;
                let bar = windows_canvas::RoundedRect::new(
                    Rect::new(x, bottom - bh, x + bar_w, bottom),
                    2.5,
                    2.5,
                );
                ctx.fill_rounded_rect(&bar, &brush);
                if hover == Some(i) {
                    ctx.draw_rounded_rect(&bar, &ink, 1.0);
                    // Hover detail top-right: "MM-DD · 12.3M tok".
                    ctx.draw_text(
                        &format!("{} · {} tok", d.get(5..10).unwrap_or(d), fmt::tokens(*v)),
                        &tf_r,
                        &Rect::new(w - 220.0, 0.0, w, top),
                        &ink,
                    );
                }
            }

            // Sparse date ticks: first / last day (MM-DD tail of ISO date).
            let tick = |d: &str| d.get(5..10).unwrap_or(d).to_string();
            ctx.draw_text(
                &tick(&days[0].0),
                &tf,
                &Rect::new(0.0, bottom + 2.0, 80.0, h),
                &ink,
            );
            ctx.draw_text(
                &tick(&days[last].0),
                &tf_r,
                &Rect::new(w - 80.0, bottom + 2.0, w, h),
                &ink,
            );
            Ok(())
        },
    ))
}

/// Two-column row; right-aligned meta. Hairline divider under the row when the
/// theme enables line separators.
pub fn key_value_row(theme: &Theme, left: String, right: String) -> View {
    let divider: View = if theme.line_separators {
        Border::new()
            .height(1.0)
            .background(theme.divider)
            .into()
    } else {
        Border::new().height(0.0).into()
    };
    Border::new()
        .padding(Thickness::xy(0.0, 3.0))
        .content(
            StackPanel::new()
                .orientation(Orientation::Vertical)
                .spacing(0.0)
                .children((
                    Grid::new()
                        .columns([GridLength::STAR, GridLength::Auto])
                        .children([
                            Border::new()
                                .grid_column(0)
                                .content(
                                    TextBlock::new().text(left).font_size(theme.body_size),
                                ),
                            Border::new()
                                .grid_column(1)
                                .content(
                                    TextBlock::new()
                                        .text(right)
                                        .font_size(theme.body_size)
                                        .foreground(theme.subtle),
                                ),
                        ]),
                    divider,
                )),
        )
}
