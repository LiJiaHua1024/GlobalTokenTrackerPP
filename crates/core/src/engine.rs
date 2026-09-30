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

/// Cap on raw segment bytes held between read and ingest. A single file
/// larger than this still loads whole (cursors need contiguous bytes) —
/// the bound is on the batch, not on any one source.
const SEGMENT_BUDGET: u64 = 256 << 20;

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
    /// Local day indices (fixed-offset frame, matching the rollup SQL)
    /// touched by rows actually written this pass — drives the partial
    /// rollup rebuild.
    pub rollup_days: std::collections::BTreeSet<i64>,
    /// A written row lacked ts_start → its day is unrecoverable → caller
    /// must fall back to a full rollup rebuild.
    pub rollup_full: bool,
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
    /// Rebuilds only the local days the writes actually touched; a full
    /// rebuild stays as the fallback for ts-less rows or huge backfills.
    /// Rollup failure must not fail the scan (derived data can be rebuilt).
    fn refresh_rollups(&self, report: &ScanReport) {
        if report.events_ingested == 0 {
            return;
        }
        let offset = local_utc_offset();
        let r = if report.rollup_full || report.rollup_days.len() > 400 {
            self.store.rebuild_rollups(&offset)
        } else {
            self.store.rebuild_rollup_days(&report.rollup_days, &offset)
        };
        if let Err(e) = r {
            tracing::warn!("rollup rebuild failed: {e}");
        }
    }

    fn scan_adapter(&self, adapter: &dyn SourceAdapter) -> Result<ScanReport> {
        let t0 = std::time::Instant::now();
        let items = adapter.discover()?;
        let report = self.scan_items(adapter, &items)?;
        debug!(
            adapter = adapter.id(),
            ms = t0.elapsed().as_millis() as u64,
            files = report.files_seen,
            scanned = report.files_scanned,
            "adapter scan"
        );
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
        // In-flight segment bytes are budgeted — a fresh install facing
        // GBs of accumulated logs must not try to hold them all at once.
        let pinned = AtomicU64::new(0);
        let mut segments: Vec<(&SourceItem, u64, Option<String>, Vec<u8>)> = Vec::new();
        let mut in_flight: u64 = 0;
        // The stat of every session file is the bulk of a no-change pass (515
        // codex files ≈ 24ms sequentially, ~47µs each) and the calls are
        // independent: fan them out on the eco pool, consume in order.
        let metas: Vec<std::io::Result<std::fs::Metadata>> = eco_pool().install(|| {
            jsonl
                .par_iter()
                .map(|i| std::fs::metadata(&i.path))
                .collect()
        });
        for (item, meta) in jsonl.iter().zip(metas) {
            let meta = match meta {
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
                        if !segments.is_empty() && in_flight + seg.len() as u64 > SEGMENT_BUDGET {
                            self.flush_segments(
                                adapter,
                                std::mem::take(&mut segments),
                                &mut report,
                            )?;
                            in_flight = 0;
                        }
                        in_flight += seg.len() as u64;
                        segments.push((item, 0, None, seg));
                    }
                }
                CursorAction::Append { from } => {
                    if let Some(seg) =
                        read_segment(&item.path, from, meta.len(), &mut report.errors)
                    {
                        let state = self.store.load_cursor(&item.key)?.state;
                        if !segments.is_empty() && in_flight + seg.len() as u64 > SEGMENT_BUDGET {
                            self.flush_segments(
                                adapter,
                                std::mem::take(&mut segments),
                                &mut report,
                            )?;
                            in_flight = 0;
                        }
                        in_flight += seg.len() as u64;
                        segments.push((item, from, state, seg));
                    }
                }
            }
        }
        report.files_pinned = pinned.load(Ordering::Relaxed);
        if !segments.is_empty() {
            self.flush_segments(adapter, segments, &mut report)?;
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

    /// Phase 2+3: parallel parse then single-writer ingest for one batch
    /// of segments. Batched by `SEGMENT_BUDGET` so in-flight bytes stay
    /// bounded regardless of how much log data accumulated on disk.
    fn flush_segments(
        &self,
        adapter: &dyn SourceAdapter,
        segments: Vec<(&SourceItem, u64, Option<String>, Vec<u8>)>,
        report: &mut ScanReport,
    ) -> Result<()> {
        let parsed: Vec<(&SourceItem, u64, Result<ScanOutcome>)> = eco_pool().install(|| {
            segments
                .into_par_iter()
                .map(|(item, from, state, data)| {
                    (
                        item,
                        from,
                        adapter.parse_jsonl(item, from, &data, state.as_deref()),
                    )
                })
                .collect()
        });
        for (item, from, res) in parsed {
            match res {
                Ok(outcome) => {
                    self.ingest(
                        adapter.id(),
                        adapter.capability(),
                        outcome.events,
                        outcome.quotas,
                        report,
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
        Ok(())
    }

    fn ingest(
        &self,
        adapter_id: &str,
        capability: crate::adapters::Capability,
        events: Vec<crate::model::UsageEvent>,
        quotas: Vec<crate::model::QuotaSnapshot>,
        report: &mut ScanReport,
    ) {
        // Day buckets use the same fixed-offset frame as the rollup SQL;
        // computed once per ingest batch, not per event.
        let off_ms = crate::viewmodel::utc_offset_ms(&local_utc_offset()).unwrap_or(0);
        for mut ev in events {
            // Metadata-tier adapters (Cursor/Qoder) deliberately emit
            // zero-token activity records — the observed session IS the
            // information. Admit them when they carry a timestamp; precise
            // adapters keep the strict billable gate to filter noise.
            let tracked = ev.is_billable()
                || (capability == crate::adapters::Capability::Metadata && ev.ts_start.is_some());
            if !tracked {
                report.events_skipped += 1;
                continue;
            }
            // Price + resolve pricing_model on the way in (idempotent).
            self.prices.apply(&mut ev);
            match self.store.upsert_event(&ev) {
                Ok(true) => {
                    report.events_ingested += 1;
                    match ev.ts_start {
                        Some(ts) => {
                            report
                                .rollup_days
                                .insert((ts + off_ms).div_euclid(86_400_000));
                        }
                        None => report.rollup_full = true,
                    }
                }
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
    dst.rollup_days.extend(src.rollup_days);
    dst.rollup_full |= src.rollup_full;
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

/// Parse pool pinned to efficiency cores. The default rayon pool threads
/// can't be QoS-marked (spawned lazily, no handle access); `spawn_handler`
/// lets every worker name + EcoQoS itself so ingest parse bursts stay off
/// the interactive P-cores. Bounded to half the logical CPUs (min 2): the
/// EcoQoS hint prefers E-cores, and a smaller pool avoids oversubscribing
/// them and spilling back onto P-cores. On non-hybrid hosts the marks are
/// no-ops and the cap just halves peak parse width — acceptable for a
/// background ingest stage.
fn eco_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let width = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(2))
            .unwrap_or(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(width)
            .spawn_handler(|t| {
                std::thread::Builder::new().spawn(move || {
                    crate::power::worker("gtt-parse");
                    t.run();
                })?;
                Ok(())
            })
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().build().unwrap())
    })
}
