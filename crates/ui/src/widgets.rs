//! Widget registry — each dashboard block is an addressable widget with an id,
//! title and icon. `UiConfig` orders/hides them; pages render them via `render`.
//! Adding a widget = one registry entry + one match arm.

use crate::theme::Theme;
use codeledger_core::viewmodel::fmt;
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

/// 30-day token trend — minimal Rectangle bars (Direct2D comes with M1 chart).
pub fn trend_strip(theme: &Theme, daily: &[(String, u64, f64)]) -> View {
    let max = daily.iter().map(|d| d.1).max().unwrap_or(1).max(1);
    let bars: Vec<View> = daily
        .iter()
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|(d, v, _)| {
            let h = ((*v as f64 / max as f64) * 72.0).max(2.0);
            Rectangle::new()
                .width(10.0)
                .height(h)
                .fill(theme.accent)
                .radius_x(3.0)
                .radius_y(3.0)
                .vertical_alignment(VerticalAlignment::Bottom)
                .tooltip(format!("{d}  {}", fmt::tokens(*v)))
        })
        .collect();
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(4.0)
        .height(84.0)
        .keyed_children(crate::pages::keyed(bars))
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
