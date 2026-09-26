//! codeledger-ui — WinUI 3 shell via windows-reactor.
//! Dumb renderer over core ViewModels; theme + layout are data (`ui.json`
//! next to ledger.db), so skins / widget ordering survive without recompiles.

mod config;
mod pages;
mod theme;
mod tray;
mod watch;
mod widgets;

use codeledger_core::adapters;
use codeledger_core::store::{default_db_path, EventRow, PriceRow, SourceHealth};
use codeledger_core::viewmodel::fmt;
use codeledger_core::viewmodel::Range;
use codeledger_core::{Engine, OverviewVm, Store};
use config::UiConfig;
use pages::*;
use std::path::PathBuf;
use theme::Theme;
use windows_reactor::*;

/// One background refresh produces this bundle (all Send-safe plain data).
pub struct Snapshot {
    pub vm: OverviewVm,
    pub detail: DetailBundle,
    pub sources: Vec<SourceHealth>,
    pub prices: Vec<PriceRow>,
    /// `prices` live-source sync timestamp (ms); `None` = seed only.
    pub prices_synced_at: Option<i64>,
    pub scan_ms: u128,
}

pub struct DetailBundle {
    pub rows: Vec<EventRow>,
    pub total: u64,
    pub page: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Detail,
    Quota,
    Sources,
    Prices,
}

pub struct Shell {
    snap: Option<Snapshot>,
    page: Page,
    scanning: bool,
    /// A filesystem event arrived while a scan was running — rescan when it ends.
    pending_rescan: bool,
    last_error: Option<String>,
    config: UiConfig,
    theme: Theme,
    editing: bool,
    /// Kept alive for the process lifetime; `!Send`, stays on the UI thread.
    tray: Option<tray_icon::TrayIcon>,
    /// Trend-chart hover state + repaint handle (shared with the D2D closure).
    trend: widgets::TrendHandle,
    /// Vendor quota channels poll at this cadence (network calls stay rare).
    quota_at: Option<std::time::Instant>,
    /// Overview statistics window (persisted in ui.json).
    range: Range,
}

pub enum Msg {
    Loaded(Box<Snapshot>),
    Failed(String),
    Tick,
    Rescan,
    WatchFired,
    Tray(tray::TrayAction),
    Nav(Option<String>),
    DetailPage(i64),
    DetailLoaded(Vec<EventRow>, u64, i64),
    ToggleEdit,
    MoveWidget(String, String, i32),
    HideWidget(String, String, bool),
    /// Pointer x (canvas-local DIPs) over the trend chart.
    TrendHover(f64),
    TrendLeave,
    /// Statistics range changed (label text from the selector).
    SetRange(String),
    /// Background quota poll finished (rows written, channel errors).
    QuotaDone(usize, Vec<String>),
}

const DETAIL_PAGE_SIZE: i64 = 200;
const REFRESH_SECS: u64 = 30;
/// Spec §6.9: quota polling is low-frequency by design.
const QUOTA_POLL_SECS: u64 = 30 * 60;

/// `CL_DEBUG=1` → diagnostic stderr (invisible for normal GUI launches).
pub(crate) fn diag_enabled() -> bool {
    std::env::var_os("CL_DEBUG").is_some()
}

macro_rules! diag {
    ($($t:tt)*) => {
        if crate::diag_enabled() {
            eprintln!($($t)*);
        }
    };
}
pub(crate) use diag;

fn db_path() -> PathBuf {
    default_db_path()
}

fn load_all(range: Range) -> Result<Snapshot, String> {
    let store = Store::open(&db_path()).map_err(|e| e.to_string())?;
    let engine = Engine::new(store).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let _ = engine.scan_once().map_err(|e| e.to_string());
    let scan_ms = t.elapsed().as_millis();
    // Price book self-heals: stale >24h → pull models.dev+LiteLLM, reprice
    // unpriced events. Network failure keeps the current book untouched.
    if codeledger_core::pricing::prices_stale(&engine.store).unwrap_or(false) {
        match codeledger_core::pricing::refresh(&engine.store) {
            Ok(r) => diag!(
                "[prices] synced: dev={} litellm={} repriced={}",
                r.models_dev, r.litellm, r.repriced
            ),
            Err(e) => diag!("[prices] refresh failed: {e}"),
        }
    }
    let vm = engine.store.overview(range).map_err(|e| e.to_string())?;
    let d = engine
        .store
        .detail(0, DETAIL_PAGE_SIZE)
        .map_err(|e| e.to_string())?;
    let sources = engine.store.source_health().map_err(|e| e.to_string())?;
    let prices = engine.store.price_rows(5000).map_err(|e| e.to_string())?;
    let prices_synced_at =
        codeledger_core::pricing::last_live_sync(&engine.store).unwrap_or(None);
    Ok(Snapshot {
        vm,
        detail: DetailBundle {
            rows: d.rows,
            total: d.total_events,
            page: 0,
        },
        sources,
        prices,
        prices_synced_at,
        scan_ms,
    })
}

fn arm_refresh(context: &ComponentContext<Shell>) {
    context.spawn_background(|_| {
        std::thread::sleep(std::time::Duration::from_secs(REFRESH_SECS));
        Msg::Tick
    });
}

/// Union of adapter watch roots that exist right now (tool absent → dir absent).
fn source_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for a in adapters::registry() {
        for r in a.watch_roots() {
            if r.is_dir() && !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

/// Live refresh: block on notify events, debounce, then report once.
fn arm_watcher(context: &ComponentContext<Shell>) {
    let roots = source_roots();
    if roots.is_empty() {
        return;
    }
    context.spawn_background(move |token| {
        diag!("[watch] armed on {} roots: {:?}", roots.len(), roots);
        if watch::wait_for_change(&roots, &token) {
            Msg::WatchFired
        } else {
            // Watch failed/cancelled — fall back to a slow poll so changes are
            // still picked up eventually.
            std::thread::sleep(std::time::Duration::from_secs(120));
            Msg::Tick
        }
    });
}

/// One blocking tray-event poll per arm; re-armed on every message.
fn arm_tray(context: &ComponentContext<Shell>) {
    context.spawn_background(|_| Msg::Tray(tray::next_action()));
}

impl Component for Shell {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let config = UiConfig::load();
        let range = Range::from_key(&config.range);
        context.spawn_background(move |_| match load_all(range) {
            Ok(s) => Msg::Loaded(Box::new(s)),
            Err(e) => Msg::Failed(e),
        });
        arm_watcher(context);
        let tray = tray::install();
        if tray.is_some() {
            arm_tray(context);
        }
        let theme = Theme::resolve(&config.theme);
        // OTLP receiver: dedicated blocking thread (never the reactor pool).
        // Port busy or CL_NO_OTEL → file-based sources only.
        let _otel = codeledger_core::otel::spawn(db_path());
        let page = match std::env::var("CL_PAGE").as_deref() {
            Ok("detail") => Page::Detail,
            Ok("quota") => Page::Quota,
            Ok("sources") => Page::Sources,
            Ok("prices") => Page::Prices,
            _ => Page::Overview,
        };
        Self {
            snap: None,
            page,
            scanning: true,
            pending_rescan: false,
            last_error: None,
            config,
            range,
            theme,
            editing: std::env::var("CL_EDIT").is_ok(),
            tray,
            trend: widgets::TrendHandle::default(),
            quota_at: None,
        }
    }

    fn update(&mut self, message: Msg, context: &ComponentContext<Self>) {
        match message {
            Msg::Loaded(s) => {
                diag!("[scan] loaded, pending_rescan={}", self.pending_rescan);
                self.snap = Some(*s);
                self.last_error = None;
                if let Some(tray) = &self.tray {
                    let total = self
                        .snap
                        .as_ref()
                        .map(|s| fmt::tokens_total(&s.vm.today))
                        .unwrap_or(0);
                    let _ = tray.set_tooltip(Some(format!(
                        "CodeLedger — 今日 {}",
                        fmt::tokens_exact(total)
                    )));
                }
                self.scanning = false;
                self.poll_quota_if_stale(context);
                if self.pending_rescan {
                    self.pending_rescan = false;
                    self.start_scan(context);
                } else {
                    arm_refresh(context);
                }
            }
            Msg::Failed(e) => {
                self.last_error = Some(e);
                self.scanning = false;
                if self.pending_rescan {
                    self.pending_rescan = false;
                    self.start_scan(context);
                } else {
                    arm_refresh(context);
                }
            }
            Msg::Tick | Msg::Rescan => {
                self.start_scan(context);
            }
            Msg::SetRange(label) => {
                let r = Range::from_label(&label);
                if r != self.range {
                    self.range = r;
                    self.config.range = r.key().to_string();
                    self.config.save();
                    // New aggregates needed — reload through the normal scan
                    // path (data hit is small; totals/by_app are indexed).
                    self.scanning = false;
                    self.start_scan(context);
                }
            }
            Msg::WatchFired => {
                diag!("[watch] fired, scanning={}", self.scanning);
                arm_watcher(context);
                if self.scanning {
                    self.pending_rescan = true;
                } else {
                    self.start_scan(context);
                }
            }
            Msg::Tray(action) => {
                match action {
                    tray::TrayAction::Focus => {
                        tray::focus_main_window();
                    }
                    tray::TrayAction::Hide => {
                        tray::hide_main_window();
                    }
                    tray::TrayAction::Quit => {
                        let _ = context.window().request_close();
                    }
                    tray::TrayAction::None => {}
                }
                if self.tray.is_some() {
                    arm_tray(context);
                }
            }
            Msg::Nav(tag) => {
                self.page = match tag.as_deref() {
                    Some("明细") | Some("detail") => Page::Detail,
                    Some("配额") | Some("quota") => Page::Quota,
                    Some("数据源") | Some("sources") => Page::Sources,
                    Some("价格") | Some("prices") => Page::Prices,
                    _ => Page::Overview,
                };
            }
            Msg::DetailPage(page) => {
                context.spawn_background(move |_| {
                    match Store::open(&db_path()).and_then(|s| s.detail(page, DETAIL_PAGE_SIZE)) {
                        Ok(d) => Msg::DetailLoaded(d.rows, d.total_events, page),
                        Err(e) => Msg::Failed(e.to_string()),
                    }
                });
            }
            Msg::DetailLoaded(rows, total, page) => {
                if let Some(s) = &mut self.snap {
                    s.detail = DetailBundle { rows, total, page };
                }
            }
            Msg::ToggleEdit => {
                self.editing = !self.editing;
            }
            Msg::MoveWidget(page, id, delta) => {
                self.config
                    .move_widget(&page, &id, &widgets::registry_ids(), delta);
            }
            Msg::HideWidget(page, id, hidden) => {
                self.config.set_hidden(&page, &id, hidden);
            }
            Msg::TrendHover(x) => {
                let sh = &self.trend.shared;
                let (w, n) = (sh.width.get(), sh.count.get());
                // Index under the pointer; unchanged → no repaint churn.
                let idx = if w > 0.0 && n > 0 {
                    Some(((x as f32 / (w / n as f32)) as usize).min(n - 1))
                } else {
                    None
                };
                if idx != sh.hover.get() {
                    sh.hover.set(idx);
                    self.trend.inv.invalidate();
                }
            }
            Msg::TrendLeave => {
                if self.trend.shared.hover.take().is_some() {
                    self.trend.inv.invalidate();
                }
            }
            Msg::QuotaDone(n, errs) => {
                diag!("[quota] {} rows, {} errors", n, errs.len());
                for e in errs {
                    diag!("[quota] {e}");
                }
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title("CodeLedger");
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(1180.0, 780.0),
        );
        let snap = self.snap.as_ref();
        let theme = &self.theme;

        let content: View = match self.page {
            Page::Overview => overview_page(
                snap,
                self.scanning,
                theme,
                &self.config,
                self.editing,
                context,
                &self.trend,
            ),
            Page::Detail => detail_page(snap, theme, context),
            Page::Quota => quota_page(snap, theme),
            Page::Sources => sources_page(snap, theme),
            Page::Prices => prices_page(snap, theme),
        };

        let item = |label: &'static str, page: Page| {
            SelectorBarItem::new()
                .text(label)
                .is_selected(self.page == page)
        };
        let nav = SelectorBar::new()
            .on_selected_text_changed(context.callback(Msg::Nav))
            .collection_slot(
                SelectorBarSlot::Items,
                [
                    KeyedView::new("overview", item("总览", Page::Overview)),
                    KeyedView::new("detail", item("明细", Page::Detail)),
                    KeyedView::new("quota", item("配额", Page::Quota)),
                    KeyedView::new("sources", item("数据源", Page::Sources)),
                    KeyedView::new("prices", item("价格", Page::Prices)),
                ],
            );
        StackPanel::new()
            .orientation(Orientation::Vertical)
            .children((
                Border::new()
                    .padding(Thickness::xy(20.0, 8.0))
                    .border_brush(theme.divider)
                    .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
                    .content(
                        StackPanel::new()
                            .orientation(Orientation::Horizontal)
                            .spacing(16.0)
                            .children((
                                SymbolIcon::new().symbol(Symbol::ViewAll),
                                TextBlock::new()
                                    .text("CodeLedger")
                                    .font_size(14.0)
                                    .font_weight(FontWeight::SEMI_BOLD)
                                    .vertical_alignment(VerticalAlignment::Center),
                                nav,
                            )),
                    ),
                content,
            ))
    }
}

impl Shell {
    /// Low-frequency vendor quota poll (spec §6.9) — `CL_NO_QUOTA` disables.
    /// Errors are logged via diag only; the quota page shows what landed.
    fn poll_quota_if_stale(&mut self, context: &ComponentContext<Self>) {
        let stale = self
            .quota_at
            .map(|t| t.elapsed().as_secs() > QUOTA_POLL_SECS)
            .unwrap_or(true);
        if !stale || std::env::var_os("CL_NO_QUOTA").is_some() {
            return;
        }
        self.quota_at = Some(std::time::Instant::now());
        context.spawn_background(|_| {
            let mut n = 0usize;
            let mut errs = Vec::new();
            match Store::open(&db_path()) {
                Ok(store) => {
                    for o in codeledger_core::quota::poll_all() {
                        if let Some(e) = o.error {
                            errs.push(format!("{}: {e}", o.app));
                        }
                        for q in o.quotas {
                            if store.insert_quota(&q).is_ok() {
                                n += 1;
                            }
                        }
                    }
                }
                Err(e) => errs.push(e.to_string()),
            }
            Msg::QuotaDone(n, errs)
        });
    }

    fn start_scan(&mut self, context: &ComponentContext<Self>) {
        if !self.scanning {
            diag!("[scan] start");
            self.scanning = true;
            let range = self.range;
            context.spawn_background(move |_| match load_all(range) {
                Ok(s) => Msg::Loaded(Box::new(s)),
                Err(e) => Msg::Failed(e),
            });
        }
    }
}

fn main() {
    // Stowed WinRT exceptions produce zero stderr; a Rust panic (e.g. inside a
    // spawn_background closure) lands in this hook instead.
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::capture();
        let msg = format!("PANIC: {info}\n{bt}");
        let _ = std::fs::write("cl_panic.log", &msg);
        eprintln!("{msg}");
    }));
    if let Err(e) = App::run_component::<Shell>(()) {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
