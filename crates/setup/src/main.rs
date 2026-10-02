#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![allow(unsafe_code)] // AttachConsole/SetStdHandle in attach_console()
//! `GlobalTokenTracker` self-extracting installer — per-user, no admin.
//!
//! `installer/package.ps1` drops `payload.zip` (globaltokentracker-ui.exe +
//! globaltokentracker-cli.exe) into `crates/setup/payload/` before building; build.rs
//! embeds it via `include_bytes!`. Plain dev builds embed an empty zip and
//! refuse to install.
//!
//! Modes:
//! - no args / `--dir <p>`          → GUI install
//! - `--uninstall [--dir <p>]`      → GUI uninstall confirmation
//! - `--quiet` (with either)        → console path, no GUI
//! - `--cli`                        → force console path
//!
//! GUI is pure Win32/GDI (gui.rs) — the installer must run on machines that
//! do NOT yet have `WinAppRuntime`.

use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::io::{self, Cursor, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use winreg::RegKey;
use winreg::enums::RegType;
use winreg::types::{FromRegValue, ToRegValue};

#[allow(dead_code)]
mod gui;

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));

pub const APP: &str = "GlobalTokenTracker++";
pub const APP_ID: &str = "GlobalTokenTrackerPP";
pub const EXE_NAME: &str = "globaltokentracker_ui.exe";
pub const SETUP_EXE_NAME: &str = "globaltokentrackerpp-setup.exe";
/// Installer version — taken from the workspace manifest so
/// `DisplayVersion` and the upgrade prompt can never drift from the release
/// that shipped this binary.
pub const VER: &str = env!("CARGO_PKG_VERSION");
pub const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\GlobalTokenTrackerPP";

/// Spawned console tools (taskkill/cmd/powershell) must never allocate a
/// console window from our GUI process — it flickers on screen and, under
/// some endpoint-protection policies, console allocation hangs the child.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn local_appdata() -> Result<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA not set")
}

/// A directory only counts as our install dir when one of our own
/// executables lives in it. Uninstall must never `rmdir /S /Q` a lookalike
/// folder — a setup.exe double-clicked in Downloads resolves `--uninstall`
/// without `--dir` to `current_exe().parent()`, i.e. the Downloads folder.
fn looks_like_install_dir(dest: &Path) -> bool {
    dest.join(EXE_NAME).is_file() || dest.join(SETUP_EXE_NAME).is_file()
}

/// Modal error for GUI mode, where stderr is invisible.
fn message_box_error(text: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };
    unsafe {
        let _ = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(APP),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// Refuse to uninstall into a directory that does not contain our binaries.
/// Prints (console) or shows (GUI) an actionable message and exits the
/// process — partial uninstalls leave a confusing half-removed state behind.
fn guard_uninstall_dest(dest: &Path, gui_mode: bool) -> ! {
    let msg = format!(
        "目录不是本应用的安装目录（未找到 {}）。\n\n\
         为避免误删无关文件，已中止卸载。\n\
         若应用安装在别处，请以 --dir <安装目录> 显式指定后重试。",
        EXE_NAME
    );
    if gui_mode {
        message_box_error(&format!("{}\n\n{}", dest.display(), msg));
    } else {
        eprintln!("{}: {msg}", dest.display());
    }
    std::process::exit(2);
}

fn dest_dir(custom: Option<&str>) -> Result<PathBuf> {
    Ok(match custom {
        Some(d) => PathBuf::from(d),
        None => local_appdata()?.join("Programs").join(APP_ID),
    })
}

fn ps(script: &str) -> Result<std::process::Output> {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("spawn powershell")
}

/// Existing install's recorded (version, location). Lets a newer installer
/// upgrade in place even when the original install used a custom `--dir`.
fn installed_info() -> Option<(String, PathBuf)> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let loc: String = key.get_value("InstallLocation").ok()?;
    let dir = PathBuf::from(loc);
    // Stale entry (dir deleted without uninstalling) is not an install.
    if !dir.join(EXE_NAME).exists() {
        return None;
    }
    let ver: String = key.get_value("DisplayVersion").unwrap_or_default();
    Some((ver, dir))
}

fn ensure_runtime(log: &dyn Fn(String)) {
    log("检查系统运行环境… 绿色原生架构，免安装外部运行时".into());
}

fn stop_running(dest: &Path) {
    // Only instances running from THIS install dir are ours to stop — a
    // taskkill by image name would also take down a second copy installed
    // elsewhere (or a portable build the user is comparing against). Exclude
    // our own PID: in console mode the uninstaller itself lives inside dest.
    let prefix = dest.display().to_string();
    let prefix = prefix.trim_end_matches(['\\', '/']).replace('\'', "''");
    let script = format!(
        "$dest='{prefix}';$me={};\
         Get-Process | Where-Object {{ $_.Path -and $_.Id -ne $me -and \
         $_.Path.TrimEnd('\\','/').StartsWith($dest,[StringComparison]::OrdinalIgnoreCase) }} | \
         Stop-Process -Force -ErrorAction SilentlyContinue",
        std::process::id()
    );
    let _ = ps(&script);
}

fn extract_payload(dest: &Path, log: &dyn Fn(String)) -> Result<u64> {
    let mut zip = zip::ZipArchive::new(Cursor::new(PAYLOAD)).context("read embedded payload")?;
    if zip.is_empty() {
        bail!("此安装程序不含载荷（开发桩）。请运行 installer\\package.ps1 生成正式安装包。");
    }
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue; // skip path-traversal entries
        };
        let out = dest.join(&name);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        entry.read_to_end(&mut buf)?;
        fs::write(&out, &buf)?;
        total += buf.len() as u64;
        log(format!("    + {}", name.display()));
    }
    Ok(total)
}

fn make_shortcuts(dest: &Path) -> Result<()> {
    let start = env::var_os("APPDATA")
        .map(PathBuf::from)
        .context("APPDATA not set")?
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(APP);
    fs::create_dir_all(&start)?;
    let ui = dest.join(EXE_NAME);
    let uninstall = dest.join(SETUP_EXE_NAME);
    let script = format!(
        "$ws=New-Object -ComObject WScript.Shell;\
         $s=$ws.CreateShortcut('{}');$s.TargetPath='{}';$s.IconLocation='{}';$s.WorkingDirectory='{}';$s.Save();\
         $u=$ws.CreateShortcut('{}');$u.TargetPath='{}';$u.Arguments='--uninstall';$u.Save()",
        start.join(format!("{APP}.lnk")).display(),
        ui.display(),
        ui.display(),
        dest.display(),
        start.join(format!("卸载 {APP}.lnk")).display(),
        uninstall.display(),
    );
    let out = ps(&script)?;
    if !out.status.success() {
        bail!("创建快捷方式失败: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

fn register_uninstall(dest: &Path, size: u64) -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(UNINSTALL_KEY)?;
    let setup = dest.join(SETUP_EXE_NAME);
    key.set_value("DisplayName", &APP)?;
    key.set_value("DisplayVersion", &VER)?;
    key.set_value("Publisher", &"GlobalTokenTracker++")?;
    key.set_value("InstallLocation", &dest.display().to_string())?;
    key.set_value("DisplayIcon", &dest.join(EXE_NAME).display().to_string())?;
    key.set_value("EstimatedSize", &u32::try_from(size / 1024).unwrap_or(u32::MAX))?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    // Always pin the resolved dir — a custom --dir install must not
    // uninstall into the default location.
    let cmd = format!(
        "\"{}\" --uninstall --dir \"{}\"",
        setup.display(),
        dest.display()
    );
    key.set_value("UninstallString", &cmd)?;
    key.set_value("QuietUninstallString", &format!("{cmd} --quiet"))?;
    Ok(())
}

/// Environment key holding the *user* PATH (and the value name inside it). Both
/// PATH helpers take the key as a parameter so tests can drive a scratch key
/// instead of the real environment.
const USER_ENV_KEY: &str = "Environment";
const PATH_VALUE: &str = "Path";

/// Open the environment key for a read-then-write. `KEY_READ` is not optional:
/// a write-only handle cannot be queried, and a PATH rebuilt from a failed read
/// would replace the user's whole PATH instead of editing it.
fn open_user_env(key_path: &str) -> io::Result<RegKey> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey_with_flags(
        key_path,
        winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
    )
}

/// Read the user PATH together with its stored type.
///
/// `Ok(None)` means the value does not exist (a fresh account) — safe to create.
/// `Err` means it exists but cannot be read or is not a string: callers must
/// then leave the registry alone, because Windows keeps no backup of the value
/// and overwriting it from an empty read loses every entry we never saw.
fn read_path(env_key: &RegKey) -> io::Result<Option<(String, RegType)>> {
    match env_key.get_raw_value(PATH_VALUE) {
        Ok(raw) => {
            if raw.vtype != RegType::REG_SZ && raw.vtype != RegType::REG_EXPAND_SZ {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{PATH_VALUE} has unexpected type {:?}", raw.vtype),
                ));
            }
            let value = String::from_reg_value(&raw)?;
            Ok(Some((value, raw.vtype)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Write the user PATH back as `vtype`. The type must survive the round trip:
/// dropping `REG_EXPAND_SZ` to `REG_SZ` leaves entries such as
/// `%USERPROFILE%\AppData\Local\Microsoft\WindowsApps` unexpanded for every
/// process launched afterwards.
///
/// Before overwriting, the value Windows is about to lose is backed up to
/// `bak` (when given) — the registry keeps no history, so this file is the
/// only way back if an edit ever goes wrong. A failed backup aborts the write.
fn write_path(
    env_key: &RegKey,
    original: Option<(&str, RegType)>,
    value: &str,
    vtype: RegType,
    bak: Option<&Path>,
) -> io::Result<()> {
    if let Some((raw, t)) = original {
        let bak = bak.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no backup location for Path")
        })?;
        if let Some(dir) = bak.parent() {
            fs::create_dir_all(dir)?;
        }
        let tname = if t == RegType::REG_EXPAND_SZ {
            "REG_EXPAND_SZ"
        } else {
            "REG_SZ"
        };
        fs::write(bak, format!("type={tname}\n{raw}\n"))?;
    }
    let mut raw = value.to_reg_value();
    raw.vtype = vtype;
    env_key.set_raw_value(PATH_VALUE, &raw)
}

/// Expand `%VAR%` references with the Windows rules (`ExpandEnvironmentStringsW`,
/// the same expansion a `REG_EXPAND_SZ` PATH gets at logon): unknown variables
/// stay literal. Any API failure returns the input unchanged, i.e. comparison
/// degrades to the old literal behaviour rather than erroring.
fn expand_env(s: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::PCWSTR;
    if !s.contains('%') {
        return s.to_string();
    }
    let src: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: `src` is NUL-terminated and outlives the call; `buf` is a
        // valid writable slice whose length the API honours.
        let need =
            unsafe { ExpandEnvironmentStringsW(PCWSTR(src.as_ptr()), Some(&mut buf)) } as usize;
        if need == 0 {
            return s.to_string();
        }
        if need <= buf.len() {
            // `need` counts the terminating NUL.
            return String::from_utf16_lossy(&buf[..need - 1]);
        }
        buf.resize(need, 0);
    }
}

/// True when a PATH entry and a directory name the same path: both sides are
/// `%VAR%`-expanded first (a PATH entry written as `%LOCALAPPDATA%\Programs\X`
/// is the same directory as its absolute spelling), then compared without
/// surrounding whitespace / trailing `\`, ASCII-case-insensitively. Purely
/// textual otherwise: no `..`, `/`, or 8.3-short-name resolution.
fn same_dir(entry: &str, dir: &str) -> bool {
    let key = |p: &str| expand_env(p).trim().trim_end_matches('\\').to_string();
    key(entry).eq_ignore_ascii_case(&key(dir))
}

/// Tell every top-level window the environment changed, so a freshly opened
/// terminal picks up the new PATH without a logoff/logon round trip.
fn broadcast_env_change() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use windows::core::w;
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

/// PATH with every entry pointing at `dest` dropped. `None` when no entry
/// matched, so callers can skip the write entirely.
fn strip_dir(cur: &str, dest: &Path) -> Option<String> {
    let d = dest.display().to_string();
    let mut hit = false;
    let mut kept: Vec<&str> = Vec::new();
    for entry in cur.split(';') {
        if same_dir(entry, &d) {
            hit = true;
        } else {
            kept.push(entry);
        }
    }
    if hit { Some(kept.join(";")) } else { None }
}

/// Append dest to the *user* PATH (`key_path`) so `globaltokentracker-cli` works
/// in terminals. Best-effort: EDR/policy may guard HKCU\Environment for unsigned
/// binaries — a denied write must not fail the whole install.
fn extend_user_path(key_path: &str, dest: &Path, log: &dyn Fn(String)) {
    let bak = std::env::var_os("USERPROFILE").map(|p| {
        Path::new(&p)
            .join(".globaltokentracker")
            .join("path.bak")
    });
    extend_user_path_at(key_path, dest, log, bak.as_deref());
}

fn extend_user_path_at(
    key_path: &str,
    dest: &Path,
    log: &dyn Fn(String),
    bak: Option<&Path>,
) {
    let reject = |e: &dyn std::fmt::Display| {
        log(format!("PATH 追加被拒（{e}）——不影响使用，CLI 可用完整路径"));
    };
    let env_key = match open_user_env(key_path) {
        Ok(key) => key,
        Err(e) => return reject(&e),
    };
    let cur = match read_path(&env_key) {
        Ok(Some(found)) => found,
        // Fresh account: there is no user PATH yet, so create one. Windows
        // stores this particular value as REG_EXPAND_SZ; match that.
        Ok(None) => (String::new(), RegType::REG_EXPAND_SZ),
        Err(e) => {
            log(format!("PATH 未追加（读取失败：{e}）——不改动现有值"));
            return;
        }
    };
    let (cur, vtype) = cur;
    let d = dest.display().to_string();
    if cur.split(';').any(|p| same_dir(p, &d)) {
        return;
    }
    let new = if cur.is_empty() { d } else { format!("{cur};{d}") };
    let original = if cur.is_empty() {
        None
    } else {
        Some((cur.as_str(), vtype.clone()))
    };
    match write_path(&env_key, original, &new, vtype, bak) {
        Ok(()) => {
            broadcast_env_change();
            log("已加入用户 PATH（新开的终端生效）".into());
        }
        Err(e) => reject(&e),
    }
}

/// Drop dest from the *user* PATH (`key_path`). Never writes when the current
/// value cannot be read — see `read_path`.
fn remove_user_path(key_path: &str, dest: &Path, log: &dyn Fn(String)) {
    let bak = std::env::var_os("USERPROFILE").map(|p| {
        Path::new(&p)
            .join(".globaltokentracker")
            .join("path.bak")
    });
    remove_user_path_at(key_path, dest, log, bak.as_deref());
}

fn remove_user_path_at(key_path: &str, dest: &Path, log: &dyn Fn(String), bak: Option<&Path>) {
    let env_key = match open_user_env(key_path) {
        Ok(key) => key,
        Err(e) => return log(format!("PATH 未清理（无法打开环境键：{e}）")),
    };
    let (cur, vtype) = match read_path(&env_key) {
        Ok(Some(found)) => found,
        Ok(None) => return,
        Err(e) => return log(format!("PATH 未清理（读取失败：{e}）——不改动现有值")),
    };
    let Some(new) = strip_dir(&cur, dest) else {
        return;
    };
    match write_path(&env_key, Some((cur.as_str(), vtype.clone())), &new, vtype, bak) {
        Ok(()) => broadcast_env_change(),
        Err(e) => log(format!("PATH 清理失败：{e}")),
    }
}

/// Shared install body — `step(pct, label)` drives a progress bar,
/// `log(line)` records detail lines. Used by both the console path and
/// the GUI worker thread.
pub fn install_steps(
    dest: &Path,
    want_shortcut: bool,
    want_path: bool,
    step: &mut dyn FnMut(u32, &str),
    log: &dyn Fn(String),
) -> Result<()> {
    step(5, "检查系统环境…");
    ensure_runtime(log);
    step(15, "结束正在运行的实例…");
    stop_running(dest);
    fs::create_dir_all(dest)?;
    step(25, "释放程序文件…");
    let size = extract_payload(dest, log)?;
    // Persist a copy of this installer as the uninstaller.
    let self_exe = env::current_exe()?;
    let setup_copy = dest.join(SETUP_EXE_NAME);
    if self_exe.canonicalize()? != setup_copy.canonicalize().unwrap_or(setup_copy.clone()) {
        fs::copy(&self_exe, &setup_copy)?;
    }
    step(70, "写入卸载注册信息…");
    register_uninstall(dest, size)?;
    if want_shortcut {
        step(80, "创建开始菜单快捷方式…");
        make_shortcuts(dest)?;
    }
    if want_path {
        step(90, "加入用户 PATH…");
        extend_user_path(USER_ENV_KEY, dest, log);
    }
    step(100, "安装完成");
    log(format!("程序目录 : {}", dest.display()));
    Ok(())
}

pub fn uninstall_steps(
    dest: &Path,
    step: &mut dyn FnMut(u32, &str),
    log: &dyn Fn(String),
) -> Result<()> {
    step(10, "结束正在运行的实例…");
    stop_running(dest);
    step(40, "移除快捷方式与注册项…");
    let start = env::var_os("APPDATA")
        .map(PathBuf::from)
        .context("APPDATA not set")?
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(APP);
    let _ = fs::remove_dir_all(&start);
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
    remove_user_path(USER_ENV_KEY, dest, log);
    step(100, "已卸载");
    log("用户数据保留在 %USERPROFILE%\\.globaltokentracker".into());
    Ok(())
}

/// After an upgrade moved the install dir, retire the old one: drop its
/// PATH entry and defer-delete the folder (old exes were already stopped by
/// `stop_running`). Refuses to wipe when `new` nests inside `old` — a rmdir
/// of the parent would eat the fresh install.
pub fn cleanup_prior_install(old: &Path, new: &Path, log: &dyn Fn(String)) {
    let same = old
        .canonicalize()
        .ok()
        .zip(new.canonicalize().ok())
        .is_some_and(|(a, b)| a == b)
        || old.display().to_string().trim_end_matches('\\').eq_ignore_ascii_case(
            new.display().to_string().trim_end_matches('\\'),
        );
    if same || new.starts_with(old) || !old.join(EXE_NAME).exists() {
        return;
    }
    log(format!("清理旧安装目录 {}", old.display()));
    remove_user_path(USER_ENV_KEY, old, log);
    if let Err(e) = schedule_dir_delete(old) {
        log(format!("旧目录延迟删除失败：{e}"));
    }
}

/// Schedule deletion of `dest` after this process exits. Must be called at
/// the LAST possible moment — while running, our own exe inside `dest` is
/// locked and the rmdir would leave it behind. GUI mode calls this when the
/// window actually closes; console mode right before exit.
/// `raw_arg` keeps `/C ...` unquoted so cmd parses `&`/`>` as metachars
/// (`.arg()` would backslash-escape the inner quotes and break `/C`).
pub fn schedule_dir_delete(dest: &Path) -> Result<()> {
    Command::new("cmd")
        .raw_arg(format!(
            "/C ping 127.0.0.1 -n 2 >nul & rmdir /S /Q \"{}\"",
            dest.display()
        ))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("schedule self-delete")?;
    Ok(())
}

/// A GUI-subsystem exe launched from a terminal has no console — attach to
/// the parent's and reopen std handles so `--quiet`/`--help` still print.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows::core::w;
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let share = FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0);
        let rw = GENERIC_READ.0 | GENERIC_WRITE.0;
        if let Ok(out) = CreateFileW(
            w!("CONOUT$"),
            rw,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        ) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, out);
            let _ = SetStdHandle(STD_ERROR_HANDLE, out);
        }
        if let Ok(inp) = CreateFileW(
            w!("CONIN$"),
            rw,
            share,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        ) {
            let _ = SetStdHandle(STD_INPUT_HANDLE, inp);
        }
    }
}

fn usage() {
    println!("{APP} {VER} 安装程序");
    println!();
    println!("用法:");
    println!("  globaltokentracker-setup [--dir <路径>]      图形界面安装");
    println!("  globaltokentracker-setup --uninstall         图形界面卸载");
    println!("  globaltokentracker-setup --quiet [--uninstall] [--dir <路径>]");
    println!("                                             静默命令行安装/卸载");
    println!("  globaltokentracker-setup --cli               强制命令行模式");
}

fn main() -> Result<()> {
    let mut uninstall_flag = false;
    let mut quiet = false;
    let mut force_cli = false;
    let mut help = false;
    let mut dir: Option<String> = None;
    let mut it = env::args().skip(1);
    let mut parse_err: Option<String> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--uninstall" | "-u" => uninstall_flag = true,
            "--quiet" | "-q" => quiet = true,
            "--cli" => force_cli = true,
            "--dir" => match it.next() {
                Some(d) if !d.starts_with("--") => dir = Some(d),
                _ => {
                    parse_err = Some("--dir 需要紧跟一个路径".into());
                    break;
                }
            },
            "--help" | "-h" => help = true,
            other => {
                parse_err = Some(format!("未知参数 {other}"));
                break;
            }
        }
    }
    let gui_mode = !quiet && !force_cli && !help;
    if !gui_mode {
        attach_console();
    }
    if help {
        usage();
        return Ok(());
    }
    if let Some(e) = parse_err {
        eprintln!("{e}");
        usage();
        std::process::exit(2);
    }
    let prior = if uninstall_flag { None } else { installed_info() };
    let dest = match dir.as_deref() {
        Some(d) => PathBuf::from(d),
        // The uninstaller copy lives at <dest>\globaltokentracker-setup.exe —
        // uninstalling from it must target our own dir, not the default path.
        None if uninstall_flag => env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or(dest_dir(None)?),
        // Upgrade in place: a custom --dir install keeps its location.
        None => match &prior {
            Some((_, d)) => d.clone(),
            None => dest_dir(None)?,
        },
    };

    if uninstall_flag && !looks_like_install_dir(&dest) {
        guard_uninstall_dest(&dest, gui_mode);
    }

    if gui_mode {
        let temp_dir = std::env::temp_dir().join(format!("GTT_Setup_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log = |_s: String| {};
        if let Err(e) = extract_payload(&temp_dir, &log) {
            // A GUI-subsystem process has no console — without a message box
            // the user just sees a silent no-op.
            message_box_error(&format!("安装程序解压失败：{e}"));
            let _ = schedule_dir_delete(&temp_dir);
            std::process::exit(1);
        }

        let self_exe = match env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                message_box_error(&format!("无法定位安装程序：{e}"));
                let _ = schedule_dir_delete(&temp_dir);
                std::process::exit(1);
            }
        };
        let orig_flutter_exe = temp_dir.join(EXE_NAME);
        let installer_exe = temp_dir.join("installer_ui.exe");
        if let Err(e) = fs::copy(&orig_flutter_exe, &installer_exe) {
            message_box_error(&format!("无法准备安装界面：{e}"));
            let _ = schedule_dir_delete(&temp_dir);
            std::process::exit(1);
        }

        let mut cmd = Command::new(&installer_exe);
        if uninstall_flag {
            cmd.arg("--uninstall");
        } else {
            cmd.arg("--setup");
        }
        cmd.arg("--installer-source").arg(&temp_dir);
        cmd.arg("--installer-exe").arg(&self_exe);
        cmd.arg("--dest").arg(&dest);

        // A spawn failure means the installer window never appeared — the one
        // failure mode with no UI of its own. A non-zero exit already showed
        // its own error screen inside the installer window.
        if let Err(e) = cmd.status() {
            message_box_error(&format!("无法启动安装界面：{e}"));
            let _ = schedule_dir_delete(&temp_dir);
            std::process::exit(1);
        }

        // Clean up temporary extracted folder after installer window closes
        let _ = schedule_dir_delete(&temp_dir);
        return Ok(());
    }

    let log = |s: String| println!("    {s}");
    let mut step = |pct: u32, msg: &str| println!("==> [{pct:3}%] {msg}");
    if uninstall_flag {
        println!("{APP} 卸载 —— 移除 {}", dest.display());
        uninstall_steps(&dest, &mut step, &log)?;
        schedule_dir_delete(&dest)?; // fires after this process exits
        println!("已移除程序、快捷方式与卸载项；用户数据保留在 %USERPROFILE%\\.globaltokentracker");
    } else {
        match &prior {
            Some((v, _)) if v == VER => println!("{APP} {VER} 已安装 —— 重装修复 {}", dest.display()),
            Some((v, _)) => println!("{APP} v{v} → v{VER} —— 更新 {}", dest.display()),
            None => println!("{APP} {VER} 安装程序 —— 安装到 {}", dest.display()),
        }
        install_steps(&dest, true, true, &mut step, &log)?;
        if let Some((_, old)) = &prior {
            cleanup_prior_install(old, &dest, &log);
        }
        println!();
        println!("{APP} {VER} 安装完成。");
        println!("  开始菜单 : %APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\{APP}");
        println!("  数据目录 : %USERPROFILE%\\.globaltokentracker（首次运行创建）");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;

    const KEEP: &str = r"C:\Windows\system32";
    const OTHER: &str = r"D:\tools";
    const INSTALL: &str = r"C:\Users\me\AppData\Local\Programs\GlobalTokenTrackerPP";

    fn dest() -> PathBuf {
        PathBuf::from(INSTALL)
    }

    /// Throwaway `HKCU\Software\...` key so the PATH helpers can be driven
    /// against the registry without touching the real environment.
    struct Scratch {
        path: String,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = format!(r"Software\GlobalTokenTrackerPP-selftest-{name}");
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&path);
            RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(&path)
                .expect("create scratch key");
            Self { path }
        }

        fn open(&self) -> RegKey {
            open_user_env(&self.path).expect("open scratch key")
        }

        fn set(&self, value: &str, vtype: RegType) {
            write_path(&self.open(), None, value, vtype, None).expect("write scratch Path");
        }

        fn get(&self) -> Option<(String, RegType)> {
            read_path(&self.open()).expect("read scratch Path")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.path);
        }
    }

    #[test]
    fn same_dir_ignores_case_and_trailing_separator() {
        assert!(same_dir(INSTALL, INSTALL));
        assert!(same_dir(&format!("{INSTALL}\\"), INSTALL));
        assert!(same_dir(&INSTALL.to_lowercase(), INSTALL));
        assert!(!same_dir(&format!("{INSTALL}-old"), INSTALL));
        assert!(!same_dir(OTHER, INSTALL));
    }

    /// The default install dir spelled with `%LOCALAPPDATA%` and absolutely.
    /// Pure strings: nothing here reads or writes the registry.
    fn env_spellings() -> (String, String) {
        let local = env::var("LOCALAPPDATA").expect("LOCALAPPDATA must be set");
        (
            r"%LOCALAPPDATA%\Programs\GlobalTokenTrackerPP".to_string(),
            format!(r"{local}\Programs\GlobalTokenTrackerPP"),
        )
    }

    #[test]
    fn expand_env_follows_windows_rules() {
        let local = env::var("LOCALAPPDATA").expect("LOCALAPPDATA must be set");
        assert_eq!(expand_env(r"%LOCALAPPDATA%\x"), format!(r"{local}\x"));
        // Variable names are case-insensitive.
        assert_eq!(expand_env(r"%localappdata%\x"), format!(r"{local}\x"));
        // Unknown variables stay literal, like the API.
        assert_eq!(expand_env(r"%GTT_NO_SUCH_VAR%\x"), r"%GTT_NO_SUCH_VAR%\x");
        assert_eq!(expand_env(r"C:\plain"), r"C:\plain");
        assert_eq!(expand_env(""), "");
    }

    #[test]
    fn same_dir_matches_variable_and_absolute_spellings() {
        let (var_form, abs_form) = env_spellings();
        assert!(same_dir(&var_form, &abs_form));
        assert!(same_dir(&abs_form, &var_form));
        assert!(same_dir(&format!("{var_form}\\"), &abs_form));
        assert!(!same_dir(&format!("{var_form}-old"), &abs_form));
    }

    #[test]
    fn strip_dir_removes_only_the_matching_entry() {
        let cur = format!("{KEEP};{INSTALL};{OTHER}");
        assert_eq!(strip_dir(&cur, &dest()), Some(format!("{KEEP};{OTHER}")));
    }

    #[test]
    fn strip_dir_reports_nothing_to_do() {
        // An empty PATH — the state a failed read used to look like — must not
        // become a write.
        assert_eq!(strip_dir("", &dest()), None);
        assert_eq!(strip_dir(&format!("{KEEP};{OTHER}"), &dest()), None);
        assert_eq!(strip_dir(&format!("{INSTALL}-old"), &dest()), None);
    }

    /// A temp file the PATH backup can be pointed at, unique per test run.
    fn temp_bak(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gtt_pathbak_{}", std::process::id()));
        dir.join(format!("{tag}.bak"))
    }

    #[test]
    fn remove_user_path_strips_the_entry_and_keeps_the_value_type() {
        let scratch = Scratch::new("remove");
        scratch.set(&format!("{KEEP};{INSTALL};{OTHER}"), RegType::REG_EXPAND_SZ);
        let bak = temp_bak("remove");

        remove_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&bak));

        let (value, vtype) = scratch.get().expect("Path still readable");
        assert_eq!(value, format!("{KEEP};{OTHER}"));
        // Losing REG_EXPAND_SZ would leave %VARS% unexpanded for every process
        // started afterwards.
        assert_eq!(vtype, RegType::REG_EXPAND_SZ);
        // The overwritten value (and its type) is recoverable from the backup.
        let saved = fs::read_to_string(&bak).expect("backup written");
        assert_eq!(saved, format!("type=REG_EXPAND_SZ\n{KEEP};{INSTALL};{OTHER}\n"));
        let _ = fs::remove_file(&bak);
    }

    #[test]
    fn remove_user_path_leaves_an_unrelated_path_untouched() {
        let scratch = Scratch::new("untouched");
        scratch.set(&format!("{KEEP};{OTHER}"), RegType::REG_EXPAND_SZ);
        let bak = temp_bak("untouched");

        remove_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&bak));

        assert_eq!(
            scratch.get(),
            Some((format!("{KEEP};{OTHER}"), RegType::REG_EXPAND_SZ))
        );
        // Nothing matched → no write → no backup churn either.
        assert!(!bak.exists());
    }

    #[test]
    fn extend_user_path_appends_once() {
        let scratch = Scratch::new("append");
        scratch.set(KEEP, RegType::REG_EXPAND_SZ);
        let bak = temp_bak("append");

        extend_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&bak));
        extend_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&bak));

        let (value, vtype) = scratch.get().expect("Path still readable");
        assert_eq!(value, format!("{KEEP};{INSTALL}"));
        assert_eq!(vtype, RegType::REG_EXPAND_SZ);
        let _ = fs::remove_file(&bak);
    }

    #[test]
    fn extend_user_path_creates_a_missing_value() {
        let scratch = Scratch::new("create");
        assert_eq!(scratch.get(), None);

        extend_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&temp_bak("create")));

        let (value, vtype) = scratch.get().expect("Path created");
        assert_eq!(value, INSTALL);
        assert_eq!(vtype, RegType::REG_EXPAND_SZ);
    }

    #[test]
    fn a_failed_backup_aborts_the_write() {
        let scratch = Scratch::new("bakfail");
        scratch.set(KEEP, RegType::REG_EXPAND_SZ);
        // The backup cannot be created: its parent is a regular file.
        let blocker = temp_bak("blocker");
        fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        fs::write(&blocker, "not a directory").unwrap();
        let bak = blocker.join("path.bak");
        let before = scratch.get().clone();

        extend_user_path_at(&scratch.path, &dest(), &|_| {}, Some(&bak));

        assert_eq!(scratch.get(), before, "PATH must be untouched when backup fails");
        let _ = fs::remove_file(&blocker);
    }

    #[test]
    fn path_helpers_report_an_unopenable_key() {
        let missing = r"Software\GlobalTokenTrackerPP-selftest-no-such-key";
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(missing);
        let logged = std::sync::Mutex::new(Vec::new());
        let log = |line: String| logged.lock().unwrap().push(line);

        remove_user_path(missing, &dest(), &log);
        extend_user_path(missing, &dest(), &log);

        assert_eq!(logged.lock().unwrap().len(), 2);
    }
}
