//! codeledger-ui — WinUI 3 shell via windows-reactor.
//! Dumb renderer over core ViewModels; theme + layout are data (`ui.json`
//! next to ledger.db), so skins / widget ordering survive without recompiles.

mod config;
mod pages;
mod theme;
mod widgets;

use codeledger_core::store::{default_db_path, EventRow, PriceRow, SourceHealth};
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
    last_error: Option<String>,
    config: UiConfig,
    theme: Theme,
    editing: bool,
}

pub enum Msg {
    Loaded(Box<Snapshot>),
    Failed(String),
    Tick,
    Rescan,
    Nav(Option<String>),
    DetailPage(i64),
    DetailLoaded(Vec<EventRow>, u64, i64),
    ToggleEdit,
    MoveWidget(String, String, i32),
    HideWidget(String, String, bool),
}

const DETAIL_PAGE_SIZE: i64 = 200;
const REFRESH_SECS: u64 = 30;

fn db_path() -> PathBuf {
    default_db_path()
}

fn load_all() -> Result<Snapshot, String> {
    let store = Store::open(&db_path()).map_err(|e| e.to_string())?;
    let engine = Engine::new(store).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let _ = engine.scan_once().map_err(|e| e.to_string());
    let scan_ms = t.elapsed().as_millis();
    let vm = engine.store.overview().map_err(|e| e.to_string())?;
    let d = engine
        .store
        .detail(0, DETAIL_PAGE_SIZE)
        .map_err(|e| e.to_string())?;
    let sources = engine.store.source_health().map_err(|e| e.to_string())?;
    let prices = engine.store.price_rows(5000).map_err(|e| e.to_string())?;
    Ok(Snapshot {
        vm,
        detail: DetailBundle {
            rows: d.rows,
            total: d.total_events,
            page: 0,
        },
        sources,
        prices,
        scan_ms,
    })
}

fn arm_refresh(context: &ComponentContext<Shell>) {
    context.spawn_background(|_| {
        std::thread::sleep(std::time::Duration::from_secs(REFRESH_SECS));
        Msg::Tick
    });
}

impl Component for Shell {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        context.spawn_background(|_| match load_all() {
            Ok(s) => Msg::Loaded(Box::new(s)),
            Err(e) => Msg::Failed(e),
        });
        let config = UiConfig::load();
        let theme = Theme::resolve(&config.theme);
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
            last_error: None,
            config,
            theme,
            editing: std::env::var("CL_EDIT").is_ok(),
        }
    }

    fn update(&mut self, message: Msg, context: &ComponentContext<Self>) {
        match message {
            Msg::Loaded(s) => {
                self.snap = Some(*s);
                self.scanning = false;
                self.last_error = None;
                arm_refresh(context);
            }
            Msg::Failed(e) => {
                self.scanning = false;
                self.last_error = Some(e);
                arm_refresh(context);
            }
            Msg::Tick | Msg::Rescan => {
                if !self.scanning {
                    self.scanning = true;
                    context.spawn_background(|_| match load_all() {
                        Ok(s) => Msg::Loaded(Box::new(s)),
                        Err(e) => Msg::Failed(e),
                    });
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
