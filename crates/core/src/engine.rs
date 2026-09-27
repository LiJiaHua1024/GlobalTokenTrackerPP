//! Engine — orchestrates discover → cursor-gated read → parse → normalize →
//! price → upsert (spec §4 pipeline). Parsing runs on rayon; the store is the
//! single writer.

use crate::adapters::{self, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::pricing::PriceBook;
use crate::store::{CursorAction, Store};
use crate::viewmodel::local_utc_offset;
use anyhow::Result;
use rayon::prelude::*;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, warn};

pub struct Engine {
    pub store: Store,
    pub prices: PriceBook,
    adapters: Vec<Box<dyn SourceAdapter>>,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub files_seen: u64,
    pub files_scanned: u64,
    pub files_pinned: u64,
    pub events_ingested: u64,
    pub events_merged: u64, // upserts that collapsed into existing rows
    pub events_skipped: u64,
    pub quotas: u64,
    pub errors: Vec<String>,
}

impl Engine {
    pub fn new(store: Store) -> Result<Self> {
        let prices = PriceBook::load(&store)?;
        Ok(Self {
            store,
            prices,
            adapters: adapters::registry(),
        })
    }

    /// One full incremental pass over every enabled adapter.
    pub fn scan_once(&self) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        for adapter in &self.adapters {
            let r = self.scan_adapter(adapter.as_ref());
            merge(&mut report, r?);
        }
        self.refresh_rollups(&report);
        // Safety-net snapshot — time-throttled inside, so this costs one
        // KV read per pass and a VACUUM INTO once a day. Never fails a scan.
        if let Err(e) = self.store.maybe_backup() {
            tracing::warn!("ledger backup failed: {e}");
        }
        Ok(report)
    }

    pub fn scan_source(&self, id: &str) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        for adapter in &self.adapters {
            if adapter.id() == id {
                merge(&mut report, self.scan_adapter(adapter.as_ref())?);
            }
        }
        self.refresh_rollups(&report);
        Ok(report)
    }

    /// Keep daily_rollups in sync — only when this pass ingested something.
    /// Rollup failure must not fail the scan (derived data can be rebuilt).
    fn refresh_rollups(&self, report: &ScanReport) {
        if report.events_ingested > 0
            && let Err(e) = self.store.rebuild_rollups(&local_utc_offset())
        {
            tracing::warn!("rollup rebuild failed: {e}");
        }
    }

    fn scan_adapter(&self, adapter: &dyn SourceAdapter) -> Result<ScanReport> {
        let items = adapter.discover()?;
        let report = self.scan_items(adapter, &items)?;
        // Source-health bookkeeping for the data-sources page.
        let _ = self.store.touch_source(
            adapter.id(),
            report.files_seen,
            report.events_ingested,
            report.errors.first().map(String::as_str),
        );
        Ok(report)
    }

    fn scan_items(&self, adapter: &dyn SourceAdapter, items: &[SourceItem]) -> Result<ScanReport> {
        let mut report = ScanReport {
            files_seen: items.len() as u64,
            ..Default::default()
        };
        let jsonl: Vec<&SourceItem> = items
            .iter()
            .filter(|i| i.kind == SourceKind::Jsonl)
            .collect();
        let sqlite: Vec<&SourceItem> = items
            .iter()
            .filter(|i| i.kind == SourceKind::Sqlite)
            .collect();

        // Phase 1: decide actions & read byte segments (sequential, cheap).
        let pinned = AtomicU64::new(0);
        let mut segments: Vec<(&SourceItem, u64, Option<String>, Vec<u8>)> = Vec::new();
        for item in &jsonl {
            let meta = match std::fs::metadata(&item.path) {
                Ok(m) => m,
                Err(e) => {
                    report
                        .errors
                        .push(format!("{}: {}", item.path.display(), e));
                    continue;
                }
            };
            let action = self
                .store
                .cursor_action(&item.path, &item.key, meta.len())?;
            match action {
                CursorAction::Unchanged => {}
                CursorAction::SkipPinned { eof } => {
                    pinned.fetch_add(1, Ordering::Relaxed);
                    let mtime = file_mtime_ms(&meta);
                    self.store
                        .pin_cursor_eof(adapter.id(), &item.key, &item.path, eof, mtime)?;
                    warn!(file = %item.path.display(), "truncated/rotated — cursor pinned to EOF");
                }
                CursorAction::Full => {
                    if let Some(seg) = read_segment(&item.path, 0, meta.len(), &mut report.errors) {
                        segments.push((item, 0, None, seg));
                    }
                }
                CursorAction::Append { from } => {
                    if let Some(seg) =
                        read_segment(&item.path, from, meta.len(), &mut report.errors)
                    {
                        let state = self.store.load_cursor(&item.key)?.state;
                        segments.push((item, from, state, seg));
                    }
                }
            }
        }
        report.files_pinned = pinned.load(Ordering::Relaxed);

        // Phase 2: parse segments in parallel (the hot path for GB-scale logs).
        let parsed: Vec<(&SourceItem, u64, Result<ScanOutcome>)> = segments
            .into_par_iter()
            .map(|(item, from, state, data)| {
                (
                    item,
                    from,
                    adapter.parse_jsonl(item, from, &data, state.as_deref()),
                )
            })
            .collect();

        // Phase 3: single-writer ingestion.
        for (item, from, res) in parsed {
            match res {
                Ok(outcome) => {
                    self.ingest(
                        adapter.id(),
                        adapter.capability(),
                        outcome.events,
                        outcome.quotas,
                        &mut report,
                    );
                    report.events_skipped += outcome.skipped;
                    report.files_scanned += 1;
                    let end = from + outcome.consumed;
                    let mtime = std::fs::metadata(&item.path)
                        .ok()
                        .map(|m| file_mtime_ms(&m))
                        .unwrap_or(0);
                    self.store.save_cursor(
                        adapter.id(),
                        &item.key,
                        &item.path,
                        end,
                        mtime,
                        outcome.new_state.as_deref(),
                    )?;
                }
                Err(e) => report
                    .errors
                    .push(format!("{}: {e:#}", item.path.display())),
            }
        }

        // SQLite sources (adapter-managed watermarks).
        for item in sqlite {
            match adapter.scan_sqlite(item, &self.store) {
                Ok(outcome) => {
                    self.ingest(
                        adapter.id(),
                        adapter.capability(),
                        outcome.events,
                        outcome.quotas,
                        &mut report,
                    );
                    report.events_skipped += outcome.skipped;
                    report.files_scanned += 1;
                }
                Err(e) => report
                    .errors
                    .push(format!("{}: {e:#}", item.path.display())),
            }
        }
        Ok(report)
    }

    fn ingest(
        &self,
        adapter_id: &str,
        capability: crate::adapters::Capability,
        events: Vec<crate::model::UsageEvent>,
        quotas: Vec<crate::model::QuotaSnapshot>,
        report: &mut ScanReport,
    ) {
        for mut ev in events {
            // Metadata-tier adapters (Cursor/Qoder) deliberately emit
            // zero-token activity records — the observed session IS the
            // information. Admit them when they carry a timestamp; precise
            // adapters keep the strict billable gate to filter noise.
            let tracked = ev.is_billable()
                || (capability == crate::adapters::Capability::Metadata
                    && ev.ts_start.is_some());
            if !tracked {
                report.events_skipped += 1;
                continue;
            }
            // Price + resolve pricing_model on the way in (idempotent).
            self.prices.apply(&mut ev);
            match self.store.upsert_event(&ev) {
                Ok(true) => report.events_ingested += 1,
                Ok(false) => report.events_merged += 1,
                Err(e) => report
                    .errors
                    .push(format!("{adapter_id} upsert {}: {e:#}", ev.dedup_key)),
            }
        }
        for q in quotas {
            match self.store.insert_quota(&q) {
                Ok(true) => report.quotas += 1,
                Ok(false) => {}
                Err(e) => report.errors.push(format!("{adapter_id} quota: {e:#}")),
            }
        }
    }

    /// Directories a live watcher should subscribe to — union of adapter roots,
    /// deduped, limited to dirs that currently exist.
    pub fn watch_roots(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for a in &self.adapters {
            for r in a.watch_roots() {
                if r.is_dir() && !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        out
    }

    pub fn adapter_ids(&self) -> Vec<&'static str> {
        self.adapters.iter().map(|a| a.id()).collect()
    }
}

fn merge(dst: &mut ScanReport, src: ScanReport) {
    dst.files_seen += src.files_seen;
    dst.files_scanned += src.files_scanned;
    dst.files_pinned += src.files_pinned;
    dst.events_ingested += src.events_ingested;
    dst.events_merged += src.events_merged;
    dst.events_skipped += src.events_skipped;
    dst.quotas += src.quotas;
    dst.errors.extend(src.errors);
}

fn file_mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn read_segment(path: &Path, from: u64, to: u64, errors: &mut Vec<String>) -> Option<Vec<u8>> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            errors.push(format!("{}: {e}", path.display()));
            return None;
        }
    };
    if f.seek(SeekFrom::Start(from)).is_err() {
        errors.push(format!("{}: seek {from} failed", path.display()));
        return None;
    }
    let mut buf = Vec::with_capacity((to - from) as usize);
    match f.take(to - from).read_to_end(&mut buf) {
        Ok(_) => {
            debug!(file = %path.display(), from, to, "segment read");
            Some(buf)
        }
        Err(e) => {
            errors.push(format!("{}: {e}", path.display()));
            None
        }
    }
}
