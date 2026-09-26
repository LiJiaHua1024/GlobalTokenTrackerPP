//! Live source watching: `notify` recursive watch over adapter `watch_roots`,
//! debounced so a burst of log writes collapses into one rescan. The 30 s
//! timer in `main` stays as the fallback for events missed while a scan is
//! in flight.

use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use windows_reactor::CancellationToken;

/// Block until any watched root reports a filesystem event, then drain ~1.2 s
/// of trailing writes. `false` when nothing is watchable or the task is
/// cancelled (e.g. window closing).
pub fn wait_for_change(roots: &[PathBuf], token: &CancellationToken) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let Ok(mut watcher) = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    }) else {
        return false;
    };
    let mut watched = false;
    for root in roots {
        if watcher.watch(root, RecursiveMode::Recursive).is_ok() {
            watched = true;
        }
    }
    if !watched {
        return false;
    }
    // Sidecar files that OUR OWN read-only opens touch: a WAL read still
    // updates `*-shm` (shared-memory lock state). If we let those through, the
    // scan we trigger touches -shm → another event → infinite self-rescan.
    // `*-wal` stays watched: real writes land there before any checkpoint.
    fn ignorable(p: &std::path::Path) -> bool {
        let s = p.to_string_lossy();
        s.ends_with("-shm") || s.ends_with("-journal") || s.ends_with(".tmp")
    }
    let mut hits: Vec<String> = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(ev)) => hits.extend(
                ev.paths
                    .iter()
                    .filter(|p| !ignorable(p))
                    .map(|p| p.display().to_string()),
            ),
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) if token.is_cancelled() => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if !hits.is_empty() {
            break;
        }
    }
    // Drain trailing events so a burst of writes = one rescan; cap total
    // debounce at 8 s so a continuously-writing tool doesn't starve the UI.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while let Ok(Ok(ev)) = rx.recv_timeout(Duration::from_millis(1200)) {
        if std::time::Instant::now() > deadline {
            break;
        }
        hits.extend(
            ev.paths
                .iter()
                .filter(|p| !ignorable(p))
                .map(|p| p.display().to_string()),
        );
    }
    hits.sort();
    hits.dedup();
    crate::diag!("[watch] {} path(s): {}", hits.len(), hits.join(" | "));
    !token.is_cancelled()
}
