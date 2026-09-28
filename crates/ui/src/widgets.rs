//! Widget registry — each dashboard block is an addressable widget with an id,
//! title and icon. `UiConfig` orders/hides them; pages render them via `render`.
//! Adding a widget = one registry entry + one match arm.

use crate::i18n::tr;
use crate::theme::Theme;
use crate::{Msg, Shell, t, tf};
use globaltokentracker_core::viewmodel::fmt;
use std::cell::Cell;
use std::rc::Rc;
use windows_canvas::Invalidator;
use windows_reactor::*;

/// (id, title, icon) — stable ids persisted in ui.json.
pub const OVERVIEW_WIDGETS: &[(&str, &str, Symbol)] = &[
    ("stats", "统计卡", Symbol::Calculator),
    ("trend", "近 30 天趋势", Symbol::FourBars),
    ("share", "占比分布", Symbol::Target),
    ("apps", "本周 · 按工具", Symbol::List),
    ("quotas", "订阅配额", Symbol::Clock),
    ("unpriced", "未计价提示", Symbol::Important),
];

pub fn widget_title(id: &str) -> &'static str {
    OVERVIEW_WIDGETS
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, t, _)| tr(t))
        .unwrap_or(tr("部件"))
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

// ---------------------------------------------------------------- nav anim

/// Page-switch slide state — closed-form spring, evaluated at paint time.
/// `dir` +1 = forward nav (new page enters from the right, old exits left).
pub struct NavAnim {
    pub from: crate::Page,
    pub dir: f64,
    /// Wall-clock start — only the hard end-of-flight cap and diagnostics
    /// read it; motion follows `clock`.
    pub t0: std::time::Instant,
    /// Animation time (secs), advanced once per tick by the frame delta
    /// clamped to `MAX_DT`. A UI-thread stall (mounting the new page, a
    /// swapchain being created) then pauses the motion instead of letting
    /// the wall clock leap the springs forward — the "jump" that read as
    /// stutter.
    pub clock: std::cell::Cell<f64>,
    /// Page content built once when the flight begins — without it every
    /// ~16ms tick rebuilds two whole pages (every TextBlock/format!/canvas
    /// closure), which is what made the slide drop frames. `View` clones
    /// are Rc-cheap, so a tick then only allocates the ~15 margin wrappers.
    /// Filled lazily on the first animated `view()` (needs `ViewContext`).
    pub cache: std::cell::RefCell<Option<NavCache>>,
    /// Rendered frames — logged once at settle under `GTT_DEBUG` so the
    /// effective animation frame rate is observable.
    pub frames: std::cell::Cell<u32>,
    /// Frame-pacing probe (GTT_DEBUG only reads it): when the previous tick
    /// was handled, the worst tick-to-tick gap, and the slowest `view()`.
    pub last_tick: std::cell::Cell<std::time::Instant>,
    pub first_gap_ms: std::cell::Cell<u128>,
    pub max_gap_ms: std::cell::Cell<u128>,
    pub max_view_ms: std::cell::Cell<u128>,
}

/// Built-once page trees for one nav flight.
pub struct NavCache {
    /// Entering page's raw top-level blocks — `slide_children` re-wraps
    /// them each frame so per-block springs still move.
    pub enter_blocks: Rc<Vec<View>>,
    /// Leaving page fully assembled at rest — only its layer margin and
    /// opacity change, never its content.
    pub leave: View,
}

/// Underdamped spring position at `t` (secs): x(0)=x0, v(0)=0, target 0.
/// ζ<1 gives the slight overshoot that reads as "springy"; ζ≥1 uses the
/// critically-damped closed form.
fn spring(t: f64, x0: f64, zeta: f64, omega: f64) -> f64 {
    if t <= 0.0 {
        return x0;
    }
    if zeta >= 1.0 {
        return x0 * (-omega * t).exp() * (1.0 + omega * t);
    }
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    x0 * (-zeta * omega * t).exp() * ((wd * t).cos() + (zeta * omega / wd) * (wd * t).sin())
}

impl NavAnim {
    /// Animation-time length — every spring has settled well before this.
    const DONE_S: f64 = 0.6;
    /// Wall-clock backstop: a pathologically slow machine still ends the
    /// flight (the clamped clock alone would stretch it without bound).
    const HARD_CAP_MS: u128 = 1500;
    /// Largest animation-time step per tick (~2 vsyncs): normal frame-rate
    /// wobble stays real-time, a genuine stall is absorbed.
    const MAX_DT: f64 = 0.032;
    /// Blocks beyond this animate instantly (long tables stay tables).
    const MAX_SLIDE: usize = 10;

    /// Whole-layer offset for the entering page — the big "new page slides
    /// in from the side" motion. Per-block springs stack on top of this.
    /// ω=15/ζ=0.97: ~240ms of travel, near-critical so the layer snaps in
    /// instead of bouncing.
    pub fn enter_layer(&self) -> f64 {
        let t = self.clock.get();
        self.dir * 430.0 * spring(t, 1.0, 0.97, 15.0)
    }

    /// Extra horizontal offset for entering block `i` — later blocks start
    /// later, travel farther, and ride a softer spring. Blocks stay opaque:
    /// the incoming page covers the outgoing one, so the pass reads as a
    /// slide rather than a see-through crossfade.
    pub fn block(&self, i: usize) -> f64 {
        if i >= Self::MAX_SLIDE {
            return 0.0;
        }
        let t = self.clock.get() - 0.012 * i as f64;
        if t <= 0.0 {
            return self.dir * (34.0 + 10.0 * i as f64);
        }
        let k = (180.0 - 13.0 * i as f64).max(60.0);
        let zeta = 0.82 + 0.008 * i as f64;
        spring(t, self.dir * (34.0 + 10.0 * i as f64), zeta, k.sqrt())
    }

    /// Opacity for the leaving page, `None` once it is fully faded (caller
    /// then drops the layer so its subtree stops costing layout per tick).
    /// Opacity-only by design: it is a compositor property — a margin would
    /// re-arrange the whole outgoing page every frame for no visual gain.
    /// ~250ms tail backs the strip the new page hasn't covered yet.
    pub fn exit(&self) -> Option<f64> {
        let t = self.clock.get();
        let op = 1.0 - t * 4.0;
        (op > 0.0).then_some(op)
    }

    pub fn done(&self) -> bool {
        self.clock.get() >= Self::DONE_S || self.t0.elapsed().as_millis() >= Self::HARD_CAP_MS
    }

    /// Step the clock for one tick and record the pacing probe. Called from
    /// the tick handler only — `view()` just reads `clock`, so every render
    /// between ticks sees one consistent animation time.
    pub fn advance(&self) {
        let now = std::time::Instant::now();
        let gap = now.duration_since(self.last_tick.replace(now));
        if self.frames.get() == 0 {
            self.first_gap_ms.set(gap.as_millis());
        } else {
            self.max_gap_ms
                .set(self.max_gap_ms.get().max(gap.as_millis()));
        }
        self.clock
            .set(self.clock.get() + gap.as_secs_f64().min(Self::MAX_DT));
    }
}

/// Wrap each top-level page block in a slide driver — the margin trick
/// (`left=+off, right=-off`) translates without changing width. The Border
/// wraps unconditionally so the anim→rest transition diffs margins instead
/// of remounting subtrees (canvas swapchains would flash on settle).
pub fn slide_children(children: Vec<View>, anim: Option<&NavAnim>) -> Vec<View> {
    children
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let off = anim.map(|a| a.block(i)).unwrap_or(0.0);
            Border::new()
                .margin(Thickness::new(off, 0.0, -off, 0.0))
                .content(v)
        })
        .collect()
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
/// `tip` is the delayed-tooltip index — armed by `TrendTip` after the pointer
/// has dwelled on one bar (~450ms), cleared on move/leave.
#[derive(Default)]
pub struct TrendShared {
    pub hover: Cell<Option<usize>>,
    pub tip: Cell<Option<usize>>,
    /// Bar the dwell timer is armed for — moving bars rearms it.
    pub pending: Cell<Option<usize>>,
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

/// Per-donut hover state — pointer callbacks write it via a Msg round trip,
/// the draw pass reads it every invalidated frame.
#[derive(Default)]
pub struct DonutShared {
    pub hover: Cell<Option<usize>>,
}

/// Owned by `Shell`; one per share-grid column, cloned into each render.
#[derive(Clone)]
pub struct DonutHandle {
    pub shared: Rc<DonutShared>,
    pub inv: Invalidator,
}

impl Default for DonutHandle {
    fn default() -> Self {
        Self {
            shared: Rc::new(DonutShared::default()),
            inv: Invalidator::new(),
        }
    }
}

/// 30-day token trend — Direct2D demand canvas: rounded bars (today at full
/// alpha, history softened), faint mid gridline + hairline baseline, sparse
/// date ticks and a max label drawn by DirectWrite — `theme.font_family` is
/// honored here (the one place family config takes effect on 0.100.0).
/// Pointer hover lifts the bar to full alpha and prints its date·tokens;
/// dwelling ~450ms opens a tooltip card with events/cost/top-3 models.
///
/// `defer`: not this chart's turn to mount yet — see `defer_slot`.
pub fn trend_strip(
    theme: &Theme,
    daily: &[globaltokentracker_core::viewmodel::TrendBucket],
    trend: &TrendHandle,
    defer: bool,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let days: Vec<globaltokentracker_core::viewmodel::TrendBucket> =
        daily.iter().rev().take(60).rev().cloned().collect();
    let accent = theme.accent_cf;
    let subtle = theme.subtle_cf;
    let divider = theme.divider_cf;
    let card_bg = theme.card_cf;
    let family = theme.font_family.clone();
    let label_pt = theme.label_size as f32;
    let shared = trend.shared.clone();
    Border::new()
        .height(160.0)
        // Null Background = XAML skips hit-testing entirely; Transparent keeps
        // the canvas invisible yet receives PointerMoved/Exited.
        .background(Brush::Solid(Color::argb(0, 0, 0, 0)))
        .on_pointer_moved(ctx.callback(|e: PointerEventInfo| Msg::TrendHover(e.x)))
        .on_pointer_exited(ctx.callback(|_| Msg::TrendLeave))
        .content(defer_slot(
            defer,
            windows_canvas::canvas_invalidated(&trend.inv, move |ctx| {
                use windows_canvas::{ColorF, Rect, TextAlignment, TextFormat, Vector2};
                // Clear before the early return — an empty `days` must still wipe
                // the previous frame, otherwise stale bars linger after filters.
                // Painted with the card fill: swapchain transparency blends onto
                // the page below, not the card, so TRANSPARENT showed as a dark box.
                ctx.clear(card_bg);
                let (w, h) = (ctx.width, ctx.height);
                if w < 16.0 || h < 24.0 || days.is_empty() {
                    return Ok(());
                }

                let tf = TextFormat::new(&family, label_pt)?;
                let tf_r = tf.clone().with_alignment(TextAlignment::Trailing);
                let ink = ctx.create_solid_brush(subtle)?;
                let line = ctx.create_solid_brush(divider)?;

                // Layout: 18px top label strip, plot area, 16px bottom ticks.
                let top = 18.0f32;
                let bottom = h - 16.0;
                let plot_h = (bottom - top).max(1.0);
                let max = days.iter().map(|d| d.tokens).max().unwrap_or(1).max(1) as f32;

                // Max label (top-left) + faint mid gridline.
                ctx.draw_text(
                    &fmt::tokens_exact(max as u64),
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
                // GTT_TIPTEST=<idx> forces the tooltip in test builds — injected
                // pointer input never reaches WinUI3's content island, so this is
                // the screenshot-verifiable path for the popup itself.
                let tip = shared
                    .tip
                    .get()
                    .or_else(|| {
                        std::env::var("GTT_TIPTEST")
                            .ok()
                            .and_then(|v| v.parse::<usize>().ok())
                    })
                    .filter(|&i| i <= last);
                shared.width.set(w);
                shared.count.set(days.len());
                for (i, d) in days.iter().enumerate() {
                    let bh = ((d.tokens as f32) / max * plot_h).max(if d.tokens > 0 {
                        3.0
                    } else {
                        1.5
                    });
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
                        // Hover detail top-right: "MM-DD · 1,234 tok".
                        ctx.draw_text(
                            &tf!(
                                "{} · {} tok",
                                d.date.get(5..10).unwrap_or(&d.date),
                                fmt::tokens_exact(d.tokens)
                            ),
                            &tf_r,
                            &Rect::new(w - 220.0, 0.0, w, top),
                            &ink,
                        );
                    }
                }

                // Delayed tooltip card — drawn last so it floats above the plot.
                if let Some(i) = tip {
                    let d = &days[i];
                    let day_label = if d.date.len() > 10 {
                        d.date.clone()
                    } else {
                        d.date.get(5..10).unwrap_or(&d.date).to_string()
                    };
                    let mut lines: Vec<String> = vec![
                        tf!(
                            "{} tok · {}",
                            fmt::tokens_exact(d.tokens),
                            fmt::usd(d.cost_usd)
                        ),
                        tf!("{} 事件", fmt::tokens_exact(d.events)),
                    ];
                    for (m, t) in &d.top {
                        lines.push(format!(
                            "{}  {}",
                            if m.chars().count() > 20 {
                                tf!("{}…", m.chars().take(19).collect::<String>())
                            } else {
                                m.clone()
                            },
                            fmt::tokens_exact(*t)
                        ));
                    }
                    let line_h = label_pt + 4.0;
                    let pw = 216.0f32;
                    let ph = 26.0 + lines.len() as f32 * line_h + 10.0;
                    let px = (slot * i as f32 + slot * 0.5 - pw * 0.5)
                        .clamp(4.0, (w - pw - 4.0).max(4.0));
                    let py = top + 2.0;
                    let panel = windows_canvas::RoundedRect::new(
                        Rect::new(px, py, px + pw, py + ph),
                        7.0,
                        7.0,
                    );
                    // Near-opaque dark card (Fluent tooltip idiom; reads on both themes).
                    let bg = ctx.create_solid_brush(ColorF::from_rgba8(28, 28, 30, 242))?;
                    let frame = ctx.create_solid_brush(ColorF::from_rgba8(255, 255, 255, 36))?;
                    let head = ctx.create_solid_brush(accent)?;
                    let body = ctx.create_solid_brush(ColorF::from_rgba8(235, 235, 235, 255))?;
                    ctx.fill_rounded_rect(&panel, &bg);
                    ctx.draw_rounded_rect(&panel, &frame, 1.0);
                    ctx.draw_text(
                        &day_label,
                        &tf,
                        &Rect::new(px + 10.0, py + 7.0, px + pw - 10.0, py + 7.0 + line_h),
                        &head,
                    );
                    for (li, l) in lines.iter().enumerate() {
                        let y = py + 7.0 + (li + 1) as f32 * line_h;
                        ctx.draw_text(
                            l,
                            &tf,
                            &Rect::new(px + 10.0, y, px + pw - 10.0, y + line_h),
                            &body,
                        );
                    }
                }

                // Sparse date ticks: first / last day (MM-DD tail of ISO date).
                let tick = |d: &str| d.get(5..10).unwrap_or(d).to_string();
                ctx.draw_text(
                    &tick(&days[0].date),
                    &tf,
                    &Rect::new(0.0, bottom + 2.0, 80.0, h),
                    &ink,
                );
                ctx.draw_text(
                    &tick(&days[last].date),
                    &tf_r,
                    &Rect::new(w - 80.0, bottom + 2.0, w, h),
                    &ink,
                );
                Ok(())
            }),
        ))
}

/// Canvas slot content. Every `canvas_invalidated` builds its *own* GPU device
/// (D3D11 + D2D) on its first layout — ~15ms each on the UI thread, and the
/// library offers no shared-device variant for demand canvases. Overview has
/// five, so mounting them mid-slide was the ~85ms freeze on every flight
/// to/from it. While `defer` (slide in flight, or later in the post-settle
/// stagger) the slot holds an empty placeholder inside the same sized Border;
/// a later frame swaps the real canvas in. Building the canvas `View` itself
/// is cheap — the device only appears once it is mounted.
fn defer_slot(defer: bool, canvas: View) -> View {
    if defer { Border::new().into() } else { canvas }
}

/// Fixed palette for pie slices 1.. (slice 0 always uses the live accent so
/// the dominant share reads in the brand color). Hues chosen to stay legible
/// on both light and dark skins; the gray tail usually lands on 其他.
pub const SLICE_PALETTE: [(u8, u8, u8); 7] = [
    (0x10, 0xa8, 0x74),
    (0xe8, 0x85, 0x3d),
    (0x8b, 0x6f, 0xd8),
    (0xd8, 0x4d, 0x5b),
    (0x5b, 0xb8, 0xd8),
    (0xd8, 0xb8, 0x4d),
    (0x9e, 0x9e, 0x9e),
];

/// XAML-side swatch for legend rows — same index rule as the D2D slice fill.
pub fn slice_brush(theme: &Theme, i: usize) -> Brush {
    if i == 0 {
        return theme.accent;
    }
    let (r, g, b) = SLICE_PALETTE[(i - 1) % SLICE_PALETTE.len()];
    Brush::Solid(Color::rgb(r, g, b))
}

/// Donut canvas — annular sectors approximated by polygon paths (~3° steps;
/// a real arc primitive isn't exposed on 0.100). The ring parks in the left
/// h×h square of a wider box so the hover bubble has room beside it;
/// `slices` arrive pre-folded (top-N + 其他); `center` is the label in the hole.
/// Each donut gets its own Invalidator — sharing one across canvases draws
/// only the first (demand canvases attach once per render anyway).
/// Everything a share-donut needs beyond `theme`/`ctx` — bundles the cell
/// data with its hover wiring so `donut`/`donut_cell` stay under the arg cap.
pub struct DonutSpec<'a> {
    /// Pre-folded slices (top-N + 其他), already zero-filtered.
    pub slices: &'a [(String, f64)],
    /// Label inside the hole (usually the formatted total).
    pub center: String,
    /// Raw-value formatter for legend + hover detail tails.
    pub fmt_v: fn(f64) -> String,
    /// Column index — the Msg key matching `Shell.donuts[key]`.
    pub key: u8,
    /// Hover state + repaint handle owned by `Shell`.
    pub handle: &'a DonutHandle,
    /// Not this donut's turn to mount yet — placeholder (see `defer_slot`).
    pub defer: bool,
}

fn donut(theme: &Theme, spec: DonutSpec<'_>, ctx: &mut ViewContext<Shell>) -> View {
    let DonutSpec {
        slices,
        center,
        fmt_v,
        key,
        handle,
        defer,
    } = spec;
    let slices: Vec<(String, f64)> = slices.to_vec();
    let total: f64 = slices.iter().map(|s| s.1).sum();
    let accent = theme.accent_cf;
    let subtle = theme.subtle_cf;
    // SwapChain pixels composite straight onto the window surface — a
    // TRANSPARENT clear shows the page, not the card beneath. Painting the
    // card's own fill is what makes the canvas indistinguishable from it.
    let card_bg = theme.card_cf;
    let family = theme.font_family.clone();
    let body_pt = theme.body_size as f32;
    let label_pt = theme.label_size as f32;
    let shared = handle.shared.clone();
    // Angular hit-test mirrors the draw geometry — the element is fixed-size
    // so pointer-local coords map straight onto the drawn ring.
    let hit_vals: Vec<f64> = slices.iter().map(|s| s.1).collect();
    // GTT_DONUTTEST=<col>,<idx> forces a hover in test builds — injected
    // pointer input never reaches WinUI3's content island (same workaround
    // as the trend tooltip's GTT_TIPTEST).
    let hover_test: Option<(u8, usize)> = std::env::var("GTT_DONUTTEST").ok().and_then(|v| {
        let (c, i) = v.split_once(',')?;
        Some((c.trim().parse().ok()?, i.trim().parse().ok()?))
    });
    Border::new()
        // Wider than the ring: the band right of it hosts the hover bubble.
        .width(200.0)
        .height(132.0)
        // Left edge aligns with the legend rows below — centering floated
        // the ring right of the text column.
        .horizontal_alignment(HorizontalAlignment::Left)
        // Transparent (not null) Background keeps the canvas hit-testable.
        .background(Brush::Solid(Color::argb(0, 0, 0, 0)))
        .on_pointer_moved(ctx.callback(move |e: PointerEventInfo| {
            Msg::DonutHover(key, donut_hit(e.x, e.y, 200.0, 132.0, &hit_vals))
        }))
        .on_pointer_exited(ctx.callback(move |_| Msg::DonutHover(key, None)))
        .content(defer_slot(
            defer,
            windows_canvas::canvas_invalidated(&handle.inv, move |ctx| {
                use windows_canvas::{
                    ColorF, ParagraphAlignment, PathBuilder, Rect, TextAlignment, TextFormat,
                    Vector2,
                };
                ctx.clear(card_bg);
                let (w, h) = (ctx.width, ctx.height);
                if w < 40.0 || total <= 0.0 {
                    return Ok(());
                }
                // Ring parks inside the left h×h square — `donut_geom` is the
                // single source the pointer hit-test mirrors.
                let (cx, cy, r_out, r_in) = donut_geom(w as f64, h as f64);
                let (cx, cy, r_out, r_in) = (cx as f32, cy as f32, r_out as f32, r_in as f32);
                let hovered = hover_test
                    .filter(|(c, _)| *c == key)
                    .map(|(_, i)| i)
                    .or_else(|| shared.hover.get())
                    .filter(|&i| i < slices.len());
                let gap = if slices.len() > 1 { 0.016f32 } else { 0.0 };
                let mut a = -std::f32::consts::FRAC_PI_2;
                let mut hover_am = None;
                for (i, (_, v)) in slices.iter().enumerate() {
                    let span = (*v / total).max(0.0) as f32 * std::f32::consts::TAU;
                    if hovered == Some(i) {
                        hover_am = Some(a + span * 0.5);
                    }
                    let a1 = a + span;
                    let (a0, a1c) = (a + gap, (a1 - gap).max(a + gap));
                    if a1c > a0 {
                        // Hovered slice pops +2.5px; siblings dim when a hover
                        // is active so the focus reads instantly.
                        let lit = hovered == Some(i);
                        let ro = if lit { r_out + 2.5 } else { r_out };
                        let n =
                            (((a1c - a0) / (std::f32::consts::TAU / 120.0)).ceil() as usize).max(2);
                        let mut pts = Vec::with_capacity(2 * (n + 1));
                        for k in 0..=n {
                            let t = a0 + (a1c - a0) * k as f32 / n as f32;
                            pts.push(Vector2::new(cx + ro * t.cos(), cy + ro * t.sin()));
                        }
                        for k in (0..=n).rev() {
                            let t = a0 + (a1c - a0) * k as f32 / n as f32;
                            pts.push(Vector2::new(cx + r_in * t.cos(), cy + r_in * t.sin()));
                        }
                        let path = PathBuilder::new(ctx.device())?.polygon(pts)?;
                        let mut color = if i == 0 {
                            accent
                        } else {
                            let (r, g, b) = SLICE_PALETTE[(i - 1) % SLICE_PALETTE.len()];
                            ColorF::from_rgb8(r, g, b)
                        };
                        if hovered.is_some() && !lit {
                            color.a *= 0.32;
                        }
                        ctx.fill_path(&path, &ctx.create_solid_brush(color)?);
                    }
                    a = a1;
                }
                let ink = ctx.create_solid_brush(subtle)?;
                // Center always shows the total — hover detail lives in the
                // bubble, which has room the hole never did.
                let tf = TextFormat::new_bold(&family, body_pt)?
                    .with_alignment(TextAlignment::Center)
                    .with_paragraph_alignment(ParagraphAlignment::Center);
                ctx.draw_text(
                    &center,
                    &tf,
                    &Rect::new(cx - r_in + 2.0, cy - r_in, cx + r_in - 2.0, cy + r_in),
                    &ink,
                );
                // Hover bubble anchored on the ring's outer edge at the slice
                // mid-angle, clamped inside the canvas — same Fluent dark
                // card idiom as the trend tooltip.
                if let Some(i) = hovered {
                    let (name, v) = &slices[i];
                    let am = hover_am.unwrap_or(-std::f32::consts::FRAC_PI_2);
                    let ar = r_out + 16.0;
                    let (bx, by) = (cx + ar * am.cos(), cy + ar * am.sin());
                    let (pw, ph) = (150.0f32, 58.0f32);
                    let px = (bx - pw * 0.5).clamp(4.0, (w - pw - 4.0).max(4.0));
                    let py = (by - ph * 0.5).clamp(4.0, (h - ph - 4.0).max(4.0));
                    let panel = windows_canvas::RoundedRect::new(
                        Rect::new(px, py, px + pw, py + ph),
                        7.0,
                        7.0,
                    );
                    let bg = ctx.create_solid_brush(ColorF::from_rgba8(28, 28, 30, 242))?;
                    let frame = ctx.create_solid_brush(ColorF::from_rgba8(255, 255, 255, 36))?;
                    ctx.fill_rounded_rect(&panel, &bg);
                    ctx.draw_rounded_rect(&panel, &frame, 1.0);
                    let short: String = if name.chars().count() > 18 {
                        crate::tf!("{}…", name.chars().take(17).collect::<String>())
                    } else {
                        name.clone()
                    };
                    let detail = format!("{:.1}% · {}", v / total * 100.0, fmt_v(*v));
                    let rank = crate::tf!("第 {} / {} 项", i + 1, slices.len());
                    let tf_name = TextFormat::new_bold(&family, label_pt + 1.0)?;
                    let tf_val = TextFormat::new(&family, label_pt)?;
                    let name_ink = ctx.create_solid_brush(accent)?;
                    let body_ink =
                        ctx.create_solid_brush(ColorF::from_rgba8(235, 235, 235, 255))?;
                    let lh = label_pt + 4.0;
                    ctx.draw_text(
                        &short,
                        &tf_name,
                        &Rect::new(px + 10.0, py + 6.0, px + pw - 10.0, py + 6.0 + lh),
                        &name_ink,
                    );
                    ctx.draw_text(
                        &detail,
                        &tf_val,
                        &Rect::new(
                            px + 10.0,
                            py + 6.0 + lh,
                            px + pw - 10.0,
                            py + 6.0 + lh * 2.0,
                        ),
                        &body_ink,
                    );
                    ctx.draw_text(
                        &rank,
                        &tf_val,
                        &Rect::new(
                            px + 10.0,
                            py + 6.0 + lh * 2.0,
                            px + pw - 10.0,
                            py + 6.0 + lh * 3.0,
                        ),
                        &ink,
                    );
                }
                Ok(())
            }),
        ))
}

/// Ring geometry inside the `w×h` canvas — the ring occupies the left
/// h×h square so the right band is free for the hover bubble. Shared by the
/// draw pass and the pointer hit-test so they can never disagree.
fn donut_geom(w: f64, h: f64) -> (f64, f64, f64, f64) {
    let side = w.min(h);
    let cx = side * 0.5;
    let cy = h * 0.5;
    let r_out = (side * 0.5 - 4.0).max(1.0);
    let r_in = r_out * 0.62;
    (cx, cy, r_out, r_in)
}

/// Ring hit-test: pointer-local (x,y) in the `w×h` donut box — outside the
/// ring or in the hole → `None`; inside a slice's angular span → its index.
/// Zero angle is 12 o'clock going clockwise, matching the draw pass.
fn donut_hit(x: f64, y: f64, w: f64, h: f64, vals: &[f64]) -> Option<usize> {
    let (cx, cy, r_out, r_in) = donut_geom(w, h);
    let (dx, dy) = (x - cx, y - cy);
    let dist = (dx * dx + dy * dy).sqrt();
    let total: f64 = vals.iter().sum();
    if dist < r_in || dist > r_out || total <= 0.0 {
        return None;
    }
    let a = (dy.atan2(dx) + std::f64::consts::FRAC_PI_2).rem_euclid(std::f64::consts::TAU);
    let mut acc = 0.0;
    for (i, v) in vals.iter().enumerate() {
        acc += (v / total).max(0.0) * std::f64::consts::TAU;
        if a < acc {
            return Some(i);
        }
    }
    // Float tail: the last sliver owns the rounding remainder.
    vals.len()
        .checked_sub(1)
        .filter(|_| a < std::f64::consts::TAU)
}

/// One share column: title + donut + compact legend (`■ name … 42.0% · $x`).
/// `spec` bundles slices/hole label/formatter/hover wiring.
/// Zero-total data renders an honest empty note.
pub fn donut_cell(
    theme: &Theme,
    title: String,
    spec: DonutSpec<'_>,
    ctx: &mut ViewContext<Shell>,
) -> View {
    let DonutSpec {
        slices,
        center,
        fmt_v,
        key,
        handle,
        defer,
    } = spec;
    let total: f64 = slices.iter().map(|s| s.1).sum();
    let body: View = if total <= 0.0 {
        TextBlock::new()
            .text(t!("暂无数据"))
            .font_size(theme.label_size)
            .foreground(theme.subtle)
            .height(132.0)
            .into()
    } else {
        let mut legend: Vec<View> = Vec::with_capacity(slices.len());
        for (i, (name, v)) in slices.iter().enumerate() {
            let pct = format!("{:.1}", v / total * 100.0);
            let name_short: String = if name.chars().count() > 16 {
                crate::tf!("{}…", name.chars().take(15).collect::<String>())
            } else {
                name.clone()
            };
            let swatch: View = Border::new()
                .width(9.0)
                .height(9.0)
                .corner_radius(CornerRadius::uniform(2.0))
                .background(slice_brush(theme, i))
                .vertical_alignment(VerticalAlignment::Center)
                .into();
            legend.push(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(6.0)
                    .children([
                        swatch,
                        TextBlock::new()
                            .text(name_short)
                            .font_size(theme.label_size)
                            .vertical_alignment(VerticalAlignment::Center)
                            .into(),
                        TextBlock::new()
                            .text(format!("{pct}% · {}", fmt_v(*v)))
                            .font_size(theme.label_size)
                            .foreground(theme.subtle)
                            .vertical_alignment(VerticalAlignment::Center)
                            .into(),
                    ]),
            );
        }
        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(6.0)
            .children([
                donut(
                    theme,
                    DonutSpec {
                        slices,
                        center,
                        fmt_v,
                        key,
                        handle,
                        defer,
                    },
                    ctx,
                ),
                StackPanel::new()
                    .orientation(Orientation::Vertical)
                    .spacing(3.0)
                    .keyed_children(crate::pages::keyed(legend)),
            ])
    };
    StackPanel::new()
        .orientation(Orientation::Vertical)
        .spacing(8.0)
        .children((
            TextBlock::new()
                .text(title)
                .font_size(theme.label_size)
                .foreground(theme.subtle),
            body,
        ))
}

/// Two-column row; right-aligned meta. Hairline divider under the row when the
/// theme enables line separators.
pub fn key_value_row(theme: &Theme, left: String, right: String) -> View {
    let divider: View = if theme.line_separators {
        Border::new().height(1.0).background(theme.divider).into()
    } else {
        Border::new().height(0.0).into()
    };
    Border::new().padding(Thickness::xy(0.0, 3.0)).content(
        StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(0.0)
            .children((
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto])
                    .children([
                        Border::new()
                            .grid_column(0)
                            .content(TextBlock::new().text(left).font_size(theme.body_size)),
                        Border::new().grid_column(1).content(
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

#[cfg(test)]
mod tests {
    use super::donut_hit;

    // 200×132 donut box → ring parks in the left 132×132 square:
    // r_out=62, r_in≈38.4, center (66,66); x>132 is bubble territory.
    #[test]
    fn hit_resolves_ring_to_slice_index() {
        let vals = [50.0, 30.0, 20.0];
        // Right of center: slice 0 owns the first 50% of the circle.
        assert_eq!(donut_hit(120.0, 66.0, 200.0, 132.0, &vals), Some(0));
        // Just clockwise of 12 o'clock → slice 0; just counterclockwise →
        // last slice owns the rounding tail.
        assert_eq!(donut_hit(67.0, 8.0, 200.0, 132.0, &vals), Some(0));
        assert_eq!(donut_hit(65.0, 8.0, 200.0, 132.0, &vals), Some(2));
        // Bottom-left: slice 0 ended at 6 o'clock → slice 1.
        assert_eq!(donut_hit(30.0, 110.0, 200.0, 132.0, &vals), Some(1));
    }

    #[test]
    fn hit_rejects_hole_outside_and_empty() {
        let vals = [50.0, 50.0];
        assert_eq!(donut_hit(66.0, 66.0, 200.0, 132.0, &vals), None); // hole
        assert_eq!(donut_hit(190.0, 66.0, 200.0, 132.0, &vals), None); // right band
        assert_eq!(donut_hit(5.0, 5.0, 200.0, 132.0, &vals), None); // outside
        assert_eq!(donut_hit(120.0, 66.0, 200.0, 132.0, &[0.0, 0.0]), None); // no data
    }
}

#[cfg(test)]
mod nav_clock_tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, Instant};

    /// A flight whose previous tick was handled `since_tick` ago and which
    /// started `since_start` ago (wall clock).
    fn anim(since_tick: Duration, since_start: Duration) -> NavAnim {
        let now = Instant::now();
        NavAnim {
            from: crate::Page::Overview,
            dir: 1.0,
            t0: now.checked_sub(since_start).expect("instant underflow"),
            clock: Cell::new(0.0),
            cache: RefCell::new(None),
            frames: Cell::new(1),
            last_tick: Cell::new(now.checked_sub(since_tick).expect("instant underflow")),
            first_gap_ms: Cell::new(0),
            max_gap_ms: Cell::new(0),
            max_view_ms: Cell::new(0),
        }
    }

    #[test]
    fn a_ui_stall_moves_the_clock_by_one_capped_step_only() {
        // 400ms hitch (mounting a page, creating a GPU device…) must not
        // leap the springs forward by 400ms.
        let a = anim(Duration::from_millis(400), Duration::from_millis(400));
        a.advance();
        assert!(
            (a.clock.get() - NavAnim::MAX_DT).abs() < 1e-9,
            "{}",
            a.clock.get()
        );
        assert!(a.max_gap_ms.get() >= 400);
    }

    #[test]
    fn a_normal_frame_gap_stays_real_time() {
        let a = anim(Duration::from_millis(12), Duration::from_millis(12));
        a.advance();
        let c = a.clock.get();
        assert!((0.012..NavAnim::MAX_DT).contains(&c), "{c}");
    }

    #[test]
    fn done_follows_animation_time_not_just_wall_time() {
        let a = anim(Duration::ZERO, Duration::from_millis(100));
        assert!(!a.done());
        a.clock.set(NavAnim::DONE_S);
        assert!(a.done());
    }

    #[test]
    fn wall_clock_backstop_ends_a_starved_flight() {
        // Every tick clamped and the machine crawling: animation time never
        // reaches DONE_S, but the flight must still end.
        let a = anim(
            Duration::ZERO,
            Duration::from_millis(NavAnim::HARD_CAP_MS as u64 + 50),
        );
        assert!(a.clock.get() < NavAnim::DONE_S);
        assert!(a.done());
    }
}
