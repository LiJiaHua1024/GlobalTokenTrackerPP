//! Theme tokens — every visual decision flows through `Theme` so skins are a
//! data swap (JSON config), not a code change. Named values resolve to live
//! `ThemeBrush` resources (follow Windows light/dark); "#rrggbb"/"#aarrggbb"
//! resolve to solid colors for full custom skins.

use serde::{Deserialize, Serialize};
use windows_canvas::ColorF;
use windows_reactor::{Brush, Color, ThemeBrush, Thickness};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    /// Named ("accent","card","stroke","text","subtle","danger","solid") or #hex.
    pub accent: Option<String>,
    pub accent_soft: Option<String>,
    pub text: Option<String>,
    pub subtle: Option<String>,
    pub danger: Option<String>,
    pub warn: Option<String>,
    pub ok: Option<String>,
    pub card_bg: Option<String>,
    pub card_border: Option<String>,
    pub divider: Option<String>,
    pub page_bg: Option<String>,
    pub radius: Option<f64>,
    pub card_pad: Option<f64>,
    pub gap: Option<f64>,
    pub section_gap: Option<f64>,
    pub line_separators: Option<bool>,
    /// Thin accent strip on the left edge of cards.
    pub accent_edge: Option<bool>,
    /// Honored by Direct2D chart text (DirectWrite); XAML controls follow the
    /// system font until reactor exposes a family setter.
    pub font_family: Option<String>,
    pub title_size: Option<f64>,
    pub h2_size: Option<f64>,
    pub body_size: Option<f64>,
    pub label_size: Option<f64>,
}

/// Resolved render-time tokens.
#[derive(Clone, Debug)]
pub struct Theme {
    pub accent: Brush,
    pub accent_soft: Brush,
    pub text: Brush,
    pub subtle: Brush,
    pub danger: Brush,
    pub warn: Brush,
    pub ok: Brush,
    pub card_bg: Brush,
    pub card_border: Brush,
    pub divider: Brush,
    pub page_bg: Option<Brush>,
    /// Direct2D copies of key colors — hex config values map exactly; named
    /// theme brushes fall back to Fluent constants below (theme brushes have
    /// no RGB at this layer).
    pub accent_cf: ColorF,
    pub subtle_cf: ColorF,
    pub divider_cf: ColorF,
    /// Card fill as drawn by the canvas: SwapChainPanel pixels composite
    /// straight onto the window surface, so transparency can't "see" the
    /// card's translucent ThemeBrush beneath — the canvas paints the same
    /// translucent fill itself and lands on the identical result.
    pub card_cf: ColorF,
    /// DirectWrite family for chart text.
    pub font_family: String,
    pub radius: f64,
    pub pad: f64,
    pub gap: f64,
    pub section_gap: f64,
    pub line_separators: bool,
    pub accent_edge: bool,
    pub title_size: f64,
    pub h2_size: f64,
    pub body_size: f64,
    pub label_size: f64,
}

fn brush_of(s: Option<&str>, default: Brush) -> Brush {
    let Some(s) = s else {
        return default;
    };
    match s.trim().to_ascii_lowercase().as_str() {
        "accent" => Brush::Theme(ThemeBrush::Accent),
        "accent_text" | "accenttext" => Brush::Theme(ThemeBrush::AccentText),
        "text" | "primary" => Brush::Theme(ThemeBrush::PrimaryText),
        "subtle" => Brush::Theme(ThemeBrush::AccentText),
        "card" | "card_bg" => Brush::Theme(ThemeBrush::CardBackground),
        "stroke" | "card_stroke" => Brush::Theme(ThemeBrush::CardStroke),
        "danger" | "critical" => Brush::Theme(ThemeBrush::SystemCritical),
        "danger_bg" | "critical_bg" => Brush::Theme(ThemeBrush::SystemCriticalBackground),
        "solid" | "background" => Brush::Theme(ThemeBrush::SolidBackground),
        hex => parse_hex(hex).map(Brush::Solid).unwrap_or(default),
    }
}

/// Canvas colors: hex strings map 1:1; named theme brushes can't be read back
/// as RGB, so they resolve to the Fluent defaults below.
fn colorf_of(s: Option<&str>, default: ColorF) -> ColorF {
    let Some(s) = s else { return default };
    let s = s.trim();
    let h = s.strip_prefix('#').unwrap_or(s);
    u32::from_str_radix(h, 16)
        .ok()
        .map(|v| match h.len() {
            6 => ColorF::from_rgb8(
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            ),
            8 => ColorF::from_rgba8(
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
                ((v >> 24) & 0xff) as u8,
            ),
            _ => default,
        })
        .unwrap_or(default)
}

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#').unwrap_or(s);
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some(Color::rgb(
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        )),
        8 => Some(Color::argb(
            ((v >> 24) & 0xff) as u8,
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        )),
        _ => None,
    }
}

impl Theme {
    pub fn resolve(cfg: &ThemeConfig) -> Self {
        Self {
            accent: brush_of(cfg.accent.as_deref(), Brush::Theme(ThemeBrush::Accent)),
            accent_soft: brush_of(
                cfg.accent_soft.as_deref(),
                Brush::Theme(ThemeBrush::AccentText),
            ),
            text: brush_of(cfg.text.as_deref(), Brush::Theme(ThemeBrush::PrimaryText)),
            subtle: brush_of(cfg.subtle.as_deref(), Brush::Theme(ThemeBrush::AccentText)),
            danger: brush_of(
                cfg.danger.as_deref(),
                Brush::Theme(ThemeBrush::SystemCritical),
            ),
            // Fluent warning yellow — not among the 8 theme brushes.
            warn: brush_of(cfg.warn.as_deref(), Brush::Solid(Color::rgb(255, 185, 0))),
            // Fluent success green.
            ok: brush_of(cfg.ok.as_deref(), Brush::Solid(Color::rgb(16, 168, 116))),
            card_bg: brush_of(
                cfg.card_bg.as_deref(),
                Brush::Theme(ThemeBrush::CardBackground),
            ),
            card_border: brush_of(
                cfg.card_border.as_deref(),
                Brush::Theme(ThemeBrush::CardStroke),
            ),
            divider: brush_of(cfg.divider.as_deref(), Brush::Theme(ThemeBrush::CardStroke)),
            page_bg: cfg
                .page_bg
                .as_deref()
                .map(|s| brush_of(Some(s), Brush::Theme(ThemeBrush::SolidBackground))),
            // Win11 dark accent #76B9ED; hex overrides map exactly.
            accent_cf: colorf_of(cfg.accent.as_deref(), ColorF::from_rgb8(0x76, 0xB9, 0xED)),
            subtle_cf: colorf_of(
                cfg.subtle.as_deref(),
                ColorF::from_rgba8(0x9E, 0x9E, 0x9E, 0xFF),
            ),
            divider_cf: colorf_of(
                cfg.divider.as_deref(),
                ColorF::from_rgba8(0x80, 0x80, 0x80, 0x44),
            ),
            // Fluent dark CardBackgroundFillColorDefault = white @ ~5% — the
            // canvas paints this over the same page the card brush blends
            // onto, landing pixel-identical. Hex card_bg overrides map 1:1.
            card_cf: colorf_of(
                cfg.card_bg.as_deref(),
                ColorF::from_rgba8(0xFF, 0xFF, 0xFF, 0x0D),
            ),
            font_family: cfg.font_family.clone().unwrap_or_else(|| "Segoe UI".into()),
            radius: cfg.radius.unwrap_or(8.0),
            pad: cfg.card_pad.unwrap_or(16.0),
            gap: cfg.gap.unwrap_or(12.0),
            section_gap: cfg.section_gap.unwrap_or(14.0),
            line_separators: cfg.line_separators.unwrap_or(true),
            accent_edge: cfg.accent_edge.unwrap_or(false),
            title_size: cfg.title_size.unwrap_or(22.0),
            h2_size: cfg.h2_size.unwrap_or(14.0),
            body_size: cfg.body_size.unwrap_or(12.0),
            label_size: cfg.label_size.unwrap_or(11.0),
        }
    }

    /// Card border thickness — side-aware so `accent_edge` only paints left.
    pub fn card_border_thickness(&self) -> Thickness {
        if self.accent_edge {
            Thickness::new(2.0, 1.0, 1.0, 1.0)
        } else {
            Thickness::uniform(1.0)
        }
    }
}
