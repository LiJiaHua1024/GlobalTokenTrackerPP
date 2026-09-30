//! SourceAdapter — one per AI coding tool (spec §6).
//!
//! Two ingestion shapes:
//! - **Jsonl**: append-only log files; the engine owns byte cursors and calls
//!   `parse_jsonl` with only the NEW byte segment. `dedup_key`s must be stable
//!   across scans (message ids / byte offsets), since a streaming record may be
//!   completed by a later segment — the store's completeness-UPSERT merges them.
//! - **Sqlite**: read-only databases; the adapter keeps its own high-water mark
//!   in `sync_cursors.last_byte_offset` (reused as "max rowid/ts seen").

use crate::model::{QuotaSnapshot, UsageEvent};
use crate::store::Store;
use anyhow::Result;
use std::path::PathBuf;

pub mod antigravity;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod commandcode;
pub mod cursor;
pub mod devin;
pub mod dsh;
pub mod grok;
pub mod kimi_code;
pub mod minimax_code;
pub mod opencode;
pub mod qoder;
pub mod workbuddy;
pub mod zcode;

/// Coverage tier shown as a badge in the UI sources page (spec §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Exact per-request token data.
    Precise,
    /// Partial / estimated.
    Estimate,
    /// Metadata only (sessions exist but no billed usage).
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Jsonl,
    Sqlite,
}

#[derive(Debug, Clone)]
pub struct SourceItem {
    /// Stable cursor key (usually the canonical file path).
    pub key: String,
    pub path: PathBuf,
    pub kind: SourceKind,
}

#[derive(Debug, Default)]
pub struct ScanOutcome {
    pub events: Vec<UsageEvent>,
    pub quotas: Vec<QuotaSnapshot>,
    /// Records deliberately skipped (e.g. ZCode cross-provider rows that other
    /// adapters already count — counted, never silently dropped, spec §6.4).
    pub skipped: u64,
    /// Bytes actually consumed (≤ segment len; tail after last '\n' stays
    /// unconsumed so a half-written line is picked up next round).
    pub consumed: u64,
    /// Adapter-private resume state persisted into `sync_cursors.adapter_state`
    /// (e.g. Codex cumulative counters needed to delta future segments).
    pub new_state: Option<String>,
    pub notes: Vec<String>,
}

pub trait SourceAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn capability(&self) -> Capability;

    /// Files/databases this adapter ingests; empty when the tool is absent.
    fn discover(&self) -> Result<Vec<SourceItem>>;

    /// Top-level directories a live watcher should subscribe to (recursive).
    /// Wider than `discover` parents on purpose: brand-new session files/dirs
    /// must fire events too. May include dirs that don't exist yet — the
    /// watcher filters on `is_dir`.
    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![]
    }

    /// Parse the JSONL segment `data` starting at absolute `from` offset.
    /// `prior_state` is the adapter's resume blob saved on the previous scan.
    fn parse_jsonl(
        &self,
        _item: &SourceItem,
        _from: u64,
        _data: &[u8],
        _prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        Ok(ScanOutcome::default())
    }

    /// Scan a SQLite source (adapter-managed high-water mark via `store`).
    fn scan_sqlite(&self, _item: &SourceItem, _store: &Store) -> Result<ScanOutcome> {
        Ok(ScanOutcome::default())
    }
}

/// All adapters, ordered by spec priority. P0 set first.
/// Drop every adapter's "seen it, nothing changed" memo — the manual refresh
/// path (see `codebuddy::forget_scan_memo`).
pub fn forget_scan_memos() {
    codebuddy::forget_scan_memo();
}

pub fn registry() -> Vec<Box<dyn SourceAdapter>> {
    vec![
        Box::new(claude::Claude),
        Box::new(codex::Codex),
        Box::new(devin::Devin),
        Box::new(cursor::Cursor),
        Box::new(qoder::Qoder),
        Box::new(opencode::OpenCode),
        Box::new(zcode::ZCode),
        Box::new(grok::Grok),
        Box::new(workbuddy::WorkBuddy),
        Box::new(codebuddy::CodeBuddyIde::new()),
        Box::new(minimax_code::MiniMaxCode),
        Box::new(kimi_code::KimiCode),
        Box::new(cline::Cline),
        Box::new(commandcode::CommandCode),
        Box::new(antigravity::Antigravity),
        Box::new(dsh::Dsh),
    ]
}

/// Split `data` at the last newline: returns (complete segment, consumed len).
/// A file being appended may end mid-line — the incomplete tail is left for
/// the next scan by reporting `consumed` < data.len().
pub(crate) fn complete_lines(data: &[u8]) -> (&[u8], u64) {
    match data.iter().rposition(|&b| b == b'\n') {
        Some(i) => (&data[..=i], (i + 1) as u64),
        None if data.is_empty() => (data, 0),
        // No newline at all: a single still-growing line — consume nothing.
        None => (&data[..0], 0),
    }
}
