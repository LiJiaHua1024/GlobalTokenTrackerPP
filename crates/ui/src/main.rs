#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! globaltokentracker-ui — WinUI 3 shell via windows-reactor.
//! Dumb renderer over core ViewModels; theme + layout are data (`ui.json`
//! next to ledger.db), so skins / widget ordering survive without recompiles.
//!
//! Release builds are GUI-subsystem — no stray console window. Diagnostics
//! (`diag!`, panic stderr) only exist when GTT_DEBUG=1; then we attach to the
//! parent console, or allocate one for a double-clicked debug launch.

mod config;
mod pages;
mod theme;
mod tray;
mod watch;
mod widgets;

use globaltokentracker_core::adapters;
use globaltokentracker_core::store::{default_db_path, EventRow, PriceRow, SourceHealth};
use globaltokentracker_core::viewmodel::fmt;
use globaltokentracker_core::viewmodel::Range;
use globaltokentracker_core::{Engine, OverviewVm, Store};
use config::{UiConfig, REFRESH_OPTIONS};
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
    /// Checked tools for the stats filter; `None` = all (persisted in ui.json).
    app_filter: Option<Vec<String>>,
    /// Checked display-model names; `None` = all (persisted in ui.json).
    model_filter: Option<Vec<String>>,
    /// Which filter-strip dropdown is open (in-content overlay, not a system
    /// Flyout — so no FlyoutPresenter surface stroke/shadow halo).
    open_menu: Option<MenuKind>,
    /// Quota page: app groups the user folded away (default all expanded).
    quota_collapsed: std::collections::BTreeSet<String>,
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
    /// Dwell timer fired for bar `usize` — arms the tooltip if still hovering.
    TrendTip(usize),
    TrendLeave,
    /// Statistics range changed (label text from the selector).
    SetRange(String),
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
    /// Event sink for RadioButton uncheck transitions — nothing to do.
    Noop,
    /// Background quota poll finished (rows written, channel errors).
    QuotaDone(usize, Vec<String>),
}

const DETAIL_PAGE_SIZE: i64 = 200;
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
        AllocConsole, AttachConsole, GetStdHandle, SetStdHandle,
        ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
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
) -> Result<Snapshot, String> {
    let store = Store::open(&db_path()).map_err(|e| e.to_string())?;
    let engine = Engine::new(store).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let _ = engine.scan_once().map_err(|e| e.to_string());
    let scan_ms = t.elapsed().as_millis();
    // Price book: fetch once per launch (force_prices on the first load) and
    // re-check every scan — stale >12h pulls models.dev + LiteLLM +
    // llmpricing.dev, then reprices unpriced events. Attempts are throttled
    // via prices_last_attempt so a dead network never hammers the CDNs;
    // failure keeps the current book untouched.
    if force_prices
        || globaltokentracker_core::pricing::prices_stale(&engine.store).unwrap_or(false)
    {
        match globaltokentracker_core::pricing::refresh(&engine.store) {
            Ok(r) => diag!(
                "[prices] synced: dev={} litellm={} llmpricing={} repriced={}",
                r.models_dev, r.litellm, r.llmpricing, r.repriced
            ),
            Err(e) => diag!("[prices] refresh failed: {e}"),
        }
    }
    let vm = engine
        .store
        .overview(range, apps.as_deref(), models.as_deref())
        .map_err(|e| e.to_string())?;
    let d = engine
        .store
        .detail(0, DETAIL_PAGE_SIZE, apps.as_deref(), models.as_deref())
        .map_err(|e| e.to_string())?;
    let sources = engine.store.source_health().map_err(|e| e.to_string())?;
    let prices = engine.store.price_rows(5000).map_err(|e| e.to_string())?;
    let prices_synced_at =
        globaltokentracker_core::pricing::last_live_sync(&engine.store).unwrap_or(None);
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

/// Arms one periodic-refresh timer. `secs == 0` (仅文件变更) skips arming —
/// the file watcher still live-refreshes on source changes.
fn arm_refresh(context: &ComponentContext<Shell>, secs: u64) {
    if secs == 0 {
        return;
    }
    context.spawn_background(move |_| {
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
        let app_filter = config.apps.clone();
        let model_filter = config.models.clone();
        // force_prices=true: one refresh attempt on every launch (per spec:
        // 每次打开软件自动获取一次), off the UI thread. Subsequent scans only
        // refresh when >12h stale.
        context.spawn_background(move |_| match load_all(range, app_filter, model_filter, true) {
            Ok(s) => Msg::Loaded(Box::new(s)),
            Err(e) => Msg::Failed(e),
        });
        // The watcher is the refresh source only in 仅文件变更 mode; in
        // timer mode per-file writes would defeat the configured cadence.
        if config.refresh_secs == 0 {
            arm_watcher(context);
        }
        let tray = tray::install();
        if tray.is_some() {
            arm_tray(context);
        }
        let theme = Theme::resolve(&config.theme);
        // OTLP receiver: dedicated blocking thread (never the reactor pool).
        // Port busy or GTT_NO_OTEL → file-based sources only.
        let _otel = globaltokentracker_core::otel::spawn(db_path());
        let page = match std::env::var("GTT_PAGE").as_deref() {
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
            app_filter: config.apps.clone(),
            model_filter: config.models.clone(),
            config,
            range,
            theme,
            editing: std::env::var("GTT_EDIT").is_ok(),
            tray,
            trend: widgets::TrendHandle::default(),
            quota_at: None,
            open_menu: None,
            quota_collapsed: std::collections::BTreeSet::new(),
        }
    }

    fn update(&mut self, message: Msg, context: &ComponentContext<Self>) {
        match message {
            Msg::Loaded(s) => {
                diag!("[scan] loaded, pending_rescan={}", self.pending_rescan);
                // Tool names can vanish from the ledger (pruned data); keep the
                // persisted filter honest — drop dead names and collapse back
                // to None once it covers every live tool.
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
                // Same reconcile for models: the checklist is scoped by the
                // app filter, so unchecking a tool can retire model names.
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
                self.snap = Some(*s);
                self.last_error = None;
                if let Some(tray) = &self.tray {
                    let total = self
                        .snap
                        .as_ref()
                        .map(|s| fmt::tokens_total(&s.vm.today))
                        .unwrap_or(0);
                    let _ = tray.set_tooltip(Some(format!(
                        "GlobalTokenTracker — 今日 {}",
                        fmt::tokens_exact(total)
                    )));
                }
                self.scanning = false;
                self.poll_quota_if_stale(context);
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
                self.start_scan(context);
            }
            Msg::SetRange(label) => {
                self.open_menu = None;
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
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::SetApps(filter) => {
                self.app_filter = filter;
                self.config.apps = self.app_filter.clone();
                self.config.save();
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
                self.scanning = false;
                self.start_scan(context);
            }
            Msg::SetModels(filter) => {
                self.model_filter = filter;
                self.config.models = self.model_filter.clone();
                self.config.save();
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
            Msg::Noop => {}
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
                        let _ = context.window().request_close();
                    }
                    tray::TrayAction::None => {}
                }
                if self.tray.is_some() {
                    arm_tray(context);
                }
            }
            Msg::Nav(tag) => {
                self.open_menu = None;
                self.page = match tag.as_deref() {
                    Some("明细") | Some("detail") => Page::Detail,
                    Some("配额") | Some("quota") => Page::Quota,
                    Some("数据源") | Some("sources") => Page::Sources,
                    Some("价格") | Some("prices") => Page::Prices,
                    _ => Page::Overview,
                };
            }
            Msg::DetailPage(page) => {
                self.open_menu = None;
                let apps = self.app_filter.clone();
                let models = self.model_filter.clone();
                context.spawn_background(move |_| {
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
            Msg::QuotaDone(n, errs) => {
                diag!("[quota] {} rows, {} errors", n, errs.len());
                for e in errs {
                    diag!("[quota] {e}");
                }
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title("GlobalTokenTracker");
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(1180.0, 780.0)
                // Real .ico path materialized beside the ledger — the
                // embedded resource still drives Explorer/shortcut icons.
                .icon(window_icon_path()),
        );
        let snap = self.snap.as_ref();
        let theme = &self.theme;

        let content: View = match self.page {
            Page::Overview => overview_page(
                snap,
                theme,
                context,
                &OverviewArgs {
                    scanning: self.scanning,
                    config: &self.config,
                    editing: self.editing,
                    trend: &self.trend,
                },
            ),
            Page::Detail => detail_page(snap, theme, context),
            Page::Quota => quota_page(snap, theme, &self.quota_collapsed, context),
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
            .horizontal_alignment(HorizontalAlignment::Center)
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
        // Root must be a Grid: a vertical StackPanel offers children infinite
        // height, which makes the page ScrollViewer measure at full content
        // size and never scroll. Star row bounds the scroll area.
        Grid::new()
            .rows([GridLength::Auto, GridLength::Auto, GridLength::STAR])
            .children((
                TitleBar::new()
                    .preferred_height(WindowTitleBarHeight::Tall)
                    .grid_row(0)
                    .slot(TitleBarSlot::Content, nav),
                brand,
                chrome,
                Border::new()
                    .grid_row(2)
                    .border_brush(theme.divider)
                    .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
                    .content(content),
                overlay,
            ))
    }
}

impl Shell {
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

    fn start_scan(&mut self, context: &ComponentContext<Self>) {
        if !self.scanning {
            diag!("[scan] start");
            self.scanning = true;
            let range = self.range;
            let apps = self.app_filter.clone();
            let models = self.model_filter.clone();
            context.spawn_background(move |_| match load_all(range, apps, models, false) {
                Ok(s) => Msg::Loaded(Box::new(s)),
                Err(e) => Msg::Failed(e),
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
