//! System tray: left click or "显示" focuses the window, "隐藏到托盘" hides
//! it (Win32 SW_HIDE — reactor 0.100.0 `WindowRef` has no hide verb), "退出"
//! closes. Tooltip carries today's totals. `TrayIcon` is `!Send` and lives
//! inside `Shell` on the UI thread.

use crate::i18n::tr;
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

/// Install the tray icon. `None` on failure or `GTT_NOTRAY` (diagnostics).
pub fn install() -> Option<TrayIcon> {
    if std::env::var("GTT_NOTRAY").is_ok() {
        return None;
    }
    // Icon resource #1 is embedded in the exe by build.rs — LoadIconW resolves
    // it fine (unlike AppWindow.SetIcon). Fall back to the materialized file,
    // then the procedural glyph for non-Windows builds / broken resources.
    let icon = Icon::from_resource(1, Some((32, 32)))
        .or_else(|_| Icon::from_path(crate::window_icon_path(), Some((32, 32))))
        .or_else(|_| glyph_icon())
        .ok()?;
    // Labels freeze at startup — a language switch localizes new launches.
    let menu = Menu::new();
    let _ = menu.append(&MenuItem::with_id(
        MENU_SHOW,
        tr("显示 GlobalTokenTracker"),
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(MENU_HIDE, tr("隐藏到托盘"), true, None));
    let _ = menu.append(&MenuItem::with_id(MENU_QUIT, tr("退出"), true, None));
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("GlobalTokenTracker")
        .with_icon(icon)
        .build()
        .map_err(|e| eprintln!("tray: install failed: {e}"))
        .ok()
}

/// HWND of THIS process's main window. `FindWindowW` matches by title
/// system-wide — with a second GTT instance running it can return the other
/// process's window, which cross-process subclassing can't attach and
/// show/hide would then move the wrong window. EnumWindows + PID filter
/// picks our own copy deterministically.
#[cfg(windows)]
pub fn main_hwnd() -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };
    const TITLE: &[u16] = &[
        0x0047, 0x006C, 0x006F, 0x0062, 0x0061, 0x006C, 0x0054, 0x006F, 0x006B, 0x0065, 0x006E,
        0x0054, 0x0072, 0x0061, 0x0063, 0x006B, 0x0065, 0x0072, // "GlobalTokenTracker"
    ];
    struct Ctx {
        pid: u32,
        found: HWND,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lp: isize) -> i32 {
        unsafe {
            let ctx = &mut *(lp as *mut Ctx);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != ctx.pid {
                return 1;
            }
            let n = GetWindowTextLengthW(hwnd);
            if n != TITLE.len() as i32 {
                return 1;
            }
            let mut buf = [0u16; 64];
            if GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) == n
                && buf[..n as usize] == *TITLE
            {
                ctx.found = hwnd;
                return 0; // stop enumeration
            }
            1
        }
    }
    unsafe {
        let mut ctx = Ctx {
            pid: GetCurrentProcessId(),
            found: std::ptr::null_mut(),
        };
        EnumWindows(Some(cb), &mut ctx as *mut Ctx as isize);
        ctx.found
    }
}

/// Bring the main window to front. `WindowRef` has no focus verb in 0.100.0,
/// so we go through Win32: own-window lookup → restore → foreground.
#[cfg(windows)]
pub fn focus_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    unsafe {
        // Permit this process to steal foreground (background call otherwise
        // gets rejected silently on locked desktops).
        AllowSetForegroundWindow(u32::MAX);
        let hwnd = main_hwnd();
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
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    unsafe {
        let hwnd = main_hwnd();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
pub fn hide_main_window() {}

/// Try-hide variant for the `--minimized` autostart path: returns false
/// while the WinUI window isn't up yet so the caller can retry. `HWND` is
/// found by PID+title — identical lookup to `focus_main_window`.
#[cfg(windows)]
pub fn try_hide_main_window() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    unsafe {
        let hwnd = main_hwnd();
        if hwnd.is_null() {
            return false;
        }
        ShowWindow(hwnd, SW_HIDE);
    }
    true
}

#[cfg(not(windows))]
pub fn try_hide_main_window() -> bool {
    false
}

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
