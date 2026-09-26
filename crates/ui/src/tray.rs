//! System tray: left click or "显示" focuses the window, "隐藏到托盘" hides
//! it (Win32 SW_HIDE — reactor 0.100.0 `WindowRef` has no hide verb), "退出"
//! closes. Tooltip carries today's totals. `TrayIcon` is `!Send` and lives
//! inside `Shell` on the UI thread.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const MENU_SHOW: &str = "show";
const MENU_HIDE: &str = "hide";
const MENU_QUIT: &str = "quit";

pub enum TrayAction {
    None,
    Focus,
    Hide,
    Quit,
}

/// Install the tray icon. `None` on failure or `CL_NOTRAY` (diagnostics).
pub fn install() -> Option<TrayIcon> {
    if std::env::var("CL_NOTRAY").is_ok() {
        return None;
    }
    let icon = glyph_icon().ok()?;
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::with_id(MENU_SHOW, "显示 CodeLedger", true, None));
    let _ = menu.append(&MenuItem::with_id(MENU_HIDE, "隐藏到托盘", true, None));
    let _ = menu.append(&MenuItem::with_id(MENU_QUIT, "退出", true, None));
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("CodeLedger")
        .with_icon(icon)
        .build()
        .map_err(|e| eprintln!("tray: install failed: {e}"))
        .ok()
}

/// Bring the main window to front. `WindowRef` has no focus verb in 0.100.0,
/// so we go through Win32: title lookup → restore → foreground.
#[cfg(windows)]
pub fn focus_main_window() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let title: Vec<u16> = "CodeLedger\0".encode_utf16().collect();
    unsafe {
        // Permit this process to steal foreground (background call otherwise
        // gets rejected silently on locked desktops).
        AllowSetForegroundWindow(u32::MAX);
        let hwnd: HWND = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(not(windows))]
pub fn focus_main_window() {}

/// Hide the main window — Win32 `SW_HIDE` works where `WindowRef` (0.100.0)
/// exposes nothing. Tray icon keeps the process reachable; left click or
/// "显示" restores via `focus_main_window` (SW_RESTORE unhides).
#[cfg(windows)]
pub fn hide_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, ShowWindow, SW_HIDE};
    let title: Vec<u16> = "CodeLedger\0".encode_utf16().collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_main_window() {}

/// Blocking poll over the tray icon + menu event channels. Re-armed by the
/// shell after every call.
pub fn next_action() -> TrayAction {
    crossbeam_channel::select! {
        recv(TrayIconEvent::receiver()) -> ev => match ev {
            Ok(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                ..
            })
            | Ok(TrayIconEvent::DoubleClick {
                button: MouseButton::Left, ..
            }) => TrayAction::Focus,
            _ => TrayAction::None,
        },
        recv(MenuEvent::receiver()) -> ev => match ev {
            Ok(e) if e.id.0 == MENU_QUIT => TrayAction::Quit,
            Ok(e) if e.id.0 == MENU_SHOW => TrayAction::Focus,
            Ok(e) if e.id.0 == MENU_HIDE => TrayAction::Hide,
            _ => TrayAction::None,
        },
    }
}

/// Programmatic 32×32 glyph: rounded accent-blue square with three bars —
/// readable at 16 px tray size, no asset files needed.
fn glyph_icon() -> Result<Icon, tray_icon::BadIcon> {
    const N: usize = 32;
    const R: i32 = 7;
    let mut rgba = vec![0u8; N * N * 4];
    let put = |rgba: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
        let i = (y * N + x) * 4;
        rgba[i..i + 4].copy_from_slice(&c);
    };
    for y in 0..N {
        for x in 0..N {
            let (xi, yi) = (x as i32, y as i32);
            let dx = (R - xi).max(xi - (N as i32 - 1 - R)).max(0);
            let dy = (R - yi).max(yi - (N as i32 - 1 - R)).max(0);
            if dx * dx + dy * dy <= R * R + 4 {
                put(&mut rgba, x, y, [0x4D, 0x8A, 0xE8, 0xFF]);
            }
        }
    }
    // Three bars — the "ledger" glyph.
    for (bx, bh) in [(9usize, 10usize), (14, 17), (19, 13)] {
        for y in (N - 5 - bh)..(N - 5) {
            for x in bx..bx + 3 {
                put(&mut rgba, x, y, [0xFF, 0xFF, 0xFF, 0xFF]);
            }
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32)
}
