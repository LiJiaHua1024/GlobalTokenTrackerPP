//! Launch-at-login via the per-user Run key — no admin, no Task Scheduler.
//! The value points at the *running* exe (`current_exe`), so an installed
//! copy registers its install dir while a dev build registers target\…;
//! `--minimized` launches straight to the tray.

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "GlobalTokenTracker";

#[cfg(windows)]
fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" --minimized", exe.display()))
}

/// Current Run-key state — the registry is authoritative, `ui.json` only
/// caches what we last wrote.
#[cfg(windows)]
pub fn enabled() -> bool {
    enabled_in(RUN_KEY)
}

#[cfg(windows)]
fn enabled_in(subkey: &str) -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let name: Vec<u16> = VALUE_NAME.encode_utf16().chain([0]).collect();
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    status == 0 && size > 0
}

/// Write or delete the Run entry. Returns the new effective state.
#[cfg(windows)]
pub fn set(on: bool) -> bool {
    set_in(RUN_KEY, on)
}

#[cfg(windows)]
fn set_in(subkey: &str, on: bool) -> bool {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW,
        RegDeleteValueW, RegSetValueExW,
    };
    let key: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let name: Vec<u16> = VALUE_NAME.encode_utf16().chain([0]).collect();
    let mut hkey: HKEY = std::ptr::null_mut();
    let status = unsafe {
        // Create opens the existing key when present — idempotent for Run.
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            std::ptr::null_mut(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if status != 0 || hkey.is_null() {
        return enabled_in(subkey);
    }
    if on {
        if let Some(cmd) = command_line() {
            let wide: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
            unsafe {
                RegSetValueExW(
                    hkey,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    wide.as_ptr() as *const u8,
                    (wide.len() * 2) as u32,
                );
            }
        }
    } else {
        // Missing value is already "off" — not an error.
        unsafe {
            RegDeleteValueW(hkey, name.as_ptr());
        }
    }
    unsafe {
        RegCloseKey(hkey);
    }
    // Report the resulting state, not op success — a failed write leaves the
    // value absent (false) and a failed delete leaves it present (true).
    enabled_in(subkey)
}

#[cfg(not(windows))]
pub fn enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set(_on: bool) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    //! Real HKCU Run-key round trip — writes and removes the actual value
    //! (per-user hive, harmless; the test always ends with the key deleted).
    use super::*;

    /// Real registry round-trip on a scratch key. The actual Run key is NOT
    /// used — AV/Defender behavior monitoring intercepts Run-key writes
    /// (~30s stalls/blocks), which would make the test flaky. The GUI-level
    /// verification covers the real path; here we exercise the plumbing.
    #[test]
    fn run_key_round_trip() {
        const SCRATCH: &str = r"Software\GlobalTokenTracker_test";
        let _ = set_in(SCRATCH, false);
        assert!(!enabled_in(SCRATCH));
        assert!(set_in(SCRATCH, true), "write blocked");
        assert!(enabled_in(SCRATCH));
        let got = std::process::Command::new("reg")
            .args(["query", &format!(r"HKCU\{SCRATCH}"), "/v", VALUE_NAME])
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&got.stdout);
        assert!(out.contains("--minimized"), "Run value missing arg: {out}");
        assert!(out.contains("globaltokentracker"), "exe path: {out}");
        assert!(!set_in(SCRATCH, false));
        assert!(!enabled_in(SCRATCH));
    }
}
