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
use std::io::{Cursor, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod gui;

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));

const APP: &str = "GlobalTokenTracker";
const VER: &str = env!("CARGO_PKG_VERSION");
const RUNTIME_URL: &str =
    "https://aka.ms/windowsappsdk/1.8/latest/windowsappruntimeinstall-x64.exe";
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\GlobalTokenTracker";
/// Spawned console tools (taskkill/cmd/powershell) must never allocate a
/// console window from our GUI process — it flickers on screen and, under
/// some endpoint-protection policies, console allocation hangs the child.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn local_appdata() -> Result<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA not set")
}

fn dest_dir(custom: Option<&str>) -> Result<PathBuf> {
    Ok(match custom {
        Some(d) => PathBuf::from(d),
        None => local_appdata()?.join("Programs").join(APP),
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

fn winapp_runtime_present() -> bool {
    ps("Get-AppxPackage -Name 'Microsoft.WindowsAppRuntime*' | Select-Object -First 1 -ExpandProperty Name")
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

fn ensure_runtime(log: &dyn Fn(String)) -> Result<()> {
    if winapp_runtime_present() {
        log("Windows App Runtime 已就位".into());
        return Ok(());
    }
    log("未检测到 Windows App Runtime，下载微软官方安装程序…".into());
    let tmp = env::temp_dir().join("windowsappruntimeinstall-x64.exe");
    let mut resp = ureq::get(RUNTIME_URL).call().context("download runtime")?;
    let mut f = fs::File::create(&tmp)?;
    std::io::copy(&mut resp.body_mut().as_reader(), &mut f)?;
    log("安装运行时（可能触发 UAC）…".into());
    Command::new(&tmp).arg("--quiet").status().context("run runtime installer")?;
    if !winapp_runtime_present() {
        bail!("Windows App Runtime 安装未完成 —— GUI 将无法启动");
    }
    log("Windows App Runtime 已安装".into());
    Ok(())
}

fn stop_running() {
    for name in ["globaltokentracker-ui.exe", "globaltokentracker-cli.exe"] {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", name])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
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
    let ui = dest.join("globaltokentracker-ui.exe");
    let uninstall = dest.join("globaltokentracker-setup.exe");
    let script = format!(
        "$ws=New-Object -ComObject WScript.Shell;\
         $s=$ws.CreateShortcut('{}');$s.TargetPath='{}';$s.IconLocation='{}';$s.WorkingDirectory='{}';$s.Save();\
         $u=$ws.CreateShortcut('{}');$u.TargetPath='{}';$u.Arguments='--uninstall';$u.Save()",
        start.join(format!("{APP}.lnk")).display(),
        ui.display(),
        ui.display(),
        dest.display(),
        start.join(format!("Uninstall {APP}.lnk")).display(),
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
    let setup = dest.join("globaltokentracker-setup.exe");
    key.set_value("DisplayName", &APP)?;
    key.set_value("DisplayVersion", &VER)?;
    key.set_value("Publisher", &APP)?;
    key.set_value("InstallLocation", &dest.display().to_string())?;
    key.set_value("DisplayIcon", &dest.join("globaltokentracker-ui.exe").display().to_string())?;
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

/// Append dest to the *user* PATH so `globaltokentracker-cli` works in
/// terminals. Best-effort: EDR/policy may guard HKCU\Environment for unsigned
/// binaries — a denied write must not fail the whole install.
fn extend_user_path(dest: &Path, log: &dyn Fn(String)) {
    let inner = || -> Result<()> {
        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let env_key = hkcu.open_subkey_with_flags(
            "Environment",
            winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
        )?;
        let cur: String = env_key.get_value("Path").unwrap_or_default();
        let d = dest.display().to_string();
        if cur.split(';').any(|p| {
            p.trim_end_matches('\\').eq_ignore_ascii_case(d.trim_end_matches('\\'))
        }) {
            return Ok(());
        }
        let new = if cur.is_empty() { d } else { format!("{cur};{d}") };
        env_key.set_value("Path", &new)?;
        Ok(())
    };
    match inner() {
        Ok(()) => log("已加入用户 PATH（新开的终端生效）".into()),
        Err(e) => log(format!("PATH 追加被拒（{e}）——不影响使用，CLI 可用完整路径")),
    }
}

fn remove_user_path(dest: &Path) {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey_with_flags("Environment", winreg::enums::KEY_WRITE) {
        let cur: String = env_key.get_value("Path").unwrap_or_default();
        let d = dest.display().to_string();
        let kept: Vec<&str> = cur
            .split(';')
            .filter(|p| !p.trim_end_matches('\\').eq_ignore_ascii_case(d.trim_end_matches('\\')))
            .collect();
        let _ = env_key.set_value("Path", &kept.join(";"));
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
    step(5, "检查 Windows App Runtime…");
    ensure_runtime(log)?;
    step(15, "结束正在运行的实例…");
    stop_running();
    fs::create_dir_all(dest)?;
    step(25, "释放程序文件…");
    let size = extract_payload(dest, log)?;
    // Persist a copy of this installer as the uninstaller.
    let self_exe = env::current_exe()?;
    let setup_copy = dest.join("globaltokentracker-setup.exe");
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
        extend_user_path(dest, log);
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
    stop_running();
    step(40, "移除快捷方式与注册项…");
    let start = env::var_os("APPDATA")
        .map(PathBuf::from)
        .context("APPDATA not set")?
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(APP);
    let _ = fs::remove_dir_all(&start);
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
    remove_user_path(dest);
    step(100, "已卸载");
    log("用户数据保留在 %USERPROFILE%\\.globaltokentracker".into());
    Ok(())
}

/// Schedule deletion of `dest` after this process exits. Must be called at
/// the LAST possible moment — while running, our own exe inside `dest` is
/// locked and the rmdir would leave it behind. GUI mode calls this when the
/// window actually closes; console mode right before exit.
/// `raw_arg` keeps `/C ...` unquoted so cmd parses `&`/`>` as metachars
/// (.arg() would backslash-escape the inner quotes and break /C).
pub fn schedule_self_delete(dest: &Path) -> Result<()> {
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
    let dest = match dir.as_deref() {
        Some(d) => PathBuf::from(d),
        // The uninstaller copy lives at <dest>\globaltokentracker-setup.exe —
        // uninstalling from it must target our own dir, not the default path.
        None if uninstall_flag => env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or(dest_dir(None)?),
        None => dest_dir(None)?,
    };

    if gui_mode {
        return gui::run(
            if uninstall_flag { gui::Mode::Uninstall } else { gui::Mode::Install },
            &dest,
        );
    }

    let log = |s: String| println!("    {s}");
    let mut step = |pct: u32, msg: &str| println!("==> [{pct:3}%] {msg}");
    if uninstall_flag {
        println!("{APP} 卸载 —— 移除 {}", dest.display());
        uninstall_steps(&dest, &mut step, &log)?;
        schedule_self_delete(&dest)?; // fires after this process exits
        println!("已移除程序、快捷方式与卸载项；用户数据保留在 %USERPROFILE%\\.globaltokentracker");
    } else {
        println!("{APP} {VER} 安装程序 —— 安装到 {}", dest.display());
        install_steps(&dest, true, true, &mut step, &log)?;
        println!();
        println!("{APP} {VER} 安装完成。");
        println!("  开始菜单 : %APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\{APP}");
        println!("  数据目录 : %USERPROFILE%\\.globaltokentracker（首次运行创建）");
    }
    Ok(())
}
