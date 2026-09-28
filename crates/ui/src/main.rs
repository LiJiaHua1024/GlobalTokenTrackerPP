#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! globaltokentracker-ui — WinUI 3 shell via windows-reactor.
//! Dumb renderer over core ViewModels; theme + layout are data (`ui.json`
//! next to ledger.db), so skins / widget ordering survive without recompiles.
//!
//! Release builds are GUI-subsystem — no stray console window. Diagnostics
//! (`diag!`, panic stderr) only exist when GTT_DEBUG=1; then we attach to the
//! parent console, or allocate one for a double-clicked debug launch.

mod autostart;
mod close_hook;
mod config;
mod fonts;
mod i18n;
mod pages;
mod theme;
mod tray;
mod watch;
mod widgets;

use config::{REFRESH_OPTIONS, UiConfig};
use globaltokentracker_core::adapters;
use globaltokentracker_core::power;
use globaltokentracker_core::store::{EventRow, PriceRow, SourceHealth, default_db_path};
use globaltokentracker_core::viewmodel::fmt;
use globaltokentracker_core::viewmodel::{Range, day_start_ms};
use globaltokentracker_core::{Engine, OverviewVm, Store};
use i18n::tr;
use pages::*;
use std::path::PathBuf;
use theme::Theme;
use windows_reactor::*;

/// Result of one background refresh. `Unchanged` means the scan ingested
/// nothing — the ~150ms of aggregate queries and the whole-view rebuild are
/// skipped, and only the scan-time indicator updates.
pub enum LoadOutcome {
    Fresh(Box<Snapshot>),
    Unchanged { scan_ms: u128, price_due: bool },
}

/// One background refresh produces this bundle (all Send-safe plain data).
/// `sources`/`prices` are page-scoped: fetched only while that page is open —
/// the 5k-row price table was otherwise rebuilt on every refresh tick.
pub struct Snapshot {
    pub vm: OverviewVm,
    pub detail: DetailBundle,
    pub sources: Option<Vec<SourceHealth>>,
    pub prices: Option<Vec<PriceRow>>,
    /// `prices` live-source sync timestamp (ms); `None` = seed only.
    pub prices_synced_at: Option<i64>,
    /// A price refresh is due (startup force or >12h stale) — run it as its
    /// own background task so network latency never gates the first paint.
    pub price_due: bool,
    pub scan_ms: u128,
}

pub struct DetailBundle {
    pub rows: Vec<EventRow>,
    pub total: u64,
    pub page: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Detail,
    Quota,
    Sources,
    Prices,
    Settings,
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
    /// Share-donut hover states — one per share-grid column (4 max).
    donuts: [widgets::DonutHandle; 4],
    /// Vendor quota channels poll at this cadence (network calls stay rare).
    quota_at: Option<std::time::Instant>,
    /// Overview statistics window (persisted in ui.json).
    range: Range,
    /// Checked tools for the stats filter; `None` = all (persisted in ui.json).
    app_filter: Option<Vec<String>>,
    /// Checked display-model names; `None` = all (persisted in ui.json).
    model_filter: Option<Vec<String>>,
    /// Which filter-strip dropdown is open (in-content overlay, not a system
    /// Flyout — so no FlyoutPresenter surface stroke/shadow halo).
    open_menu: Option<MenuKind>,
    /// Quota page: app groups the user folded away (default all expanded).
    quota_collapsed: std::collections::BTreeSet<String>,
    /// A background price fetch is in flight — prevents overlapping pulls
    /// when consecutive scans all report `price_due`.
    prices_refreshing: bool,
    /// Visible aggregates must rebuild even if the next scan lands nothing —
    /// set by filter/range/page changes, quota polls and repricing.
    views_stale: bool,
    /// Overview reflow column count — driven by the width ruler's
    /// Metrics events (4 until the first measurement lands).
    overview_cols: usize,
    /// Page-switch slide mid-flight — the new page's top-level blocks spring
    /// in horizontally (each with its own damping/velocity) while the old
    /// page slides out on a second layer. `None` when at rest.
    nav_anim: Option<widgets::NavAnim>,
    /// Close prompt dialog open (title-bar X swallowed by close_hook).
    close_prompt: bool,
    /// "记住我的选择" checkbox inside the close prompt — persists whichever
    /// button the user then picks into `config.close_action`.
    close_remember: bool,
    /// 1-DIP full-width ruler panel on the overview page; its surface
    /// metrics report the real content width for adaptive grids.
    ruler: ElementRef<SwapChainPanel>,
}

/// Which filter-strip picker is open — `Tools`/`Models` are multi-select
/// checkbox lists, `Refresh` is single-select cadence radios.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    Tools,
    Models,
    Refresh,
}

pub enum Msg {
    Loaded(LoadOutcome),
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
    /// Dwell timer fired for bar `usize` — arms the tooltip if still hovering.
    TrendTip(usize),
    TrendLeave,
    /// Share-donut pointer hover: (column index, hovered slice or None).
    DonutHover(u8, Option<usize>),
    /// Statistics range changed — resolved to `Range` at the selector so
    /// localized labels never leak into state handling.
    SetRange(Range),
    /// "自定义" selector item picked — adopt stored bounds (else last 7 days).
    PickCustomRange,
    /// Calendar picker: custom start day (local start-of-day, epoch ms).
    SetCustomStart(i64),
    /// Calendar picker: custom end day (local start-of-day, INCLUSIVE day).
    SetCustomEnd(i64),
    /// Tool checkbox toggled (app name, new checked state).
    ToggleApp(String, bool),
    /// Bulk tool-scope set from the filter flyout — `None` = all tools,
    /// `Some(vec![])` = deliberately empty view.
    SetApps(Option<Vec<String>>),
    /// Model checkbox toggled (display-model name, new checked state).
    ToggleModel(String, bool),
    /// Bulk model-scope set — `None` = all models, `Some(vec![])` = empty view.
    SetModels(Option<Vec<String>>),
    /// Refresh-cadence pick from the picker flyout (seconds).
    SetRefreshSecs(u64),
    /// Filter-strip pill clicked — opens its overlay, or closes it when the
    /// same one is already open; a different picker's overlay replaces it.
    ToggleMenu(MenuKind),
    /// Quota group header clicked — fold/unfold the app's quota windows.
    ToggleQuotaGroup(String),
    /// Width-ruler observer: overview grids should use this many columns.
    SetOverviewCols(usize),
    /// One animation frame of the nav slide — chained until NavAnim::done.
    NavAnimTick,
    /// Event sink for RadioButton uncheck transitions — nothing to do.
    Noop,
    /// Settings: window theme — "system" | "light" | "dark".
    SetThemeMode(&'static str),
    /// Settings: accent override — "" restores the theme default.
    SetAccent(&'static str),
    /// Settings: D2D chart font family — "" restores Segoe UI. `String`
    /// because the picker lists system fonts discovered at runtime.
    SetFontFamily(String),
    /// Settings: body font size (pt); title/h2/label derive from it.
    SetFontSize(f64),
    /// Settings: UI language — "zh" | "en".
    SetLang(&'static str),
    /// Settings: Run-key launch-at-login toggle; arg is the switch's new state.
    SetAutostart(bool),
    /// Settings: close-button behavior — "" = ask, "quit", "tray".
    SetCloseAction(&'static str),
    /// Title-bar X swallowed by the window subclass — open the ask dialog
    /// (or apply the remembered action directly).
    CloseRequested,
    /// Close prompt dismissed — arg says which button ended it.
    CloseDialogResult(ContentDialogResult),
    /// "记住我的选择" checkbox toggled inside the close prompt.
    CloseRemember(bool),
    /// Background quota poll finished (rows written, channel errors).
    QuotaDone(usize, Vec<String>),
    /// Background price-source fetch finished — repriced>0 triggers one
    /// follow-up scan so newly-priced events show their USD.
    PricesDone(Result<globaltokentracker_core::pricing::RefreshReport, String>),
}

const DETAIL_PAGE_SIZE: i64 = 200;
/// Local-day range math is 24h-aligned (same convention as `day_start_ms`).
const DAY_MS: i64 = 86_400_000;
/// Spec §6.9: quota polling is low-frequency by design.
const QUOTA_POLL_SECS: u64 = 30 * 60;

/// `GTT_DEBUG=1` → diagnostic stderr (invisible for normal GUI launches).
pub(crate) fn diag_enabled() -> bool {
    std::env::var_os("GTT_DEBUG").is_some()
}

macro_rules! diag {
    ($($t:tt)*) => {
        if crate::diag_enabled() {
            eprintln!($($t)*);
        }
    };
}
pub(crate) use diag;

/// Give `eprintln!`/`diag!` somewhere to land in a GUI-subsystem build:
/// attach to the invoker's console when present, else allocate a fresh one
/// (GTT_DEBUG double-click debugging). No-op when the console handle is
/// already valid (console-subsystem dev build).
#[cfg(windows)]
fn diag_console() {
    use std::ptr;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, GetStdHandle, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };
    unsafe {
        if !GetStdHandle(STD_ERROR_HANDLE).is_null() {
            return; // already have a console (dev build / console launch)
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 && AllocConsole() == 0 {
            return; // no console anywhere and can't allocate — stay silent
        }
        let mut name: Vec<u16> = "CONOUT$".encode_utf16().chain(Some(0)).collect();
        let h = CreateFileW(
            name.as_mut_ptr(),
            0x8000_0000 | 0x4000_0000, // GENERIC_READ | GENERIC_WRITE
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if !h.is_null() && h != -1isize as _ {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, h);
            let _ = SetStdHandle(STD_ERROR_HANDLE, h);
        }
    }
}

fn db_path() -> PathBuf {
    default_db_path()
}

/// Window icon as a real file. `AppWindow.SetIcon` does NOT resolve bare
/// resource-ID strings for an unpackaged exe (verified: the window ended up
/// with the generic pane glyph), so the embedded ICO is materialized once
/// into the data dir — identical for dev runs and installed copies.
pub(crate) fn window_icon_path() -> &'static str {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let dir = db_path()
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let p = dir.join("icon.ico");
        if !p.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&p, include_bytes!("../../../assets/icon.ico"));
        }
        p.to_string_lossy().into_owned()
    })
    .as_str()
}

fn load_all(
    range: Range,
    apps: Option<Vec<String>>,
    models: Option<Vec<String>>,
    force_prices: bool,
    page: Page,
    // True when the caller knows visible data must be rebuilt even if the
    // scan lands nothing (filter/range/page change, reprice, quota poll).
    force_views: bool,
) -> Result<LoadOutcome, String> {
    let store = Store::open(&db_path()).map_err(|e| e.to_string())?;
    let engine = Engine::new(store).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let report = engine.scan_once();
    let scan_ms = t.elapsed().as_millis();
    // A failed scan can't prove "nothing changed" — rebuild views anyway.
    let changed = report
        .map(|r| r.events_ingested > 0 || r.quotas > 0)
        .unwrap_or(true);
    // Price refresh runs as its own background task (see PricesDone) so a
    // slow network never gates first paint or a refresh tick. `price_due`
    // fires once per launch (force_prices) and whenever >12h stale; the
    // app_state attempt stamp throttles failures to the same TTL.
    let price_due = force_prices
        || globaltokentracker_core::pricing::prices_stale(&engine.store).unwrap_or(false);
    if !force_views && !changed {
        return Ok(LoadOutcome::Unchanged { scan_ms, price_due });
    }
    let vm = engine
        .store
        .overview(range, apps.as_deref(), models.as_deref())
        .map_err(|e| e.to_string())?;
    let d = engine
        .store
        .detail(0, DETAIL_PAGE_SIZE, apps.as_deref(), models.as_deref())
        .map_err(|e| e.to_string())?;
    // Page-scoped reads: the heavy tables only exist while their page is open.
    let sources = (page == Page::Sources)
        .then(|| engine.store.source_health())
        .transpose()
        .map_err(|e| e.to_string())?;
    let prices = (page == Page::Prices)
        .then(|| engine.store.price_rows(5000))
        .transpose()
        .map_err(|e| e.to_string())?;
    let prices_synced_at =
        globaltokentracker_core::pricing::last_live_sync(&engine.store).unwrap_or(None);
    Ok(LoadOutcome::Fresh(Box::new(Snapshot {
        vm,
        detail: DetailBundle {
            rows: d.rows,
            total: d.total_events,
            page: 0,
        },
        sources,
        prices,
        prices_synced_at,
        price_due,
        scan_ms,
    })))
}

/// Arms one periodic-refresh timer. `secs == 0` (仅文件变更) skips arming —
/// the file watcher still live-refreshes on source changes.
fn arm_refresh(context: &ComponentContext<Shell>, secs: u64) {
    if secs == 0 {
        return;
    }
    context.spawn_background(move |_| {
        power::worker("gtt-timer");
        std::thread::sleep(std::time::Duration::from_secs(secs));
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
        power::worker("gtt-watch");
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
    context.spawn_background(|_| {
        power::worker("gtt-tray");
        Msg::Tray(tray::next_action())
    });
}

impl Component for Shell {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let mut config = UiConfig::load();
        i18n::set_lang(i18n::Lang::from_config(&config.lang));
        // The registry is authoritative — ui.json only mirrors the last write
        // (a fresh install or manual removal clears the flag honestly).
        config.autostart = autostart::enabled();
        let range = Range::from_config(&config.range, config.range_start_ms, config.range_end_ms);
        let app_filter = config.apps.clone();
        let model_filter = config.models.clone();
        let page = match std::env::var("GTT_PAGE").as_deref() {
            Ok("detail") => Page::Detail,
            Ok("quota") => Page::Quota,
            Ok("sources") => Page::Sources,
            Ok("prices") => Page::Prices,
            Ok("settings") => Page::Settings,
            _ => Page::Overview,
        };
        // force_prices=true: one refresh attempt on every launch (per spec:
        // 每次打开软件自动获取一次), off the UI thread. Subsequent scans only
        // refresh when >12h stale.
        context.spawn_background(move |_| {
            power::worker("gtt-scan");
            match load_all(range, app_filter, model_filter, true, page, true) {
                Ok(s) => Msg::Loaded(s),
                Err(e) => Msg::Failed(e),
            }
        });
        // The watcher is the refresh source only in 仅文件变更 mode; in
        // timer mode per-file writes would defeat the configured cadence.
        if config.refresh_secs == 0 {
            arm_watcher(context);
        }
        let tray = tray::install();
        if tray.is_some() {
            arm_tray(context);
            // `--minimized` (autostart): hide once the window exists — the
            // background poll tolerates the WinUI window not being up yet.
            // No tray → stay visible; hidden without tray would be a zombie.
            if std::env::args().any(|a| a == "--minimized") {
                context.spawn_background(|_| {
                    power::worker("gtt-minhide");
                    for _ in 0..20 {
                        if tray::try_hide_main_window() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(400));
                    }
                    Msg::Noop
                });
            }
        }
        let theme = Theme::resolve(&config.theme);
        // OTLP receiver: dedicated blocking thread (never the reactor pool).
        // Port busy or GTT_NO_OTEL → file-based sources only.
        let _otel = globaltokentracker_core::otel::spawn(db_path());
        Self {
            snap: None,
            page,
            scanning: true,
            pending_rescan: false,
            last_error: None,
            app_filter: config.apps.clone(),
            model_filter: config.models.clone(),
            config,
            range,
            theme,
            editing: std::env::var("GTT_EDIT").is_ok(),
            tray,
            trend: widgets::TrendHandle::default(),
            donuts: std::array::from_fn(|_| widgets::DonutHandle::default()),
            quota_at: None,
            open_menu: None,
            quota_collapsed: std::collections::BTreeSet::new(),
            prices_refreshing: false,
            views_stale: true,
            overview_cols: 4,
            nav_anim: None,
            close_prompt: false,
            close_remember: false,
            ruler: ElementRef::new(),
        }
    }

    fn update(&mut self, message: Msg, context: &ComponentContext<Self>) {
        match message {
            Msg::Loaded(outcome) => {
                let price_due = match outcome {
                    LoadOutcome::Fresh(s) => {
                        diag!("[scan] loaded, pending_rescan={}", self.pending_rescan);
                        // Tool names can vanish from the ledger (pruned data);
                        // keep the persisted filter honest — drop dead names
                        // and collapse back to None on full coverage.
                        if let Some(f) = &mut self.app_filter {
                            f.retain(|a| s.vm.apps.contains(a));
                            if s.vm.apps.iter().all(|a| f.contains(a)) {
                                self.app_filter = None;
                            }
                            if self.app_filter != self.config.apps {
                                self.config.apps = self.app_filter.clone();
                                self.config.save();
                            }
                        }
                        if let Some(f) = &mut self.model_filter {
                            f.retain(|m| s.vm.models.contains(m));
                            if s.vm.models.iter().all(|m| f.contains(m)) {
                                self.model_filter = None;
                            }
                            if self.model_filter != self.config.models {
                                self.config.models = self.model_filter.clone();
                                self.config.save();
                            }
                        }
                        let price_due = s.price_due;
                        self.snap = Some(*s);
                        // Test hook: GTT_NAVTEST=<nav label> fires one page
                        // switch after first paint — scripted input can't
                        // reach the content island for real verification.
                        if let Ok(spec) = std::env::var("GTT_NAVTEST") {
                            static NAVFIRED: std::sync::atomic::AtomicBool =
                                std::sync::atomic::AtomicBool::new(false);
                            if !NAVFIRED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                // "label@ms" — alternates label↔总览 every
                                // ms, so any capture burst lands inside a
                                // flight window regardless of scan speed.
                                let mut parts = spec.split('@');
                                let label = parts.next().unwrap_or("配额").to_string();
                                let ms = parts
                                    .next()
                                    .and_then(|m| m.parse::<u64>().ok())
                                    .unwrap_or(1200);
                                let alt = parts.next().unwrap_or("总览").to_string();
                                for k in 0..8u64 {
                                    let to = if k % 2 == 0 { &label } else { &alt }.to_string();
                                    context.spawn_background(move |_| {
                                        power::worker("gtt-navtest");
                                        std::thread::sleep(std::time::Duration::from_millis(
                                            ms * (k + 1),
                                        ));
                                        Msg::Nav(Some(to))
                                    });
                                }
                            }
                        }
                        if let Some(tray) = &self.tray {
                            let total = self
                                .snap
                                .as_ref()
                                .map(|s| fmt::tokens_total(&s.vm.today))
                                .unwrap_or(0);
                            let _ = tray.set_tooltip(Some(tf!(
                                "GlobalTokenTracker — 今日 {}",
                                fmt::tokens_exact(total)
                            )));
                        }
                        price_due
                    }
                    LoadOutcome::Unchanged { scan_ms, price_due } => {
                        if let Some(old) = &mut self.snap {
                            old.scan_ms = scan_ms;
                        }
                        price_due
                    }
                };
                self.last_error = None;
                self.scanning = false;
                // New data mid-slide → drop the frozen page cache so the
                // entering page renders fresh on the next frame instead of
                // staying stale until settle.
                if let Some(a) = &self.nav_anim {
                    *a.cache.borrow_mut() = None;
                }
                self.poll_quota_if_stale(context);
                // Detached price-source fetch: triggered here (post-load) so
                // network latency never delays the snapshot we just painted.
                if price_due && !self.prices_refreshing {
                    self.prices_refreshing = true;
                    context.spawn_background(|_| {
                        power::worker("gtt-prices");
                        Msg::PricesDone(
                            Store::open(&db_path())
                                .and_then(|s| globaltokentracker_core::pricing::refresh(&s))
                                .map_err(|e| e.to_string()),
                        )
                    });
                }
                if self.pending_rescan {
                    self.pending_rescan = false;
                    self.start_scan(context);
                } else {
                    arm_refresh(context, self.config.refresh_secs);
                }
            }
            Msg::Failed(e) => {
                self.last_error = Some(e);
                self.scanning = false;
                // The failed load may have owed the UI a forced rebuild.
                self.views_stale = true;
                if self.pending_rescan {
                    self.pending_rescan = false;
                    self.start_scan(context);
                } else {
                    arm_refresh(context, self.config.refresh_secs);
                }
            }
            Msg::Tick => {
                self.start_scan(context);
            }
            Msg::Rescan => {
                // Manual refresh dismisses an open picker — the user moved on.
                self.open_menu = None;
                self.views_stale = true;
                self.start_scan(context);
            }
            Msg::SetRange(r) => {
                self.open_menu = None;
                self.set_range(r, context);
            }
            Msg::PickCustomRange => {
                self.open_menu = None;
                let (s, e) = match self.range {
                    Range::Custom { start_ms, end_ms } => (start_ms, end_ms),
                    _ => self
                        .config
                        .range_start_ms
                        .zip(self.config.range_end_ms)
                        .unwrap_or_else(|| (day_start_ms(6), day_start_ms(-1))),
                };
                self.set_range(Range::custom(s, e), context);
            }
            Msg::SetCustomStart(day_ms) => {
                let e = match self.range {
                    Range::Custom { end_ms, .. } => end_ms,
                    _ => self.config.range_end_ms.unwrap_or(day_ms + DAY_MS),
                };
                self.set_range(Range::custom(day_ms, e.max(day_ms + DAY_MS)), context);
            }
            Msg::SetCustomEnd(day_ms) => {
                // The picked day is inclusive → store start-of-next-day.
                let e = day_ms + DAY_MS;
                let s = match self.range {
                    Range::Custom { start_ms, .. } => start_ms,
                    _ => self.config.range_start_ms.unwrap_or(day_ms),
                };
                self.set_range(Range::custom(s.min(day_ms), e), context);
            }
            Msg::ToggleApp(app, on) => {
                let all: Vec<String> = self
                    .snap
                    .as_ref()
                    .map(|s| s.vm.apps.clone())
                    .unwrap_or_default();
                // Checked set: explicit filter, else every known tool.
                let mut set: std::collections::BTreeSet<String> = self
                    .app_filter
                    .clone()
                    .unwrap_or_else(|| all.to_vec())
                    .into_iter()
                    .collect();
                if on {
                    set.insert(app);
                } else {
                    set.remove(&app);
                }
                // Full coverage collapses back to None (no filter) so the
                // persisted config stays clean.
                self.app_filter = if all.iter().all(|a| set.contains(a)) {
                    None
                } else {
                    Some(set.into_iter().collect())
                };
                self.config.apps = self.app_filter.clone();
                self.config.save();
                self.views_stale = true;
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::SetApps(filter) => {
                self.app_filter = filter;
                self.config.apps = self.app_filter.clone();
                self.config.save();
                self.views_stale = true;
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::ToggleModel(model, on) => {
                let all: Vec<String> = self
                    .snap
                    .as_ref()
                    .map(|s| s.vm.models.clone())
                    .unwrap_or_default();
                // Same collapse rule as tools: full coverage → None.
                let mut set: std::collections::BTreeSet<String> = self
                    .model_filter
                    .clone()
                    .unwrap_or_else(|| all.to_vec())
                    .into_iter()
                    .collect();
                if on {
                    set.insert(model);
                } else {
                    set.remove(&model);
                }
                self.model_filter = if all.iter().all(|m| set.contains(m)) {
                    None
                } else {
                    Some(set.into_iter().collect())
                };
                self.config.models = self.model_filter.clone();
                self.config.save();
                self.views_stale = true;
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::SetModels(filter) => {
                self.model_filter = filter;
                self.config.models = self.model_filter.clone();
                self.config.save();
                self.views_stale = true;
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::SetRefreshSecs(secs) => {
                if REFRESH_OPTIONS.iter().any(|(s, _)| *s == secs)
                    && secs != self.config.refresh_secs
                {
                    let was_off = self.config.refresh_secs == 0;
                    self.config.refresh_secs = secs;
                    self.config.save();
                    if secs == 0 {
                        // Entering 仅文件变更: the watcher (which lapses in
                        // timer mode) becomes the refresh source again.
                        arm_watcher(context);
                    } else if was_off && !self.scanning {
                        // Leaving it: kick one timer now so the new cadence
                        // starts without waiting for the next scan to end.
                        arm_refresh(context, secs);
                    }
                }
                // Single-select semantics: a pick light-dismisses the panel.
                self.open_menu = None;
            }
            Msg::ToggleMenu(kind) => {
                self.open_menu = if self.open_menu == Some(kind) {
                    None
                } else {
                    Some(kind)
                };
            }
            Msg::ToggleQuotaGroup(app) => {
                if !self.quota_collapsed.remove(&app) {
                    self.quota_collapsed.insert(app);
                }
            }
            Msg::SetThemeMode(v) => {
                self.config.window_theme = v.to_string();
                self.config.save();
                // `window_visuals` is re-published every view — no extra work.
            }
            Msg::SetAccent(v) => {
                self.config.theme.accent = if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                };
                self.theme = Theme::resolve(&self.config.theme);
                self.config.save();
            }
            Msg::SetFontFamily(v) => {
                self.config.theme.font_family = if v.is_empty() { None } else { Some(v) };
                self.theme = Theme::resolve(&self.config.theme);
                self.config.save();
            }
            Msg::SetFontSize(body) => {
                // Slider range is 9–18; title/h2/label keep their offsets.
                let body = body.clamp(9.0, 18.0);
                let t = &mut self.config.theme;
                t.body_size = Some(body);
                t.title_size = Some(body + 10.0);
                t.h2_size = Some(body + 2.0);
                t.label_size = Some(body - 1.0);
                self.theme = Theme::resolve(&self.config.theme);
                self.config.save();
            }
            Msg::SetLang(v) => {
                self.config.lang = v.to_string();
                self.config.save();
                i18n::set_lang(i18n::Lang::from_config(v));
            }
            Msg::SetAutostart(on) => {
                // Registry write may fail (policy/AV) — mirror the real
                // outcome so the toggle reflects the truth, not the intent.
                let got = autostart::set(on);
                diag!("[settings] autostart want={on} got={got}");
                self.config.autostart = got;
                self.config.save();
            }
            Msg::CloseRequested => match self.config.close_action.as_str() {
                "quit" => self.quit_now(context),
                // "tray" remembered but the icon failed to install → hiding
                // would strand the process with no way back; ask instead.
                "tray" if self.tray.is_some() => tray::hide_main_window(),
                _ => self.close_prompt = true,
            },
            Msg::CloseDialogResult(res) => {
                self.close_prompt = false;
                // Consume the checkbox — a dismissed dialog must not leave
                // "remember" armed for the next open.
                let remember = self.close_remember;
                self.close_remember = false;
                match res {
                    ContentDialogResult::Primary => {
                        if remember {
                            self.config.close_action = "quit".into();
                            self.config.save();
                        }
                        self.quit_now(context);
                    }
                    ContentDialogResult::Secondary => {
                        if remember {
                            self.config.close_action = "tray".into();
                            self.config.save();
                        }
                        if self.tray.is_some() {
                            tray::hide_main_window();
                        } else {
                            self.quit_now(context);
                        }
                    }
                    _ => {}
                }
            }
            Msg::CloseRemember(on) => self.close_remember = on,
            Msg::SetCloseAction(v) => {
                self.config.close_action = v.to_string();
                self.config.save();
            }
            Msg::Noop => {}
            Msg::NavAnimTick => {
                // One msg per frame — the view() rebuild re-evaluates each
                // block's closed-form spring at the new elapsed time.
                if self.nav_anim.as_ref().is_none_or(|a| a.done()) {
                    if let Some(a) = self.nav_anim.take() {
                        diag!(
                            "[nav] anim settled: {} frames in {}ms",
                            a.frames.get(),
                            a.t0.elapsed().as_millis()
                        );
                    }
                } else {
                    if let Some(a) = &self.nav_anim {
                        a.frames.set(a.frames.get() + 1);
                    }
                    // 10ms sleep + ~4ms thin rebuild lands ~14ms cadence —
                    // a fresh frame is ready for every 60Hz vsync. Named but
                    // deliberately NOT efficiency-marked: frame cadence is
                    // latency-sensitive, stays on the P-core side.
                    context.spawn_background(|_| {
                        power::name_thread("gtt-anim");
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        Msg::NavAnimTick
                    });
                }
            }
            // Width-ruler metrics → reflow column count changed. The
            // observer already dedupes, so landing here always rebuilds.
            Msg::SetOverviewCols(n) => self.overview_cols = n.clamp(1, 4),
            Msg::WatchFired => {
                diag!("[watch] fired, scanning={}", self.scanning);
                // Only 仅文件变更 mode scans on file events — in timer mode
                // the next Tick picks up everything, and this one in-flight
                // watcher lapses (not re-armed).
                if self.config.refresh_secs == 0 {
                    arm_watcher(context);
                    if self.scanning {
                        self.pending_rescan = true;
                    } else {
                        self.start_scan(context);
                    }
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
                        self.quit_now(context);
                    }
                    tray::TrayAction::None => {}
                }
                if self.tray.is_some() {
                    arm_tray(context);
                }
            }
            Msg::Nav(tag) => {
                self.open_menu = None;
                let prev = self.page;
                self.page = match tag.as_deref() {
                    Some("明细") | Some("Details") | Some("detail") => Page::Detail,
                    Some("配额") | Some("Quota") | Some("quota") => Page::Quota,
                    Some("数据源") | Some("Sources") | Some("sources") => Page::Sources,
                    Some("价格") | Some("Prices") | Some("prices") => Page::Prices,
                    Some("设置") | Some("Settings") | Some("settings") => Page::Settings,
                    Some("总览") | Some("Overview") | Some("overview") => Page::Overview,
                    // None or an unrecognized label → keep the current page;
                    // a cleared selector must not teleport the user.
                    _ => prev,
                };
                if self.page != prev {
                    diag!("[nav] {prev:?} → {:?} (anim start)", self.page);
                    // Forward nav → new page springs in from the right while
                    // the old page exits left; backward flips the direction.
                    self.nav_anim = Some(widgets::NavAnim {
                        from: prev,
                        dir: if (self.page as u8) > (prev as u8) {
                            1.0
                        } else {
                            -1.0
                        },
                        t0: std::time::Instant::now(),
                        cache: std::cell::RefCell::new(None),
                        frames: std::cell::Cell::new(0),
                    });
                    context.spawn_background(|_| {
                        power::name_thread("gtt-anim");
                        std::thread::sleep(std::time::Duration::from_millis(16));
                        Msg::NavAnimTick
                    });
                }
                // Page-scoped data is lazy: first visit to Sources/Prices
                // triggers one load; ticks keep it fresh while open.
                let missing = match self.page {
                    Page::Sources => self.snap.as_ref().is_none_or(|s| s.sources.is_none()),
                    Page::Prices => self.snap.as_ref().is_none_or(|s| s.prices.is_none()),
                    _ => false,
                };
                if missing {
                    self.views_stale = true;
                    self.start_scan(context);
                }
            }
            Msg::DetailPage(page) => {
                self.open_menu = None;
                let apps = self.app_filter.clone();
                let models = self.model_filter.clone();
                context.spawn_background(move |_| {
                    power::worker("gtt-detail");
                    match Store::open(&db_path()).and_then(|s| {
                        s.detail(page, DETAIL_PAGE_SIZE, apps.as_deref(), models.as_deref())
                    }) {
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
                self.open_menu = None;
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
                diag!("[trend] hover x={x}");
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
                    sh.tip.set(None);
                    sh.pending.set(idx);
                    // Dwell arm: tooltip shows only if the pointer is still on
                    // the same bar when the timer lands (~450ms, Fluent-ish).
                    if let Some(i) = idx {
                        context.spawn_background(move |_| {
                            std::thread::sleep(std::time::Duration::from_millis(450));
                            Msg::TrendTip(i)
                        });
                    }
                    self.trend.inv.invalidate();
                }
            }
            Msg::TrendTip(i) => {
                let sh = &self.trend.shared;
                if sh.pending.get() == Some(i) && sh.hover.get() == Some(i) {
                    sh.tip.set(Some(i));
                    self.trend.inv.invalidate();
                }
            }
            Msg::TrendLeave => {
                let sh = &self.trend.shared;
                sh.pending.set(None);
                sh.tip.set(None);
                if sh.hover.take().is_some() {
                    self.trend.inv.invalidate();
                }
            }
            Msg::DonutHover(k, idx) => {
                if let Some(h) = self.donuts.get_mut(k as usize) {
                    // Unchanged → no repaint churn during pointer jitter.
                    if h.shared.hover.get() != idx {
                        h.shared.hover.set(idx);
                        h.inv.invalidate();
                    }
                }
            }
            Msg::QuotaDone(n, errs) => {
                diag!("[quota] {} rows, {} errors", n, errs.len());
                if n > 0 {
                    // New quota rows land outside the scan pipeline — force
                    // the next refresh to rebuild views for them.
                    self.views_stale = true;
                }
                for e in errs {
                    diag!("[quota] {e}");
                }
            }
            Msg::PricesDone(res) => {
                self.prices_refreshing = false;
                match res {
                    Ok(r) => {
                        diag!(
                            "[prices] synced: dev={} litellm={} llmpricing={} repriced={}",
                            r.models_dev,
                            r.litellm,
                            r.llmpricing,
                            r.repriced
                        );
                        // A successful sync refreshes prices_synced_at too;
                        // repriced>0 additionally changes visible USD.
                        self.views_stale = true;
                        if r.repriced > 0 {
                            self.start_scan(context);
                        }
                    }
                    Err(e) => diag!("[prices] refresh failed: {e}"),
                }
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // Publish the render locale before any `tr`/`tf!` resolves text.
        i18n::set_lang(i18n::Lang::from_config(&self.config.lang));
        // WM_CLOSE → Msg::CloseRequested (idempotent once the HWND exists).
        close_hook::ensure_installed(&context.sender());
        context.window_title("GlobalTokenTracker");
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(1180.0, 780.0)
                .theme(match self.config.window_theme.as_str() {
                    "light" => WindowTheme::Light,
                    "dark" => WindowTheme::Dark,
                    _ => WindowTheme::System,
                })
                // Real .ico path materialized beside the ledger — the
                // embedded resource still drives Explorer/shortcut icons.
                .icon(window_icon_path()),
        );
        let snap = self.snap.as_ref();
        let theme = &self.theme;

        // Slide-in-flight renders BOTH pages on stacked Grid layers: the
        // outgoing page stays alive (springs out under the incoming one)
        // instead of unmounting instantly. "enter" keeps its key across the
        // transition so a settled view is never re-mounted — canvas panels
        // (trend/donut swapchains) would otherwise flicker on every nav.
        let mut layers: Vec<KeyedView> = Vec::with_capacity(2);
        // While a slide is in flight both pages' content is built ONCE into
        // the NavAnim cache — each ~16ms tick then only allocates the thin
        // margin wrappers instead of two whole page trees (this was the
        // frame-cost that made the slide stutter).
        let mut cached_enter: Option<std::rc::Rc<Vec<View>>> = None;
        if let Some(a) = &self.nav_anim {
            if a.cache.borrow().is_none() {
                let built = widgets::NavCache {
                    enter_blocks: std::rc::Rc::new(self.page_blocks(self.page, snap, context)),
                    leave: self.page_view(a.from, snap, context, None),
                };
                *a.cache.borrow_mut() = Some(built);
            }
            let cache = a.cache.borrow();
            let c = cache.as_ref().expect("nav cache filled above");
            let (off, op) = a.exit();
            layers.push(KeyedView::new(
                "leave",
                Border::new()
                    .grid_row(0)
                    .margin(Thickness::new(off, 0.0, -off, 0.0))
                    .opacity(op)
                    .content(c.leave.clone()),
            ));
            cached_enter = Some(c.enter_blocks.clone());
        }
        // Border wraps unconditionally — keeps the enter layer's element
        // type identical across the anim→rest boundary (no remount flash).
        let off = self
            .nav_anim
            .as_ref()
            .map(|a| a.enter_layer())
            .unwrap_or(0.0);
        let enter_page = match cached_enter {
            // Cached blocks — only the margin wrappers are new this frame.
            Some(b) => pages::frame_page(
                theme,
                self.page_gap(self.page),
                (*b).clone(),
                self.nav_anim.as_ref(),
            ),
            None => self.page_view(self.page, snap, context, None),
        };
        layers.push(KeyedView::new(
            "enter",
            Border::new()
                .margin(Thickness::new(off, 0.0, -off, 0.0))
                .content(enter_page),
        ));
        let content: View = Grid::new()
            .rows([GridLength::STAR])
            .grid_row(2)
            .keyed_children(layers);

        let item = |label: &'static str, page: Page| {
            SelectorBarItem::new()
                .text(label)
                .is_selected(self.page == page)
        };
        // Nav floats on row 0 centered across the full window width — the
        // TitleBar.Content slot centers within the area that excludes the
        // caption buttons, which reads as left-shifted.
        let nav = SelectorBar::new()
            .on_selected_text_changed(context.callback(Msg::Nav))
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .grid_row(0)
            .collection_slot(
                SelectorBarSlot::Items,
                [
                    KeyedView::new("overview", item(t!("总览"), Page::Overview)),
                    KeyedView::new("detail", item(t!("明细"), Page::Detail)),
                    KeyedView::new("quota", item(t!("配额"), Page::Quota)),
                    KeyedView::new("sources", item(t!("数据源"), Page::Sources)),
                    KeyedView::new("prices", item(t!("价格"), Page::Prices)),
                    KeyedView::new("settings", item(t!("设置"), Page::Settings)),
                ],
            );
        // Brand mark pinned to the caption area's left edge. It renders in
        // the root Grid's row 0, on top of the TitleBar (TitleBar.Content is
        // centered by design and LeftHeader is not bound in this framework
        // version — so the nav keeps the Content slot and the brand floats).
        let brand = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(10.0)
            .vertical_alignment(VerticalAlignment::Center)
            .horizontal_alignment(HorizontalAlignment::Left)
            .margin(Thickness::new(12.0, 0.0, 0.0, 0.0))
            .grid_row(0)
            .children((
                // Embedded PNG (assets/icon-64.png) keeps the titlebar logo
                // identical to the window/tray icon with zero runtime files.
                ImageIcon::new()
                    .source_data(EncodedImage::from_static(include_bytes!(
                        "../../../assets/icon-64.png"
                    )))
                    .width(18.0)
                    .height(18.0),
                TextBlock::new()
                    .text("GlobalTokenTracker")
                    .font_size(13.0)
                    .font_weight(FontWeight::SEMI_BOLD)
                    .vertical_alignment(VerticalAlignment::Center),
            ));
        // Filter chrome is a pinned strip between title bar and scrolling
        // page on the data pages (overview/detail); it collapses elsewhere.
        let chrome_state = ChromeState {
            apps: &self.app_filter,
            models: &self.model_filter,
            refresh_secs: self.config.refresh_secs,
            open: self.open_menu,
        };
        let chrome: View = match (self.page, snap) {
            (Page::Overview | Page::Detail, Some(s)) => {
                filter_chrome(s, theme, &chrome_state, context)
            }
            _ => Border::new().into(),
        };
        let chrome = Border::new().grid_row(1).content(chrome);
        // Dropdown overlay renders last so its card floats above the page;
        // closed → an empty background-less Border (XAML skips hit-testing
        // null-background elements, so it never swallows clicks).
        let overlay: View = match (self.page, snap, self.open_menu) {
            (Page::Overview | Page::Detail, Some(s), Some(kind)) => {
                dropdown_overlay(s, theme, &chrome_state, kind, context)
            }
            _ => Border::new().grid_row(2).into(),
        };
        // Close prompt: quit vs hide-to-tray, with a remember checkbox.
        // The secondary button is disabled when no tray icon exists (hiding
        // would strand the process with no way back).
        let mut dlg_rows: Vec<View> = vec![
            TextBlock::new()
                .text(tr("要彻底退出，还是隐藏到托盘继续后台统计？"))
                .font_size(theme.body_size)
                .text_wrapping(windows_reactor::TextWrapping::Wrap)
                .into(),
        ];
        if self.tray.is_none() {
            dlg_rows.push(
                TextBlock::new()
                    .text(tr("托盘图标不可用"))
                    .font_size(theme.label_size)
                    .foreground(theme.subtle)
                    .into(),
            );
        }
        dlg_rows.push(
            CheckBox::new()
                .is_checked(self.close_remember)
                .on_is_checked_changed(context.callback(Msg::CloseRemember))
                .content(
                    TextBlock::new()
                        .text(tr("记住我的选择"))
                        .font_size(theme.body_size),
                ),
        );
        let close_dialog: View = ContentDialog::new()
            .is_open(self.close_prompt)
            .title(tr("关闭 GlobalTokenTracker"))
            .primary_button_text(tr("彻底退出"))
            .secondary_button_text(tr("隐藏到托盘"))
            .is_secondary_button_enabled(self.tray.is_some())
            .close_button_text(tr("取消"))
            .on_closed(context.callback(Msg::CloseDialogResult))
            .content(
                StackPanel::new()
                    .spacing(12.0)
                    .keyed_children(keyed(dlg_rows)),
            );
        // "pagehost" stays mounted across navs — the exit slide is driven by
        // the "leave" layer above, so this frame needs no transition chrome.
        // Root must be a Grid: a vertical StackPanel offers children infinite
        // height, which makes the page ScrollViewer measure at full content
        // size and never scroll. Star row bounds the scroll area.
        Grid::new()
            .rows([GridLength::Auto, GridLength::Auto, GridLength::STAR])
            .keyed_children([
                KeyedView::new(
                    "titlebar",
                    TitleBar::new()
                        .preferred_height(WindowTitleBarHeight::Tall)
                        .grid_row(0),
                ),
                KeyedView::new("nav", nav),
                KeyedView::new("brand", brand),
                KeyedView::new("chrome", chrome),
                KeyedView::new(
                    "pagehost",
                    Border::new()
                        .grid_row(2)
                        .border_brush(theme.divider)
                        .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                        .content(content),
                ),
                KeyedView::new("overlay", overlay),
                KeyedView::new("closedlg", close_dialog),
            ])
    }
}

impl Shell {
    /// Programmatic quit — `allow_next_close` lets the close we requested
    /// pass our own WM_CLOSE swallow (without it the subclass eats it).
    fn quit_now(&mut self, context: &ComponentContext<Self>) {
        close_hook::allow_next_close();
        let _ = context.window().request_close();
    }

    /// Raw top-level blocks of one page — cached per nav flight so ticks
    /// only re-wrap them, then assembled by `page_view`/`frame_page`.
    fn page_blocks(
        &self,
        page: Page,
        snap: Option<&Snapshot>,
        context: &mut ViewContext<Self>,
    ) -> Vec<View> {
        let theme = &self.theme;
        match page {
            Page::Overview => overview_page(
                snap,
                theme,
                context,
                &OverviewArgs {
                    scanning: self.scanning,
                    config: &self.config,
                    editing: self.editing,
                    trend: &self.trend,
                    cols: self.overview_cols,
                    ruler: &self.ruler,
                    donuts: &self.donuts,
                },
            ),
            Page::Detail => detail_page(snap, theme, context),
            Page::Quota => quota_page(snap, theme, &self.quota_collapsed, context),
            Page::Sources => sources_page(snap, theme),
            Page::Prices => prices_page(snap, theme),
            Page::Settings => settings_page(&self.config, theme, context),
        }
    }

    /// Top-level block spacing — Overview/Settings use the section gap,
    /// the list pages pack tighter (10 DIP, their historical rhythm).
    fn page_gap(&self, page: Page) -> f64 {
        match page {
            Page::Overview | Page::Settings => self.theme.section_gap,
            _ => 10.0,
        }
    }

    /// Assemble one page for the stacked-layer host. `anim` is `Some` only
    /// on the entering layer — the leaving layer renders at rest (its
    /// whole-page offset comes from the layer wrapper, not block springs).
    fn page_view(
        &self,
        page: Page,
        snap: Option<&Snapshot>,
        context: &mut ViewContext<Self>,
        anim: Option<&widgets::NavAnim>,
    ) -> View {
        pages::frame_page(
            &self.theme,
            self.page_gap(page),
            self.page_blocks(page, snap, context),
            anim,
        )
    }

    /// Low-frequency vendor quota poll (spec §6.9) — `GTT_NO_QUOTA` disables.
    /// Errors are logged via diag only; the quota page shows what landed.
    fn poll_quota_if_stale(&mut self, context: &ComponentContext<Self>) {
        let stale = self
            .quota_at
            .map(|t| t.elapsed().as_secs() > QUOTA_POLL_SECS)
            .unwrap_or(true);
        if !stale || std::env::var_os("GTT_NO_QUOTA").is_some() {
            return;
        }
        self.quota_at = Some(std::time::Instant::now());
        context.spawn_background(|_| {
            power::worker("gtt-quota");
            let mut n = 0usize;
            let mut errs = Vec::new();
            match Store::open(&db_path()) {
                Ok(store) => {
                    for o in globaltokentracker_core::quota::poll_all() {
                        if let Some(e) = o.error {
                            errs.push(format!("{}: {e}", o.app));
                        }
                        for q in o.quotas {
                            if matches!(store.insert_quota(&q), Ok(true)) {
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

    /// Apply + persist a range pick, then reload aggregates (totals/by_app
    /// are indexed; the rescan path is cheap).
    fn set_range(&mut self, r: Range, context: &ComponentContext<Self>) {
        if r == self.range {
            return;
        }
        self.range = r;
        self.config.range = r.key().to_string();
        if let Range::Custom { start_ms, end_ms } = r {
            self.config.range_start_ms = Some(start_ms);
            self.config.range_end_ms = Some(end_ms);
        }
        self.config.save();
        self.views_stale = true;
        self.scanning = false;
        self.start_scan(context);
    }

    fn start_scan(&mut self, context: &ComponentContext<Self>) {
        if !self.scanning {
            diag!("[scan] start");
            self.scanning = true;
            let force_views = std::mem::take(&mut self.views_stale);
            let range = self.range;
            let apps = self.app_filter.clone();
            let models = self.model_filter.clone();
            let page = self.page;
            context.spawn_background(move |_| {
                power::worker("gtt-scan");
                match load_all(range, apps, models, false, page, force_views) {
                    Ok(s) => Msg::Loaded(s),
                    Err(e) => Msg::Failed(e),
                }
            });
        }
    }
}

fn main() {
    #[cfg(windows)]
    if diag_enabled() {
        diag_console();
    }
    // Stowed WinRT exceptions produce zero stderr; a Rust panic (e.g. inside a
    // spawn_background closure) lands in this hook instead.
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::capture();
        let msg = format!("PANIC: {info}\n{bt}");
        let _ = std::fs::write("gtt_panic.log", &msg);
        eprintln!("{msg}");
    }));
    if let Err(e) = App::run_component::<Shell>(()) {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
