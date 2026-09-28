//! CPU scheduling hints for hybrid (P/E-core) hosts — Windows 11 EcoQoS.
//!
//! Three levels, mapped onto real work in the app:
//!
//! - [`efficiency_thread`] — mark the calling thread "efficiency preferred"
//!   (`ThreadPowerThrottling`). Used by every non-UI background worker
//!   (scan/ingest, file watcher, tray event pump, network polls) so bursty
//!   background load lands on E-cores and stays off the interactive P-cores.
//! - [`efficiency_process`] — process-wide "Efficiency Mode" as Task Manager
//!   applies it: `ProcessPowerThrottling` EcoQoS **plus** `IDLE_PRIORITY_CLASS`
//!   **plus** the timer-resolution opt-out flag, and a low `ProcessMemoryPriority`
//!   so our pages are reclaimed first under memory pressure. Toggled on when
//!   the window hides to the tray — a tray-only app should idle like an
//!   efficiency-mode process — and toggled off when it returns to the
//!   foreground, where normal class + default QoS prefer P-cores.
//! - [`worker`] — convenience: `SetThreadDescription` name + EcoQoS mark,
//!   so background threads are also identifiable in Process Explorer/ETW.
//!
//! On non-Windows (macOS/Linux port) these are no-ops — the QoS concept is
//! Windows-specific; Apple silicon cores are scheduled by the OS scheduler
//! (GCD QoS classes would be the port-side equivalent). `GTT_NO_ECO=1`
//! disables all marking for diagnostics. Calls fail silently on hosts
//! without EcoQoS (pre-Win11, non-hybrid CPUs) — the hints degrade to no-ops.

/// Name + EcoQoS-mark the calling thread — top of every background closure.
pub fn worker(name: &str) {
    name_thread(name);
    efficiency_thread();
}

/// `SetThreadDescription` — diagnostic name visible in debuggers/ETW.
pub fn name_thread(name: &str) {
    #[cfg(windows)]
    if !disabled() {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadDescription};
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            SetThreadDescription(GetCurrentThread(), wide.as_ptr());
        }
    }
}

/// Mark the calling thread EcoQoS — the scheduler prefers efficiency cores.
/// Per-thread (not inherited): each worker must mark itself.
pub fn efficiency_thread() {
    #[cfg(windows)]
    if !disabled() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadInformation, THREAD_POWER_THROTTLING_CURRENT_VERSION,
            THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
            ThreadPowerThrottling,
        };
        // ControlMask must be EXECUTION_SPEED (explicit control) — the
        // documented "0 = system decides" form returns INVALID_PARAMETER on
        // Win11 (verified on 26200). GetThreadInformation for this class is
        // unsupported (write-only), so callers cannot read the state back.
        let state = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        };
        unsafe {
            let ok = SetThreadInformation(
                GetCurrentThread(),
                ThreadPowerThrottling,
                &state as *const _ as *const _,
                size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
            );
            if ok == 0 {
                log_fail("SetThreadInformation");
            }
        }
    }
}

/// Process-wide Efficiency Mode switch — `true` when the window hides to the
/// tray, `false` on return to the foreground. Mirrors what Task Manager's
/// "Efficiency mode" applies to a process:
/// 1. `ProcessPowerThrottling` EcoQoS — scheduler prefers efficiency cores.
/// 2. `IDLE_PRIORITY_CLASS` — every thread drops to idle base priority so
///    background scans/watches/timers yield to any foreground work on the
///    machine. Back to `NORMAL_PRIORITY_CLASS` on restore.
/// 3. `IGNORE_TIMER_RESOLUTION` — a hidden process must not keep the system
///    timer at high resolution (that's a battery drain); foreground restores
///    the default so animation ticks can still request tight cadence.
/// 4. `MEMORY_PRIORITY_LOW` — our pages are reclaimed first under pressure;
///    unlike `EmptyWorkingSet` this faults nothing out eagerly, so restore
///    doesn't pay a page-in storm.
pub fn efficiency_process(on: bool) {
    #[cfg(windows)]
    if !disabled() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, IDLE_PRIORITY_CLASS, MEMORY_PRIORITY_INFORMATION,
            MEMORY_PRIORITY_LOW, MEMORY_PRIORITY_NORMAL, NORMAL_PRIORITY_CLASS,
            PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
            ProcessMemoryPriority, ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
        };
        let control = PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;
        // Explicit control only: ControlMask names the aspects, StateMask the
        // values — ControlMask=0 is INVALID_PARAMETER on Win11 (verified on
        // 26200; see SetThreadInformation notes above).
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: control,
            StateMask: if on { control } else { 0 },
        };
        let prio = if on {
            IDLE_PRIORITY_CLASS
        } else {
            NORMAL_PRIORITY_CLASS
        };
        let mem = MEMORY_PRIORITY_INFORMATION {
            MemoryPriority: if on {
                MEMORY_PRIORITY_LOW
            } else {
                MEMORY_PRIORITY_NORMAL
            },
        };
        unsafe {
            let ok = SetProcessInformation(
                GetCurrentProcess(),
                ProcessPowerThrottling,
                &state as *const _ as *const _,
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            );
            if ok == 0 {
                log_fail("SetProcessInformation(PowerThrottling)");
            }
            if SetPriorityClass(GetCurrentProcess(), prio) == 0 {
                log_fail("SetPriorityClass");
            }
            let ok = SetProcessInformation(
                GetCurrentProcess(),
                ProcessMemoryPriority,
                &mem as *const _ as *const _,
                size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
            );
            if ok == 0 {
                log_fail("SetProcessInformation(MemoryPriority)");
            }
        }
    }
}

/// Report the first failure per process so a host rejecting EcoQoS is
/// diagnosable instead of silently degrading — subsequent failures of the
/// same API stay quiet (every worker calls this on spawn).
#[cfg(windows)]
fn log_fail(api: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        eprintln!("[power] {api} rejected by host, err={err}");
    }
}

#[cfg(windows)]
fn disabled() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("GTT_NO_ECO").is_some())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::*;

    #[test]
    fn eco_set_calls_succeed() {
        // Get*Information is unsupported for the power-throttling classes on
        // Win11 (write-only) — assert the documented Set calls return true.
        unsafe {
            let st = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            };
            assert_ne!(
                SetThreadInformation(
                    GetCurrentThread(),
                    ThreadPowerThrottling,
                    &st as *const _ as *const _,
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                ),
                0
            );
            let pst = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            };
            assert_ne!(
                SetProcessInformation(
                    GetCurrentProcess(),
                    ProcessPowerThrottling,
                    &pst as *const _ as *const _,
                    size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                ),
                0
            );
        }
        // exercised paths must not panic
        efficiency_thread();
        efficiency_process(false);
    }

    #[test]
    fn efficiency_mode_toggles_priority_class() {
        // SetPriorityClass/GetPriorityClass IS readable back — the one leg of
        // efficiency mode with a verifiable read path.
        unsafe {
            let base = GetPriorityClass(GetCurrentProcess());
            efficiency_process(true);
            assert_eq!(GetPriorityClass(GetCurrentProcess()), IDLE_PRIORITY_CLASS);
            efficiency_process(false);
            assert_eq!(GetPriorityClass(GetCurrentProcess()), NORMAL_PRIORITY_CLASS);
            if base != NORMAL_PRIORITY_CLASS {
                SetPriorityClass(GetCurrentProcess(), base);
            }
        }
    }

    #[test]
    fn thread_name_is_readable_back() {
        name_thread("gtt-probe");
        unsafe {
            let mut out: windows_sys::core::PWSTR = std::ptr::null_mut();
            let hr = GetThreadDescription(GetCurrentThread(), &mut out);
            assert!(hr >= 0, "GetThreadDescription failed {hr:#x}");
            let s = {
                let mut len = 0;
                while *out.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(out, len))
            };
            assert_eq!(s, "gtt-probe");
        }
    }
}
