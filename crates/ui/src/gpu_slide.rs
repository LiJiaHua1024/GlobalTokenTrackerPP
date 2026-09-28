//! Compositor-driven page slide.
//!
//! The slide used to be layout-driven: every ~9ms a tick message rebuilt the
//! view, diffed it and re-arranged margins — all UI-thread CPU work. Here the
//! motion is handed to the composition engine instead: each moving XAML
//! element's own `Visual` gets a pre-sampled key-frame animation on
//! `Offset.X`, and from then on DWM/GPU runs it with zero UI-thread cost —
//! it keeps gliding even while the UI thread is busy mounting content.
//!
//! Reaching an element's Visual: reactor exposes a Composition host on `Grid`
//! (`observe_composition_host`) that reports the window's lifted `Compositor`
//! and lets us attach a child visual (`request_set_child_visual`). Once the
//! first composition commit lands, that child's `parent()` *is* the host
//! element's own visual (verified: moving it moves the whole XAML subtree).
//!
//! Every `windows-composition` wrapper `unwrap()`s its COM calls, so all of
//! them run under `guarded`: a failure disables the GPU slide for the session
//! and navigation degrades to an instant page switch instead of a crash.
use crate::Page;
use crate::diag;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_composition::{SpriteVisual, Visual};
use windows_reactor::*;

/// Page blocks (top-level cards) that get their own spring; the rest ride
/// the page layer.
pub const MAX_SLIDE: usize = 10;
/// Time the springs take — every one has settled by then.
pub const FLIGHT_S: f64 = 0.6;
/// Overview charts start mounting this long after the animations start —
/// the page has essentially landed by then (critically damped spring).
pub const CHARTS_AFTER_MS: u64 = 300;
/// Flight ends this long after the springs land.
pub const SETTLE_AFTER_S: f64 = FLIGHT_S + 0.06;
const _: () = assert!(SETTLE_AFTER_S > FLIGHT_S);

static DISABLED: AtomicBool = AtomicBool::new(false);

/// `GTT_NO_GPU_SLIDE=1` (diagnostics) or a caught composition failure turns
/// the slide off; navigation then switches pages instantly.
pub fn enabled() -> bool {
    static ENV_OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let env_off = *ENV_OFF.get_or_init(|| std::env::var_os("GTT_NO_GPU_SLIDE").is_some());
    !env_off && !DISABLED.load(Ordering::Relaxed)
}

/// Run a composition call; a panic inside the (unwrap-happy) wrappers
/// disables the slide for the rest of the session instead of unwinding
/// through the UI.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(_) => {
            DISABLED.store(true, Ordering::Relaxed);
            diag!("[gpu] composition call panicked — GPU slide disabled");
            None
        }
    }
}

// ------------------------------------------------------------------ host

/// One XAML `Grid` whose Visual we can animate on the compositor. Cheap to
/// clone (shared state); the `ElementRef` is bound with `Grid::element_ref`.
#[derive(Clone, Default)]
pub struct LayerHost {
    pub r: ElementRef<Grid>,
    st: Rc<RefCell<HostState>>,
}

#[derive(Default)]
struct HostState {
    /// Keeps the child visual (and so the attachment) alive.
    sprite: Option<SpriteVisual>,
    /// The element's own visual, resolved lazily (see module docs).
    visual: Option<Visual>,
}

impl LayerHost {
    /// Observe the composition host (idempotent per `key`) and attach the
    /// probe child visual on each new binding.
    pub fn attach(&self, ctx: &mut ViewContext<crate::Shell>, key: &'static str) {
        if !enabled() {
            return;
        }
        ctx.use_effect(key, (), {
            let host = self.clone();
            move || {
                let h = host.clone();
                let obs = host.r.observe_composition_host(move |ev| {
                    let CompositionHostEvent::Ready { compositor, .. } = ev else {
                        return;
                    };
                    let made = guarded(|| {
                        let comp = windows_composition::Compositor::from_host(compositor).ok()?;
                        let sprite = comp.create_sprite_visual();
                        sprite.set_size(1.0, 1.0);
                        Some(sprite)
                    })
                    .flatten();
                    let Some(sprite) = made else {
                        return;
                    };
                    let raw = sprite.as_raw();
                    // A fresh binding → forget the old visual, attach again.
                    *h.st.borrow_mut() = HostState {
                        sprite: Some(sprite),
                        visual: None,
                    };
                    let bound = h.r.request_set_child_visual(Some(raw.into()), |res| {
                        if let Err(e) = res {
                            diag!("[gpu] set_child_visual failed: {e:?}");
                        }
                    });
                    if !bound {
                        diag!("[gpu] host element not bound — child visual not attached");
                    }
                });
                Some(Box::new(move || drop(obs)))
            }
        });
    }

    /// The host element's Visual — `None` until the first commit after the
    /// child was attached.
    pub fn visual(&self) -> Option<Visual> {
        let mut st = self.st.borrow_mut();
        if st.visual.is_none() {
            let resolved = st
                .sprite
                .as_ref()
                .and_then(|s| guarded(|| s.parent().map(|p| (*p).clone())).flatten());
            st.visual = resolved;
        }
        st.visual.clone()
    }

    /// Laid-out width of the element in DIPs.
    pub fn width(&self) -> Option<f64> {
        let v = self.visual()?;
        guarded(|| f64::from(v.size().x))
    }

    /// Start a compositor animation of `Offset.X` through `frames`
    /// (`(progress, x)`, see [`frames`]) over `total`. Only the X component
    /// is animated, so layout keeps owning Y.
    pub fn slide_x(&self, frames: &[(f32, f32)], total: Duration) -> bool {
        let Some(v) = self.visual() else {
            return false;
        };
        guarded(|| {
            let comp = v.compositor();
            let linear = comp.create_linear_easing_function();
            let anim = comp.create_scalar_key_frame_animation();
            for &(p, x) in frames {
                anim.insert_key_frame_with_easing(p, x, &linear);
            }
            anim.set_duration(total);
            v.start_animation("Offset.X", &anim);
            true
        })
        .unwrap_or(false)
    }

    /// Drop the cached element visual so it is resolved again. The child
    /// visual stays: if the element persists (layers swap roles under the
    /// same key, so nothing remounts and no new `Ready` fires) it is still
    /// attached and resolves again; if the element was replaced, its `Ready`
    /// swaps in a fresh child anyway. (Clearing the child here made a quick
    /// second navigation permanently lose its visuals.)
    pub fn reset(&self) {
        self.st.borrow_mut().visual = None;
    }

    /// Replace any running slide with a 1ms move to X=0 (rest) — used when a
    /// new navigation interrupts a flight.
    pub fn snap_home(&self) {
        self.slide_x(&[(1.0, 0.0)], Duration::from_millis(1));
    }
}

// ---------------------------------------------------------------- flight

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// New page mounted but hidden (opacity 0 — not a fade, just "not until
    /// ready", so it never flashes at rest before the animation starts);
    /// waiting for the composition visuals to resolve.
    Prepare,
    /// Compositor animations are running and the page is revealed. Layout
    /// never changes during a flight: the compositor value overrides the
    /// layout Offset, and XAML has no reason to rewrite it (an Offset write
    /// mid-flight knocks the animation out — seen when margins were reset).
    Running,
}

/// One page-switch in progress.
pub struct Flight {
    pub id: u64,
    pub from: Page,
    /// +1 forward (new page enters from the right), −1 backward.
    pub dir: f64,
    /// Page-layer width the slide travels (DIPs).
    pub w: f64,
    pub phase: Phase,
    /// How many Overview charts the outgoing page had mounted when the flight
    /// began (its canvases must not be touched mid-flight).
    pub from_ready: usize,
    /// `NavGo` polls spent waiting for visuals.
    pub waited: u8,
    pub t0: std::time::Instant,
}

/// What `slide_children` needs to wrap a page's blocks: the hosts bound to
/// its first `MAX_SLIDE` block wrappers (one set per page layer).
pub struct Slide<'a> {
    pub hosts: &'a [LayerHost; MAX_SLIDE],
}

/// Extra displacement of block `i` beyond the page layer's own travel —
/// later blocks start farther out.
pub fn block_extra(i: usize) -> f64 {
    34.0 + 10.0 * i as f64
}

// --------------------------------------------------------------- springs

/// Underdamped spring position at `t` (secs): x(0)=x0, v(0)=0, target 0.
/// ζ<1 gives the slight overshoot that reads as "springy"; ζ≥1 uses the
/// critically-damped closed form.
pub fn spring(t: f64, x0: f64, zeta: f64, omega: f64) -> f64 {
    if t <= 0.0 {
        return x0;
    }
    if zeta >= 1.0 {
        return x0 * (-omega * t).exp() * (1.0 + omega * t);
    }
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    x0 * (-zeta * omega * t).exp() * ((wd * t).cos() + (zeta * omega / wd) * (wd * t).sin())
}

/// Pre-sample a spring `from → to` into compositor key frames
/// `(progress, x)` over [`FLIGHT_S`]: hold `from` for `delay`, then run the
/// spring. Between samples the compositor interpolates linearly (12ms apart
/// — well below what the eye resolves). The last sample lands exactly on
/// `to`, so the animation ends where layout rests: no snap when it hands back.
pub fn frames(from: f64, to: f64, zeta: f64, omega: f64, delay: f64) -> Vec<(f32, f32)> {
    const SAMPLES: usize = 48;
    let span = (FLIGHT_S - delay).max(0.05);
    let total = delay + span;
    let mut out = Vec::with_capacity(SAMPLES + 2);
    out.push((0.0, from as f32));
    if delay > 0.0 {
        out.push(((delay / total) as f32, from as f32));
    }
    for k in 1..=SAMPLES {
        let t = span * k as f64 / SAMPLES as f64;
        let x = if k == SAMPLES {
            to
        } else {
            to + (from - to) * spring(t, 1.0, zeta, omega)
        };
        out.push((((delay + t) / total) as f32, x as f32));
    }
    out
}

/// Page layer travel: critically damped so the two pages stay exactly one
/// page-width apart (no overshoot gap or overlap).
const LAYER_OMEGA: f64 = 16.0;

/// Entering layer: `dir·w → 0`.
pub fn enter_frames(dir: f64, w: f64) -> Vec<(f32, f32)> {
    frames(dir * w, 0.0, 1.0, LAYER_OMEGA, 0.0)
}

/// Leaving layer: `0 → −dir·w` on the same spring — pushed out by the
/// entering page, so nothing ever overlaps and nothing needs to fade.
pub fn leave_frames(dir: f64, w: f64) -> Vec<(f32, f32)> {
    frames(0.0, -dir * w, 1.0, LAYER_OMEGA, 0.0)
}

/// Per-block spring: each block has its own start delay, travel, stiffness
/// and damping — later blocks start later, farther out, and ride a softer
/// spring, giving the cascade.
pub fn block_frames(i: usize, dir: f64) -> Vec<(f32, f32)> {
    let delay = 0.012 * i as f64;
    let k = (180.0 - 13.0 * i as f64).max(60.0);
    let zeta = 0.82 + 0.008 * i as f64;
    frames(dir * block_extra(i), 0.0, zeta, k.sqrt(), delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_start_at_from_and_land_exactly_on_to() {
        for f in [
            enter_frames(1.0, 1180.0),
            leave_frames(1.0, 1180.0),
            block_frames(3, -1.0),
        ] {
            assert_eq!(f.first().map(|p| p.0), Some(0.0));
            let last = *f.last().unwrap();
            assert_eq!(last.0, 1.0);
        }
        assert_eq!(enter_frames(1.0, 1180.0).first().unwrap().1, 1180.0);
        assert_eq!(enter_frames(1.0, 1180.0).last().unwrap().1, 0.0);
        assert_eq!(leave_frames(1.0, 1180.0).first().unwrap().1, 0.0);
        assert_eq!(leave_frames(1.0, 1180.0).last().unwrap().1, -1180.0);
        assert_eq!(leave_frames(-1.0, 1180.0).last().unwrap().1, 1180.0);
    }

    #[test]
    fn progress_is_strictly_increasing() {
        for f in [enter_frames(1.0, 900.0), block_frames(9, 1.0)] {
            assert!(f.windows(2).all(|w| w[0].0 < w[1].0), "{f:?}");
        }
    }

    #[test]
    fn a_block_holds_its_start_position_through_its_delay() {
        // block 5 waits 60ms of the 600ms flight before it moves
        let f = block_frames(5, 1.0);
        let start = f[0].1;
        let held = f[1];
        assert_eq!(held.1, start);
        assert!((held.0 - 0.1).abs() < 1e-6, "{}", held.0);
    }

    #[test]
    fn push_keeps_the_pages_exactly_one_width_apart() {
        // enter x − leave x == dir·w at every sample (same spring, same
        // sample times) → the pages never overlap or open a gap.
        let (w, dir) = (1180.0_f64, 1.0);
        let (e, l) = (enter_frames(dir, w), leave_frames(dir, w));
        assert_eq!(e.len(), l.len());
        for (a, b) in e.iter().zip(&l) {
            assert!((f64::from(a.1 - b.1) - dir * w).abs() < 0.05, "{a:?} {b:?}");
        }
    }

    #[test]
    fn critically_damped_layer_never_overshoots() {
        assert!(
            enter_frames(1.0, 1000.0)
                .iter()
                .all(|&(_, x)| (0.0..=1000.0).contains(&x))
        );
    }
}
