//! Source discovery helpers. `notify` watching + 60 s fallback polling is
//! layered on in the UI/service shell — the engine's `scan_once` is already
//! incremental, so polling is just "call it again".

use std::path::{Path, PathBuf};

/// Recursively collect files under `root` matching `ext` (e.g. "jsonl"),
/// skipping unreadable dirs. Depth-limited to keep pathological trees cheap.
pub fn collect_files(root: &Path, ext: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(root, ext, max_depth, &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            collect(&p, ext, depth - 1, out);
        } else if ft.is_file() && p.extension().is_some_and(|e| e == ext) {
            out.push(p);
        }
    }
}

/// `~/<sub>` resolution.
pub fn home(sub: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(sub)
}
