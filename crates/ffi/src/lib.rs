#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::sync::Mutex;

use globaltokentracker_core::store::{default_db_path, Store};
use globaltokentracker_core::viewmodel::Range;
use globaltokentracker_core::Engine;
use serde::Serialize;

pub struct GttContext {
    inner: Mutex<Engine>,
}

#[derive(Serialize)]
struct ScanResultJson {
    ok: bool,
    files_seen: u64,
    files_scanned: u64,
    events_ingested: u64,
    events_merged: u64,
    events_skipped: u64,
    quotas: u64,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct ErrorJson {
    ok: bool,
    error: String,
}

fn to_c_string(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

fn to_json_c_string<T: Serialize>(val: &T) -> *mut c_char {
    let s = serde_json::to_string(val).unwrap_or_else(|e| {
        format!(r#"{{"ok":false,"error":{}}}"#, serde_json::to_string(&e.to_string()).unwrap_or_default())
    });
    to_c_string(s)
}

fn to_error_c_string(e: impl ToString) -> *mut c_char {
    let err = ErrorJson {
        ok: false,
        error: e.to_string(),
    };
    to_c_string(serde_json::to_string(&err).unwrap_or_default())
}

unsafe fn parse_opt_str(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let c_str = unsafe { CStr::from_ptr(ptr) };
    c_str.to_str().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

unsafe fn parse_string_list(ptr: *const c_char) -> Option<Vec<String>> {
    let raw = unsafe { parse_opt_str(ptr) }?;
    serde_json::from_str::<Vec<String>>(&raw).ok()
}

/// Returns default ledger database path as a C string.
#[unsafe(no_mangle)]
pub extern "C" fn gtt_default_db_path() -> *mut c_char {
    to_c_string(default_db_path().to_string_lossy().to_string())
}

/// Frees a C string allocated by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

/// Opens the engine and database. Pass NULL for default db path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_engine_open(db_path: *const c_char) -> *mut GttContext {
    let path = if let Some(p) = unsafe { parse_opt_str(db_path) } {
        PathBuf::from(p)
    } else {
        default_db_path()
    };

    let store = match Store::open(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[gtt_ffi] Failed to open store: {e}");
            return std::ptr::null_mut();
        }
    };

    let engine = match Engine::new(store) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[gtt_ffi] Failed to init engine: {e}");
            return std::ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(GttContext {
        inner: Mutex::new(engine),
    }))
}

/// Closes the engine context and frees its resources.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_engine_close(ctx: *mut GttContext) {
    if !ctx.is_null() {
        unsafe {
            drop(Box::from_raw(ctx));
        }
    }
}

/// Runs a single incremental scan pass across all configured tools.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_engine_scan(ctx: *mut GttContext) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    match engine.scan_once() {
        Ok(report) => {
            let res = ScanResultJson {
                ok: true,
                files_seen: report.files_seen,
                files_scanned: report.files_scanned,
                events_ingested: report.events_ingested,
                events_merged: report.events_merged,
                events_skipped: report.events_skipped,
                quotas: report.quotas,
                errors: report.errors,
            };
            to_json_c_string(&res)
        }
        Err(e) => to_error_c_string(e),
    }
}

/// Fetches overview statistics, pie/donut chart data, and trends.
/// range_key: "today" | "week" | "month" | "all" | "custom"
/// filter_apps_json: optional JSON array e.g. ["claude", "cursor"]
/// filter_models_json: optional JSON array e.g. ["claude-3-7-sonnet"]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_get_overview(
    ctx: *mut GttContext,
    range_key: *const c_char,
    custom_start_ms: i64,
    custom_end_ms: i64,
    filter_apps_json: *const c_char,
    filter_models_json: *const c_char,
) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let r_key = unsafe { parse_opt_str(range_key) }.unwrap_or_else(|| "week".to_string());
    let range = match r_key.as_str() {
        "today" => Range::Today,
        "month" => Range::Month,
        "all" => Range::All,
        "custom" => Range::custom(custom_start_ms, custom_end_ms),
        _ => Range::Week,
    };

    let apps = unsafe { parse_string_list(filter_apps_json) };
    let models = unsafe { parse_string_list(filter_models_json) };

    match engine.store.overview(range, apps.as_deref(), models.as_deref()) {
        Ok(vm) => to_json_c_string(&vm),
        Err(e) => to_error_c_string(e),
    }
}

/// Fetches event detail rows with pagination and filters.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_get_details(
    ctx: *mut GttContext,
    page: i64,
    page_size: i64,
    filter_apps_json: *const c_char,
    filter_models_json: *const c_char,
) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let apps = unsafe { parse_string_list(filter_apps_json) };
    let models = unsafe { parse_string_list(filter_models_json) };

    match engine.store.detail(page, page_size, apps.as_deref(), models.as_deref()) {
        Ok(vm) => to_json_c_string(&vm),
        Err(e) => to_error_c_string(e),
    }
}

/// Fetches source log discovery and sync health.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_get_sources(ctx: *mut GttContext) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    match engine.store.source_health() {
        Ok(sources) => to_json_c_string(&sources),
        Err(e) => to_error_c_string(e),
    }
}

/// Fetches pricing rows.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_get_prices(ctx: *mut GttContext, limit: usize) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let lim = if limit == 0 { 5000 } else { limit } as i64;
    match engine.store.price_rows(lim) {
        Ok(prices) => to_json_c_string(&prices),
        Err(e) => to_error_c_string(e),
    }
}

/// Refreshes prices from online sources and reprices unpriced events.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_update_prices(ctx: *mut GttContext) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    match globaltokentracker_core::pricing::refresh(&engine.store) {
        Ok(report) => {
            #[derive(Serialize)]
            struct PricingResult {
                ok: bool,
                models_dev: usize,
                litellm: usize,
                llmpricing: usize,
                repriced: u64,
            }
            to_json_c_string(&PricingResult {
                ok: true,
                models_dev: report.models_dev,
                litellm: report.litellm,
                llmpricing: report.llmpricing,
                repriced: report.repriced,
            })
        }
        Err(e) => to_error_c_string(e),
    }
}

/// Polls vendor quota endpoints once and stores latest quotas.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtt_poll_quotas(ctx: *mut GttContext) -> *mut c_char {
    if ctx.is_null() {
        return to_error_c_string("Engine context is null");
    }
    let gtt = unsafe { &*ctx };
    let engine = match gtt.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let mut count = 0;
    for outcome in globaltokentracker_core::quota::poll_all() {
        for q in &outcome.quotas {
            if let Ok(true) = engine.store.insert_quota(q) {
                count += 1;
            }
        }
    }

    #[derive(Serialize)]
    struct QuotaResult {
        ok: bool,
        updated: usize,
    }
    to_json_c_string(&QuotaResult { ok: true, updated: count })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_memory_and_overview() {
        let store = Store::open_memory().unwrap();
        let engine = Engine::new(store).unwrap();
        let ctx = Box::into_raw(Box::new(GttContext {
            inner: Mutex::new(engine),
        }));

        let c_str = unsafe {
            gtt_get_overview(
                ctx,
                std::ptr::null(),
                0,
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert!(!c_str.is_null());
        let json_str = unsafe { CStr::from_ptr(c_str) }.to_str().unwrap();
        assert!(json_str.contains("range"));
        assert!(json_str.contains("span"));
        unsafe {
            gtt_free_string(c_str);
            gtt_engine_close(ctx);
        }
    }
}
