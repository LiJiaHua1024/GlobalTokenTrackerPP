//! Installed-font enumeration via DirectWrite — the same engine that renders
//! chart text, so every family name listed is guaranteed to resolve in
//! `TextFormat`. Enumerated once per process; a font installed while the app
//! runs shows up after restart.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::OnceLock;

use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory, IDWriteLocalizedStrings,
};
use windows::core::{BOOL, PCWSTR, w};

/// Sorted family names usable by DirectWrite, cached for the process.
pub fn families() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(enumerate)
}

fn enumerate() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) else {
            return out;
        };
        let mut collection = None;
        if factory
            .GetSystemFontCollection(&mut collection, true)
            .is_err()
        {
            return out;
        }
        let Some(collection) = collection else {
            return out;
        };
        let locale = user_locale();
        for i in 0..collection.GetFontFamilyCount() {
            let Ok(family) = collection.GetFontFamily(i) else {
                continue;
            };
            let Ok(names) = family.GetFamilyNames() else {
                continue;
            };
            if let Some(name) = localized_name(&names, &locale) {
                out.push(name);
            }
        }
    }
    out.sort_by_cached_key(|s| s.to_lowercase());
    out.dedup();
    out
}

/// UI locale ("zh-CN") as a NUL-terminated wide string; empty when the call
/// fails — `localized_name` then skips straight to the fallbacks.
fn user_locale() -> Vec<u16> {
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n <= 0 {
        Vec::new()
    } else {
        buf[..n as usize].to_vec()
    }
}

/// Family name in the user's locale, else en-US, else the first entry —
/// DirectWrite accepts any localized alias, we just display the friendliest.
fn localized_name(names: &IDWriteLocalizedStrings, locale: &[u16]) -> Option<String> {
    unsafe {
        let mut index = 0u32;
        let mut exists = BOOL(0);
        if !locale.is_empty() {
            let _ = names.FindLocaleName(PCWSTR(locale.as_ptr()), &mut index, &mut exists);
        }
        if !exists.as_bool() {
            let _ = names.FindLocaleName(w!("en-us"), &mut index, &mut exists);
        }
        if !exists.as_bool() {
            index = 0;
        }
        let len = names.GetStringLength(index).ok()? as usize;
        let mut buf = vec![0u16; len + 1]; // GetStringLength excludes the NUL
        names.GetString(index, &mut buf).ok()?;
        buf.truncate(len);
        Some(OsString::from_wide(&buf).to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_system_fonts() {
        let families = families();
        assert!(
            families.len() > 10,
            "expected dozens of families: {families:?}"
        );
        // Segoe UI ships with every supported Windows version.
        assert!(
            families.iter().any(|f| f.eq_ignore_ascii_case("Segoe UI")),
            "Segoe UI missing: {families:?}"
        );
    }
}
