//! `CodeLedger` self-extracting installer — per-user, no admin.
//!
//! `installer/package.ps1` drops `payload.zip` (codeledger-ui.exe +
//! codeledger-cli.exe) into `crates/setup/payload/` before building; build.rs
//! embeds it via `include_bytes!`. Plain dev builds embed an empty zip and
//! refuse to install.
//!
//! Flags: `--uninstall`  `--quiet`  `--dir <path>`

use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip"));

const APP: &str = "CodeLedger";
const VER: &str = env!("CARGO_PKG_VERSION");
const RUNTIME_URL: &str =
    "https://aka.ms/windowsappsdk/1.8/latest/windowsappruntimeinstall-x64.exe";
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\CodeLedger";

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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("spawn powershell")
}

fn winapp_runtime_present() -> bool {
    ps("Get-AppxPackage -Name 'Microsoft.WindowsAppRuntime*' | Select-Object -First 1 -ExpandProperty Name")
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

fn ensure_runtime(quiet: bool) -> Result<()> {
    if winapp_runtime_present() {
        println!("==> Windows App Runtime 已就位");
        return Ok(());
    }
    println!("!! 未检测到 Windows App Runtime（GUI 需要，CLI 不受影响）");
    if !quiet {
        print!("下载并运行微软官方安装程序？[Y/n] ");
        std::io::stdout().flush()?;
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        let s = s.trim();
        if !(s.is_empty() || s.eq_ignore_ascii_case("y")) {
            println!("!! 已跳过 —— codeledger-ui.exe 在安装运行时后才能启动");
            return Ok(());
        }
    }
    let tmp = env::temp_dir().join("windowsappruntimeinstall-x64.exe");
    println!("==> 下载 {RUNTIME_URL}");
    let mut resp = ureq::get(RUNTIME_URL).call().context("download runtime")?;
    let mut f = fs::File::create(&tmp)?;
    std::io::copy(&mut resp.body_mut().as_reader(), &mut f)?;
    println!("==> 运行运行时安装程序（可能触发 UAC）");
    Command::new(&tmp).arg("--quiet").status().context("run runtime installer")?;
    if !winapp_runtime_present() {
        bail!("Windows App Runtime 安装未完成 —— GUI 将无法启动");
    }
    println!("==> Windows App Runtime 已安装");
    Ok(())
}

fn stop_running() {
    for name in ["codeledger-ui.exe", "codeledger-cli.exe"] {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn extract_payload(dest: &Path) -> Result<u64> {
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
        println!("    + {}", name.display());
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
    let ui = dest.join("codeledger-ui.exe");
    let uninstall = dest.join("codeledger-setup.exe");
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
    let setup = dest.join("codeledger-setup.exe");
    key.set_value("DisplayName", &APP)?;
    key.set_value("DisplayVersion", &VER)?;
    key.set_value("Publisher", &APP)?;
    key.set_value("InstallLocation", &dest.display().to_string())?;
    key.set_value("DisplayIcon", &dest.join("codeledger-ui.exe").display().to_string())?;
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

/// Append dest to the *user* PATH so `codeledger-cli` works in terminals.
fn extend_user_path(dest: &Path) -> Result<()> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let env_key = hkcu.open_subkey_with_flags(
        "Environment",
        winreg::enums::KEY_READ | winreg::enums::KEY_WRITE,
    )?;
    let cur: String = env_key.get_value("Path").unwrap_or_default();
    let d = dest.display().to_string();
    if cur.split(';').any(|p| p.trim_end_matches('\\').eq_ignore_ascii_case(d.trim_end_matches('\\'))) {
        return Ok(());
    }
    let new = if cur.is_empty() { d } else { format!("{cur};{d}") };
    env_key.set_value("Path", &new)?;
    println!("==> 已加入用户 PATH（新开的终端生效）");
    Ok(())
}

fn install(dest: &Path, quiet: bool) -> Result<()> {
    println!("{APP} {VER} 安装程序 —— 安装到 {}", dest.display());
    ensure_runtime(quiet)?;
    stop_running();
    fs::create_dir_all(dest)?;
    println!("==> 释放文件");
    let size = extract_payload(dest)?;
    // Persist a copy of this installer as the uninstaller.
    let self_exe = env::current_exe()?;
    let setup_copy = dest.join("codeledger-setup.exe");
    if self_exe.canonicalize()? != setup_copy.canonicalize().unwrap_or(setup_copy.clone()) {
        fs::copy(&self_exe, &setup_copy)?;
    }
    make_shortcuts(dest)?;
    register_uninstall(dest, size)?;
    extend_user_path(dest)?;
    println!();
    println!("{APP} {VER} 安装完成：");
    println!("  程序目录 : {}", dest.display());
    println!("  开始菜单 : %APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\{APP}");
    println!("  数据目录 : %USERPROFILE%\\.codeledger（首次运行创建）");
    Ok(())
}

fn uninstall(dest: &Path) -> Result<()> {
    println!("{APP} 卸载 —— 移除 {}", dest.display());
    stop_running();
    let start = env::var_os("APPDATA")
        .map(PathBuf::from)
        .context("APPDATA not set")?
        .join(r"Microsoft\Windows\Start Menu\Programs")
        .join(APP);
    let _ = fs::remove_dir_all(&start);
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
    if let Ok(env_key) = hkcu.open_subkey_with_flags("Environment", winreg::enums::KEY_WRITE) {
        let cur: String = env_key.get_value("Path").unwrap_or_default();
        let d = dest.display().to_string();
        let kept: Vec<&str> = cur
            .split(';')
            .filter(|p| !p.trim_end_matches('\\').eq_ignore_ascii_case(d.trim_end_matches('\\')))
            .collect();
        let _ = env_key.set_value("Path", &kept.join(";"));
    }
    // We're running from dest\codeledger-setup.exe — schedule dir deletion
    // via a detached cmd after this process exits. raw_arg keeps `/C ...`
    // unquoted in the command line so cmd parses `&`/`>` as metachars
    // (.arg() would backslash-escape the inner quotes and break /C).
    Command::new("cmd")
        .raw_arg(format!(
            "/C ping 127.0.0.1 -n 2 >nul & rmdir /S /Q \"{}\"",
            dest.display()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("schedule self-delete")?;
    println!("已移除程序、快捷方式与卸载项；用户数据保留在 %USERPROFILE%\\.codeledger");
    Ok(())
}

fn main() -> Result<()> {
    let mut uninstall_flag = false;
    let mut quiet = false;
    let mut dir: Option<String> = None;
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--uninstall" | "-u" => uninstall_flag = true,
            "--quiet" | "-q" => quiet = true,
            "--dir" => dir = it.next(),
            "--help" | "-h" => {
                println!("{APP} {VER} installer\n  --uninstall  卸载\n  --quiet      静默（自动接受运行时安装）\n  --dir <p>    自定义安装目录");
                return Ok(());
            }
            other => bail!("未知参数 {other}"),
        }
    }
    let dest = dest_dir(dir.as_deref())?;
    if uninstall_flag {
        uninstall(&dest)
    } else {
        install(&dest, quiet)
    }
}
