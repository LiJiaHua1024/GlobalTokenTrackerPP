#![allow(unsafe_code)]
// Win32 interop fights several pedantic lints (prelude glob imports, &ref as
// raw-pointer args, i32/u32 handle math, hex COLORREFs) — allowed module-wide.
#![allow(
    clippy::wildcard_imports,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::unreadable_literal,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::min_ident_chars,
    clippy::many_single_char_names,
    clippy::struct_excessive_bools
)]
//! GUI installer front-end — pure Win32/GDI.
//!
//! Deliberately NOT WinUI 3: the installer's job is to run on machines that
//! do not yet have WinAppRuntime, so the UI must be plain USER32+GDI.
//! Fluent-dark styling is applied manually: #202020 surface, #60CDFF accent,
//! dark title bar (DWMWA_USE_IMMERSIVE_DARK_MODE), `DarkMode_Explorer`-themed
//! controls, Per-Monitor-V2 DPI.
//!
//! Work runs on a std::thread; the worker updates the status/progress child
//! windows via SendMessage (marshalled to the UI thread) and posts a final
//! WM_APP to swap the button row to its Done state.

use crate::{install_steps, uninstall_steps, APP, VER};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath,
    GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
    GdipDeletePen, GdipDrawLines, GdipDrawPath, GdipFillPath, GdipSetSmoothingMode,
    GdiplusStartup, GdiplusStartupInput, GdiplusShutdown, GpGraphics, GpPath, PointF,
    SmoothingModeAntiAlias, UnitPixel,
};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

// COLORREF is 0x00BBGGRR.
const BG: u32 = 0x00202020;
const CARD: u32 = 0x002B2B2B;
const TEXT: u32 = 0x00E9E9E9;
const SUBTLE: u32 = 0x009C9C9C;
const ACCENT: u32 = 0x00FFCD60; // #60CDFF
const ACCENT_HOT: u32 = 0x00FFDD94;
const DANGER: u32 = 0x007D6AF9; // #F96A7D
const LINE: u32 = 0x00363636;

const IDC_EDIT: i32 = 100;
const IDC_BROWSE: i32 = 101;
const IDC_CHK_SHORTCUT: i32 = 102;
const IDC_CHK_PATH: i32 = 103;
const IDC_PRIMARY: i32 = 110;
const IDC_CANCEL: i32 = 111;
const IDC_STATUS: i32 = 112;
const IDC_PROG: i32 = 113;

const WM_APP_DONE: u32 = WM_APP + 1;

// ---------------------------------------------------------------- GDI+ AA helpers
// GDI RoundRect/paths are aliased (visible burrs on rounded corners); GDI+
// draws them anti-aliased. gdiplus.dll ships with Windows — no dependency.

/// COLORREF (0x00BBGGRR) → GDI+ ARGB (0xAARRGGBB).
fn argb(c: u32) -> u32 {
    0xFF00_0000 | ((c & 0xFF) << 16) | (c & 0xFF00) | (c >> 16)
}

fn gp_graphics(hdc: HDC) -> *mut GpGraphics {
    unsafe {
        let mut g = std::ptr::null_mut();
        let _ = GdipCreateFromHDC(hdc, &mut g);
        if !g.is_null() {
            let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        }
        g
    }
}

fn rr_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    unsafe {
        let mut p = std::ptr::null_mut();
        let _ = GdipCreatePath(FillModeAlternate, &mut p);
        if p.is_null() {
            return p;
        }
        let d = (r * 2.0).min(w).min(h);
        let _ = GdipAddPathArc(p, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(p, x + w - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(p, x + w - d, y + h - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(p, x, y + h - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(p);
        p
    }
}

fn fill_rr(hdc: HDC, rc: &RECT, r: f32, color: u32) {
    unsafe {
        let g = gp_graphics(hdc);
        if g.is_null() {
            return;
        }
        let p = rr_path(
            rc.left as f32,
            rc.top as f32,
            (rc.right - rc.left) as f32,
            (rc.bottom - rc.top) as f32,
            r,
        );
        let mut br = std::ptr::null_mut();
        let _ = GdipCreateSolidFill(argb(color), &mut br);
        let _ = GdipFillPath(g, br.cast(), p);
        let _ = GdipDeleteBrush(br.cast());
        let _ = GdipDeletePath(p);
        let _ = GdipDeleteGraphics(g);
    }
}

fn stroke_rr(hdc: HDC, rc: &RECT, r: f32, color: u32, width: f32) {
    unsafe {
        let g = gp_graphics(hdc);
        if g.is_null() {
            return;
        }
        // Stroke centered on the path — inset by half the pen width so the
        // border lands inside the rect rather than bleeding out.
        let i = width / 2.0;
        let p = rr_path(
            rc.left as f32 + i,
            rc.top as f32 + i,
            (rc.right - rc.left) as f32 - width,
            (rc.bottom - rc.top) as f32 - width,
            r,
        );
        let mut pen = std::ptr::null_mut();
        let _ = GdipCreatePen1(argb(color), width, UnitPixel, &mut pen);
        let _ = GdipDrawPath(g, pen, p);
        let _ = GdipDeletePen(pen);
        let _ = GdipDeletePath(p);
        let _ = GdipDeleteGraphics(g);
    }
}

fn draw_check(hdc: HDC, x: f32, y: f32, s: f32, color: u32) {
    unsafe {
        let g = gp_graphics(hdc);
        if g.is_null() {
            return;
        }
        let mut pen = std::ptr::null_mut();
        let _ = GdipCreatePen1(argb(color), s * 0.12, UnitPixel, &mut pen);
        let pts = [
            PointF { X: x + s * 0.20, Y: y + s * 0.55 },
            PointF { X: x + s * 0.42, Y: y + s * 0.76 },
            PointF { X: x + s * 0.82, Y: y + s * 0.26 },
        ];
        let _ = GdipDrawLines(g, pen, pts.as_ptr(), 3);
        let _ = GdipDeletePen(pen);
        let _ = GdipDeleteGraphics(g);
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Install,
    Uninstall,
}

#[derive(Default)]
struct Shared {
    ok: bool,
    err: String,
    launch: Option<PathBuf>,
}

struct Gui {
    mode: Mode,
    dir: PathBuf,
    prog: HWND,
    status: HWND,
    primary: HWND,
    cancel: HWND,
    edit: HWND,
    edit_focus: bool,
    // Owner-drawn checkboxes: BS_AUTOCHECKBOX|BS_OWNERDRAW collapses to
    // plain owner-draw (0x03|0x0B=0x0B) so Windows never toggles the check —
    // state lives here and clicks flip it manually.
    chk_shortcut: bool,
    chk_path: bool,
    // Brushes are returned to Windows every WM_CTLCOLOR* — must be
    // pre-allocated, not created per-message (GDI object leak).
    bg_brush: HBRUSH,
    field_brush: HBRUSH,
    working: bool,
    done_ok: bool,
    shared: Arc<Mutex<Shared>>,
}

/// UTF-16 with trailing NUL — bind the Vec in a `let` so the pointer outlives
/// the API call (a `PCWSTR(v.as_ptr())` from a temporary would dangle).
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Logical-unit scaler for a DPI value.
fn sc(x: i32, dpi: u32) -> i32 {
    x * dpi as i32 / 96
}

fn font(pt: f32, semibold: bool, dpi: u32) -> HFONT {
    unsafe {
        CreateFontW(
            -(pt * dpi as f32 / 72.0) as i32,
            0,
            0,
            0,
            (if semibold { FW_SEMIBOLD } else { FW_NORMAL }).0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            u32::from(DEFAULT_PITCH.0),
            w!("Segoe UI"),
        )
    }
}

fn hmenu_id(id: i32) -> HMENU {
    HMENU(id as isize as *mut _)
}

fn make_btn(
    parent: HWND,
    text: &[u16],
    x: i32,
    y: i32,
    wd: i32,
    ht: i32,
    id: i32,
    owner_drawn: bool,
    dpi: u32,
) -> HWND {
    unsafe {
        let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(WS_TABSTOP.0)
            | WINDOW_STYLE(if owner_drawn { BS_OWNERDRAW as u32 } else { BS_PUSHBUTTON as u32 });
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            PCWSTR(text.as_ptr()),
            style,
            x,
            y,
            wd,
            ht,
            Some(parent),
            Some(hmenu_id(id)),
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
        .unwrap_or_default();
        let _ = SetWindowTheme(h, w!("DarkMode_Explorer"), None);
        let _ = SendMessageW(h, WM_SETFONT, Some(WPARAM(font(11.0, false, dpi).0 as usize)), Some(LPARAM(1)));
        h
    }
}

fn set_prog(hwnd: HWND, pct: u32) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, pct.clamp(0, 1000) as isize);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

extern "system" fn prog_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let h = (rc.bottom - rc.top) as f32;
            fill_rr(hdc, &rc, h / 2.0, CARD);
            let pct = GetWindowLongPtrW(hwnd, GWLP_USERDATA).clamp(0, 1000) as i32;
            if pct > 0 {
                let w = (rc.right - rc.left).max(1) * pct / 1000;
                let fill = RECT { left: rc.left, top: rc.top, right: rc.left + w.max(h as i32), bottom: rc.bottom };
                fill_rr(hdc, &fill, h / 2.0, ACCENT);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

fn gui(hwnd: HWND) -> *mut Gui {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Gui }
}

fn set_status(hwnd: HWND, text: &str) {
    let s = w(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(s.as_ptr()));
    }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wpar: WPARAM, lpar: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => unsafe {
            let cs = lpar.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            LRESULT(0)
        },
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => unsafe {
            let g = &*gui(hwnd);
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            // Double-buffered full paint — no flicker.
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
            let old = SelectObject(mem, bmp.into());
            let bg = CreateSolidBrush(COLORREF(BG));
            FillRect(mem, &rc, bg);
            let _ = DeleteObject(bg.into());
            let _ = SetBkMode(mem, TRANSPARENT);
            let dpi = GetDpiForWindow(hwnd);
            let sx = |v: i32| sc(v, dpi);
            // Accent tick + title + subtitle (clear air between the lines).
            let tick = CreateSolidBrush(COLORREF(ACCENT));
            let tr = RECT { left: sx(28), top: sx(28), right: sx(32), bottom: sx(66) };
            FillRect(mem, &tr, tick);
            let _ = DeleteObject(tick.into());
            let title = font(20.0, true, dpi);
            let _ = SelectObject(mem, title.into());
            let _ = SetTextColor(mem, COLORREF(TEXT));
            let mut r = RECT { left: sx(44), top: sx(22), right: rc.right, bottom: sx(54) };
            let mut t = w(APP);
            let n = t.len() - 1;
            DrawTextW(mem, &mut t[..n], &mut r, DT_LEFT | DT_SINGLELINE);
            let sub = font(11.5, false, dpi);
            let _ = SelectObject(mem, sub.into());
            let _ = SetTextColor(mem, COLORREF(SUBTLE));
            let mut sub_t = w(&format!(
                "{} — v{VER}",
                if g.mode == Mode::Install { "本地 AI 编码工具用量统计" } else { "卸载程序" }
            ));
            let n2 = sub_t.len() - 1;
            let mut r2 = RECT { left: sx(44), top: sx(58), right: rc.right, bottom: sx(80) };
            DrawTextW(mem, &mut sub_t[..n2], &mut r2, DT_LEFT | DT_SINGLELINE);
            // Hairlines.
            let ln = CreateSolidBrush(COLORREF(LINE));
            for y in [104, 326] {
                let hr = RECT { left: sx(28), top: sx(y), right: rc.right - sx(28), bottom: sx(y) + 1 };
                FillRect(mem, &hr, ln);
            }
            let _ = DeleteObject(ln.into());
            // Flat rounded frame around the path edit (drawn on the parent
            // surface just outside the child rect — the edit paints over the
            // interior). Accent when focused, hairline otherwise.
            if g.mode == Mode::Install && !g.edit.0.is_null() {
                let mut er = RECT::default();
                let _ = GetWindowRect(g.edit, &mut er);
                let mut pts = [
                    POINT { x: er.left, y: er.top },
                    POINT { x: er.right, y: er.bottom },
                ];
                let _ = MapWindowPoints(Option::None, Some(hwnd), &mut pts);
                let frame = RECT {
                    left: pts[0].x - 1,
                    top: pts[0].y - 1,
                    right: pts[1].x + 1,
                    bottom: pts[1].y + 1,
                };
                let c = if g.edit_focus { ACCENT } else { 0x004A4A4A };
                stroke_rr(mem, &frame, sx(5) as f32, c, 1.0);
            }
            if g.mode == Mode::Uninstall {
                let body = font(11.5, false, dpi);
                let _ = SelectObject(mem, body.into());
                let _ = SetTextColor(mem, COLORREF(TEXT));
                let mut l1 = w(&format!("将从以下位置移除 {APP}："));
                let n3 = l1.len() - 1;
                let mut r3 = RECT { left: sx(32), top: sx(128), right: rc.right - sx(32), bottom: sx(150) };
                DrawTextW(mem, &mut l1[..n3], &mut r3, DT_LEFT);
                let mut l2 = w(&g.dir.display().to_string());
                let n4 = l2.len() - 1;
                let mut r4 = RECT { left: sx(32), top: sx(152), right: rc.right - sx(32), bottom: sx(174) };
                let _ = SetTextColor(mem, COLORREF(ACCENT));
                DrawTextW(mem, &mut l2[..n4], &mut r4, DT_LEFT);
                let _ = SetTextColor(mem, COLORREF(SUBTLE));
                let mut l3 = w("程序文件、快捷方式与卸载注册项将被移除；用户数据目录 %USERPROFILE%\\.globaltokentracker 保留。");
                let n5 = l3.len() - 1;
                let mut r5 = RECT { left: sx(32), top: sx(196), right: rc.right - sx(32), bottom: sx(250) };
                DrawTextW(mem, &mut l3[..n5], &mut r5, DT_LEFT | DT_WORDBREAK);
                let _ = DeleteObject(body.into());
            }
            let _ = DeleteObject(title.into());
            let _ = DeleteObject(sub.into());
            // Restore BEFORE deleting bmp — and crucially AFTER the BitBlt,
            // otherwise the blit sources from the 1x1 stock bitmap and the
            // client stays blank.
            let _ = BitBlt(hdc, 0, 0, rc.right, rc.bottom, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        },
        WM_CTLCOLOREDIT => unsafe {
            let g = &*gui(hwnd);
            let hdc = HDC(wpar.0 as *mut _);
            let _ = SetTextColor(hdc, COLORREF(TEXT));
            let _ = SetBkColor(hdc, COLORREF(CARD));
            LRESULT(g.field_brush.0 as isize)
        },
        WM_CTLCOLORSTATIC => unsafe {
            // Static labels sit directly on the window surface — return the
            // window bg brush or they'd show a lighter band behind the text.
            let g = &*gui(hwnd);
            let hdc = HDC(wpar.0 as *mut _);
            let _ = SetTextColor(hdc, COLORREF(SUBTLE));
            let _ = SetBkColor(hdc, COLORREF(BG));
            LRESULT(g.bg_brush.0 as isize)
        },
        WM_CTLCOLORBTN => unsafe {
            // Checkboxes: themed face + transparent text background.
            let hdc = HDC(wpar.0 as *mut _);
            let _ = SetTextColor(hdc, COLORREF(TEXT));
            let _ = SetBkMode(hdc, TRANSPARENT);
            LRESULT(GetStockObject(NULL_BRUSH).0 as isize)
        },
        WM_DRAWITEM => unsafe {
            let di = &*(lpar.0 as *const DRAWITEMSTRUCT);
            if di.CtlType != ODT_BUTTON {
                return LRESULT(0);
            }
            let g = &*gui(hwnd);
            let dpi = GetDpiForWindow(hwnd);
            let pressed = di.itemState.0 & ODS_SELECTED.0 != 0;
            let enabled = di.itemState.0 & ODS_DISABLED.0 == 0;
            let sx = |v: i32| sc(v, dpi);

            if di.CtlID == IDC_CHK_SHORTCUT as u32 || di.CtlID == IDC_CHK_PATH as u32 {
                // Owner-drawn checkbox — same rounded/flat vocabulary as the
                // buttons so the glyph and text match the dark surface.
                let mut rc = di.rcItem;
                let bg = CreateSolidBrush(COLORREF(BG));
                FillRect(di.hDC, &rc, bg);
                let _ = DeleteObject(bg.into());
                let checked = if di.CtlID == IDC_CHK_SHORTCUT as u32 { g.chk_shortcut } else { g.chk_path };
                let bs = sx(17);
                let bx = rc.left;
                let by = rc.top + (rc.bottom - rc.top - bs) / 2;
                let brc = RECT { left: bx, top: by, right: bx + bs, bottom: by + bs };
                let face = if checked { ACCENT } else if pressed { 0x003A3A3A } else { CARD };
                fill_rr(di.hDC, &brc, sx(4) as f32, face);
                let edge = if checked { ACCENT } else if enabled { 0x005A5A5A } else { 0x00404040 };
                stroke_rr(di.hDC, &brc, sx(4) as f32, edge, 1.0);
                if checked {
                    draw_check(di.hDC, bx as f32, by as f32, bs as f32, 0x00151515);
                }
                let _ = SetBkMode(di.hDC, TRANSPARENT);
                let _ = SetTextColor(di.hDC, COLORREF(if enabled { TEXT } else { SUBTLE }));
                let f = font(10.5, false, dpi);
                let _ = SelectObject(di.hDC, f.into());
                let mut buf = [0u16; 128];
                let n = GetWindowTextW(di.hwndItem, &mut buf).max(0) as usize;
                rc.left = bx + bs + sx(9);
                DrawTextW(di.hDC, &mut buf[..n], &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                let _ = DeleteObject(f.into());
                return LRESULT(1);
            }

            let is_primary = di.CtlID == IDC_PRIMARY as u32;
            let (fill, txt) = if is_primary {
                let accent = if g.mode == Mode::Uninstall && !g.done_ok { DANGER } else { ACCENT };
                let f = if !enabled { 0x00484848 } else if pressed { ACCENT_HOT } else { accent };
                (f, if enabled { 0x00151515 } else { 0x00AAAAAA })
            } else {
                let f = if pressed { 0x003A3A3A } else { 0x00333333 };
                (f, TEXT)
            };
            let mut rc = di.rcItem;
            let rad = sx(6) as f32;
            fill_rr(di.hDC, &rc, rad, fill);
            if !is_primary {
                stroke_rr(di.hDC, &rc, rad, 0x00555555, 1.0);
            }
            let _ = SetBkMode(di.hDC, TRANSPARENT);
            let _ = SetTextColor(di.hDC, COLORREF(txt));
            let f = font(11.0, is_primary, dpi);
            let _ = SelectObject(di.hDC, f.into());
            let mut buf = [0u16; 64];
            let n = GetWindowTextW(di.hwndItem, &mut buf).max(0) as usize;
            DrawTextW(di.hDC, &mut buf[..n], &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let _ = DeleteObject(f.into());
            LRESULT(1)
        },
        WM_COMMAND => unsafe {
            let id = (wpar.0 & 0xFFFF) as i32;
            let g = &mut *gui(hwnd);
            // Edit focus notifications → repaint so its frame turns accent.
            if id == IDC_EDIT && matches!((wpar.0 >> 16) as u32, x if x == EN_SETFOCUS || x == EN_KILLFOCUS) {
                g.edit_focus = (wpar.0 >> 16) as u32 == EN_SETFOCUS;
                let _ = InvalidateRect(Some(hwnd), None, false);
                return LRESULT(0);
            }
            match id {
                IDC_BROWSE => {
                    if let Some(p) = pick_folder(hwnd) {
                        let s = w(&p.display().to_string());
                        let _ = SetWindowTextW(GetDlgItem(Some(hwnd), IDC_EDIT).unwrap_or_default(), PCWSTR(s.as_ptr()));
                    }
                    LRESULT(0)
                }
                IDC_PRIMARY => {
                    if g.working {
                        return LRESULT(0);
                    }
                    if g.done_ok {
                        let launch = g.shared.lock().unwrap().launch.clone();
                        if let Some(exe) = launch {
                            let s = w(&exe.display().to_string());
                            ShellExecuteW(
                                Some(hwnd), w!("open"), PCWSTR(s.as_ptr()),
                                PCWSTR::null(), PCWSTR::null(), SW_SHOW,
                            );
                        }
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        return LRESULT(0);
                    }
                    if g.mode == Mode::Install {
                        let mut buf = vec![0u16; 1024];
                        let n = GetWindowTextW(
                            GetDlgItem(Some(hwnd), IDC_EDIT).unwrap_or_default(),
                            &mut buf,
                        ).max(0) as usize;
                        let d = PathBuf::from(String::from_utf16_lossy(&buf[..n]).trim());
                        if d.as_os_str().is_empty() {
                            let t = w("安装目录不能为空");
                            let c = w(APP);
                            MessageBoxW(Some(hwnd), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_ICONWARNING);
                            return LRESULT(0);
                        }
                        g.dir = d;
                    }
                    start_work(hwnd);
                    LRESULT(0)
                }
                IDC_CANCEL => {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    LRESULT(0)
                }
                IDC_CHK_SHORTCUT => {
                    g.chk_shortcut = !g.chk_shortcut;
                    let _ = InvalidateRect(GetDlgItem(Some(hwnd), IDC_CHK_SHORTCUT).ok(), None, false);
                    LRESULT(0)
                }
                IDC_CHK_PATH => {
                    g.chk_path = !g.chk_path;
                    let _ = InvalidateRect(GetDlgItem(Some(hwnd), IDC_CHK_PATH).ok(), None, false);
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wpar, lpar),
            }
        },
        WM_APP_DONE => unsafe {
            let g = &mut *gui(hwnd);
            g.working = false;
            let (ok, err) = {
                let s = g.shared.lock().unwrap();
                (s.ok, s.err.clone())
            };
            if ok {
                g.done_ok = true;
                set_status(g.status, "完成。");
                set_prog(g.prog, 1000);
                let lbl = w(if g.mode == Mode::Install { "启动并关闭" } else { "关闭" });
                let _ = SetWindowTextW(g.primary, PCWSTR(lbl.as_ptr()));
                let _ = ShowWindow(g.cancel, SW_HIDE);
                let _ = EnableWindow(g.primary, true);
            } else {
                set_status(g.status, &format!("失败：{err}"));
                let _ = EnableWindow(g.primary, true);
                let _ = EnableWindow(g.cancel, true);
                // Re-enable inputs so the user can fix the path and retry.
                for id in [IDC_EDIT, IDC_BROWSE, IDC_CHK_SHORTCUT, IDC_CHK_PATH] {
                    if let Ok(h) = GetDlgItem(Some(hwnd), id) {
                        let _ = EnableWindow(h, true);
                    }
                }
            }
            LRESULT(0)
        },
        WM_CLOSE => unsafe {
            let g = &*gui(hwnd);
            if g.working {
                // Closing now would kill the worker mid-install and leave a
                // half-written program dir — defer the close instead.
                set_status(g.status, "正在执行，完成后可关闭");
                return LRESULT(0);
            }
            if g.mode == Mode::Uninstall && g.done_ok {
                // Our exe lives inside the dir being deleted — only schedule
                // the deferred rmdir now that the window is really closing.
                let _ = crate::schedule_self_delete(&g.dir);
            }
            DefWindowProcW(hwnd, msg, wpar, lpar)
        },
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wpar, lpar) },
    }
}

fn pick_folder(hwnd: HWND) -> Option<PathBuf> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).ok()?;
        dlg.SetOptions(dlg.GetOptions().ok()? | FOS_PICKFOLDERS).ok()?;
        if dlg.Show(Some(hwnd)).is_err() {
            return None;
        }
        let item = dlg.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        Some(PathBuf::from(path.to_string().ok()?))
    }
}

fn start_work(hwnd: HWND) {
    let g = unsafe { &mut *gui(hwnd) };
    g.working = true;
    unsafe {
        let _ = EnableWindow(g.primary, false);
        let _ = EnableWindow(g.cancel, false);
        for id in [IDC_EDIT, IDC_BROWSE, IDC_CHK_SHORTCUT, IDC_CHK_PATH] {
            if let Ok(h) = GetDlgItem(Some(hwnd), id) {
                let _ = EnableWindow(h, false);
            }
        }
        let _ = ShowWindow(g.prog, SW_SHOW);
    }
    let want_shortcut = g.mode == Mode::Install && g.chk_shortcut;
    let want_path = g.mode == Mode::Install && g.chk_path;
    let dir = g.dir.clone();
    let mode = g.mode;
    let shared = g.shared.clone();
    // HWNDs are raw pointers in windows 0.62 → !Send. Cross the thread
    // boundary as usize, rebuild inside the worker.
    let status = g.status.0 as usize;
    let prog = g.prog.0 as usize;
    let hwnd_ptr = hwnd.0 as usize;
    std::thread::spawn(move || {
        let status = HWND(status as *mut _);
        let prog = HWND(prog as *mut _);
        let hwnd = HWND(hwnd_ptr as *mut _);
        let mut step = |pct: u32, msg: &str| {
            set_status(status, msg);
            set_prog(prog, pct);
        };
        // No console in GUI mode — println! on an invalid stdout panics and
        // would kill this worker mid-install. Detail lines are dropped; step
        // labels + the final error message carry the GUI narrative.
        let log = |_msg: String| {};
        let res = match mode {
            Mode::Install => install_steps(&dir, want_shortcut, want_path, &mut step, &log),
            Mode::Uninstall => uninstall_steps(&dir, &mut step, &log),
        };
        {
            let mut s = shared.lock().unwrap();
            match res {
                Ok(()) => {
                    s.ok = true;
                    if mode == Mode::Install {
                        s.launch = Some(dir.join("globaltokentracker-ui.exe"));
                    }
                }
                Err(e) => {
                    s.err = e.to_string();
                }
            }
        }
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_DONE, WPARAM(0), LPARAM(0));
        }
    });
}

pub fn run(mode: Mode, initial_dir: &Path) -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // GDI+ for anti-aliased rounded drawing (system DLL, always present).
        let mut gdip_token = 0usize;
        let gdip_in = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: FALSE,
            SuppressExternalCodecs: FALSE,
        };
        let _ = GdiplusStartup(&mut gdip_token, &gdip_in, std::ptr::null_mut());
        let hinst: HINSTANCE = GetModuleHandleW(PCWSTR::null())?.into();
        // Icon resource #1 is embedded by build.rs (assets/icon.ico) —
        // MAKEINTRESOURCEW(1) = a non-provenance pointer carrying the ordinal.
        let hicon = LoadIconW(
            Some(hinst),
            PCWSTR(std::ptr::without_provenance::<u16>(1)),
        )
        .unwrap_or_default();
        let wc = WNDCLASSW {
            hInstance: hinst,
            lpszClassName: w!("GttSetupWnd"),
            lpfnWndProc: Some(wnd_proc),
            hIcon: hicon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: CreateSolidBrush(COLORREF(BG)),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let pc = WNDCLASSW {
            hInstance: hinst,
            lpszClassName: w!("GttProgress"),
            lpfnWndProc: Some(prog_proc),
            ..Default::default()
        };
        RegisterClassW(&pc);

        let state = Box::new(Gui {
            mode,
            dir: initial_dir.to_path_buf(),
            prog: HWND::default(),
            status: HWND::default(),
            primary: HWND::default(),
            cancel: HWND::default(),
            edit: HWND::default(),
            edit_focus: false,
            chk_shortcut: true,
            chk_path: true,
            bg_brush: CreateSolidBrush(COLORREF(BG)),
            field_brush: CreateSolidBrush(COLORREF(CARD)),
            working: false,
            done_ok: false,
            shared: Arc::new(Mutex::new(Shared::default())),
        });
        let state_ptr = Box::into_raw(state);

        let title = w(if mode == Mode::Install {
            "GlobalTokenTracker 安装"
        } else {
            "GlobalTokenTracker 卸载"
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GttSetupWnd"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            sc(660, 96),
            sc(430, 96),
            None,
            None,
            Some(hinst),
            Some(state_ptr.cast_const().cast()),
        )?;
        let dark = TRUE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            4,
        );

        let dpi = GetDpiForWindow(hwnd);
        let sx = |v: i32| sc(v, dpi);
        let g = &mut *state_ptr;

        if mode == Mode::Install {
            let lbl_t = w("安装位置");
            let lbl = CreateWindowExW(
                WINDOW_EX_STYLE(0), w!("STATIC"), PCWSTR(lbl_t.as_ptr()), WS_CHILD | WS_VISIBLE,
                sx(32), sx(122), sx(200), sx(18), Some(hwnd), Some(hmenu_id(0)), Some(hinst), None,
            )?;
            let _ = SendMessageW(lbl, WM_SETFONT, Some(WPARAM(font(11.0, false, dpi).0 as usize)), Some(LPARAM(1)));
            let ed_t = w(&initial_dir.display().to_string());
            // No WS_EX_CLIENTEDGE — the 3D sunken edge fights the flat style;
            // the parent paints a rounded frame around it instead.
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0), w!("EDIT"), PCWSTR(ed_t.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
                sx(32), sx(144), sx(452), sx(28), Some(hwnd), Some(hmenu_id(IDC_EDIT)), Some(hinst), None,
            )?;
            let _ = SetWindowTheme(edit, w!("DarkMode_Explorer"), None);
            let _ = SendMessageW(edit, WM_SETFONT, Some(WPARAM(font(10.5, false, dpi).0 as usize)), Some(LPARAM(1)));
            g.edit = edit;
            let br_t = w("浏览…");
            make_btn(hwnd, &br_t, sx(492), sx(144), sx(96), sx(28), IDC_BROWSE, true, dpi);
            // Plain owner-drawn buttons (not AUTOCHECKBOX — the style bits
            // collide); clicks arrive as WM_COMMAND and flip Gui state.
            let c1t = w("创建开始菜单快捷方式");
            let c1 = CreateWindowExW(
                WINDOW_EX_STYLE(0), w!("BUTTON"), PCWSTR(c1t.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                sx(32), sx(194), sx(260), sx(22), Some(hwnd), Some(hmenu_id(IDC_CHK_SHORTCUT)), Some(hinst), None,
            )?;
            let _ = SendMessageW(c1, WM_SETFONT, Some(WPARAM(font(10.5, false, dpi).0 as usize)), Some(LPARAM(1)));
            let c2t = w("加入用户 PATH（终端可直接运行 CLI）");
            let c2 = CreateWindowExW(
                WINDOW_EX_STYLE(0), w!("BUTTON"), PCWSTR(c2t.as_ptr()),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(WS_TABSTOP.0 | BS_OWNERDRAW as u32),
                sx(32), sx(222), sx(360), sx(22), Some(hwnd), Some(hmenu_id(IDC_CHK_PATH)), Some(hinst), None,
            )?;
            let _ = SendMessageW(c2, WM_SETFONT, Some(WPARAM(font(10.5, false, dpi).0 as usize)), Some(LPARAM(1)));
        }

        g.status = CreateWindowExW(
            WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_CHILD | WS_VISIBLE,
            sx(32), sx(342), sx(340), sx(20), Some(hwnd), Some(hmenu_id(IDC_STATUS)), Some(hinst), None,
        )?;
        let _ = SendMessageW(g.status, WM_SETFONT, Some(WPARAM(font(10.0, false, dpi).0 as usize)), Some(LPARAM(1)));
        g.prog = CreateWindowExW(
            WINDOW_EX_STYLE(0), w!("GttProgress"), w!(""),
            WS_CHILD, sx(32), sx(372), sx(596), sx(6),
            Some(hwnd), Some(hmenu_id(IDC_PROG)), Some(hinst), None,
        )?;
        let ptxt = w(if mode == Mode::Install { "安装" } else { "卸载" });
        g.primary = make_btn(hwnd, &ptxt, sx(496), sx(338), sx(132), sx(34), IDC_PRIMARY, true, dpi);
        let ctxt = w("取消");
        g.cancel = make_btn(hwnd, &ctxt, sx(384), sx(338), sx(104), sx(34), IDC_CANCEL, true, dpi);

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).into() {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
        GdiplusShutdown(gdip_token);
        // state_ptr intentionally leaked — it lives as long as the process.
        Ok(())
    }
}
