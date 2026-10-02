//! Antigravity adapter — Google's agentic coding tool in all its shells: the
//! Antigravity 2.0 app, the IDE extension and the `agy` CLI. They share one
//! on-disk format, so one adapter covers them.
//!
//! ## Where the data is
//!
//! Every conversation is its own SQLite database, `<uuid>.db`, in any of
//! `<base>/antigravity-cli/conversations/`, `<base>/antigravity/conversations/`
//! and `<base>/antigravity/`, where `<base>` is `~/.gemini` (or
//! `$GEMINI_CLI_HOME/.gemini`). Older IDE builds stored opaque `.pb` files
//! instead; those carry no readable usage and are ignored. The
//! `conversation_summaries.db` that 1.2.x drops beside them is an index, not a
//! conversation (it has no `gen_metadata`) and is skipped.
//!
//! ## What is in a conversation database
//!
//! Not documented by Google — the layout below was reverse-engineered by the
//! community (tokscale `antigravity_cli.rs`, CodexBar `docs/antigravity.md`,
//! tokscale issue #1184 / PR #1327) against real databases; field numbers are
//! protobuf tags.
//!
//! - `gen_metadata(idx, data, size)` — one row per model generation, `data` is
//!   a protobuf. `#1` = chat model message:
//!   - `#4` usage: `#1` fixed system-prompt tokens (~1132) and `#2` newly
//!     processed (non-cached) input tokens — both billed as input; `#5` cache
//!     read; `#9` text output; `#10` thinking output (`#9 + #10 == #3`);
//!     `#11` the response id.
//!   - `#19` machine model id (`gemini-3-flash-a`, `gemini-pro-default`, …;
//!     sometimes absent or the routing label `gemini-default`), `#21` display
//!     label (`Gemini 3.5 Flash (High)`).
//!   - `#9.#4` a `{seconds, nanos}` timestamp — agy ≤ 1.1.17 only.
//! - `steps(idx, step_type, metadata)` — for `step_type = 15` (model turns)
//!   `metadata.#1` is the wall-clock `{seconds, nanos}`; `#9.#11` repeats the
//!   response id and `#20.#3` is the `gen_metadata.idx`. agy ≥ 1.1.18 dates
//!   generations *only* here.
//! - `trajectory_metadata_blob(id, data)` — `#2` session created-at,
//!   `#1.#1` the workspace as a `file://` URI.
//!
//! ## Mapping
//!
//! `input = #1 + #2` (already excludes cache); `output = #9 + #10` because the
//! provider bills thinking as output and the price maths only multiplies
//! `output_tokens`; `reasoning = #10` is the informational subset (same
//! convention as Codex/Claude). Antigravity records no cache-write figure.
//!
//! Events are deduplicated on the response id alone, not per file: `/fork`
//! and importing an IDE conversation into the CLI copy the earlier
//! generations into a new database, and those tokens were spent once. The
//! conversation that reported a turn first keeps it (later copies are skipped
//! rather than re-written), so attribution stays put across rescans.
//!
//! ## Freshness
//!
//! Each database is re-read whenever its size/mtime (or its `-wal`'s)
//! changed; unchanged files cost two `stat`s. Reads are read-only; a WAL
//! database left without sidecars by a clean close is retried `immutable=1`.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension, types::ValueRef};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub struct Antigravity;

/// Index database next to the conversations — never holds usage.
const SUMMARIES_DB: &str = "conversation_summaries.db";
/// The router's placeholder for "some Gemini"; names no concrete model.
const ROUTING_LABEL: &str = "gemini-default";
/// Earliest plausible turn time (Antigravity did not exist before).
const MIN_TS_MS: i64 = 1_577_836_800_000; // 2020-01-01
/// Tolerated clock skew for a time read from disk.
const SKEW_MS: i64 = 60 * 60 * 1000;

impl SourceAdapter for Antigravity {
    fn id(&self) -> &'static str {
        apps::GEMINI_ANTIGRAVITY
    }
    fn display_name(&self) -> &'static str {
        "Antigravity"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        roots_under(&base_dir())
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(discover_in(&base_dir()))
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let fp = fingerprint(&item.path);
        let cur = store.load_cursor(&item.key)?;
        if cur.state.as_deref() == Some(fp.as_str()) {
            return Ok(ScanOutcome::default());
        }
        let conn = open_db(&item.path)?;
        let (mut out, rows) = read_conversation(&conn, item, store)?;
        // Fingerprint taken BEFORE the read: a write racing the read moves the
        // file past it, so the next pass simply reads again. The engine
        // persists the watermark only after the events are ingested.
        out.pending_cursor = Some(PendingCursor {
            key: item.key.clone(),
            path: item.path.clone(),
            offset: rows,
            mtime_ms: 0,
            state: Some(fp),
        });
        Ok(out)
    }
}

// ------------------------------------------------------------------ discovery

/// `~/.gemini`, or `$GEMINI_CLI_HOME/.gemini` when that override is set (the
/// Gemini CLI's own convention — Antigravity shares the directory).
fn base_dir() -> PathBuf {
    let env = std::env::var("GEMINI_CLI_HOME").ok();
    base_dir_from(env.as_deref(), &dirs::home_dir().unwrap_or_default())
}

fn base_dir_from(gemini_cli_home: Option<&str>, home: &Path) -> PathBuf {
    match gemini_cli_home.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => Path::new(h).join(".gemini"),
        None => home.join(".gemini"),
    }
}

fn roots_under(base: &Path) -> Vec<PathBuf> {
    vec![
        base.join("antigravity-cli").join("conversations"),
        base.join("antigravity").join("conversations"),
        base.join("antigravity"),
    ]
}

/// Immediate `*.db` entries of every root, deduplicated and sorted.
fn discover_in(base: &Path) -> Vec<SourceItem> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for root in roots_under(base) {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let is_db = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("db"));
            let is_index = p.file_name().is_some_and(|n| n == SUMMARIES_DB);
            if !is_db || is_index || !p.is_file() {
                continue;
            }
            let key = p.to_string_lossy().to_string();
            if seen.insert(key.clone()) {
                items.push(SourceItem {
                    key,
                    path: p,
                    kind: SourceKind::Sqlite,
                });
            }
        }
    }
    items.sort_by(|a, b| a.key.cmp(&b.key));
    items
}

/// `len:mtime` of the database and its `-wal` — a WAL database takes writes
/// there long before a checkpoint touches the main file.
fn fingerprint(db: &Path) -> String {
    let stamp = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                format!("{}:{}", m.len(), mtime)
            })
            .unwrap_or_default()
    };
    let mut wal = db.as_os_str().to_os_string();
    wal.push("-wal");
    format!("{}|{}", stamp(db), stamp(Path::new(&wal)))
}

// -------------------------------------------------------------------- opening

fn open_with(path: &Path, immutable: bool) -> rusqlite::Result<Connection> {
    let p = path.to_string_lossy().replace('\\', "/");
    let uri = if immutable {
        format!("file:{p}?immutable=1")
    } else {
        format!("file:{p}?mode=ro")
    };
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    conn.busy_timeout(std::time::Duration::from_millis(1500))?;
    // Force the header/WAL to be read now so an unusable open fails here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    Ok(conn)
}

/// A cleanly closed WAL database has no `-wal`/`-shm`; a read-only handle
/// cannot create them and SQLite then refuses to open it. With no live WAL
/// there is nothing to miss, so an immutable open is safe. A non-empty `-wal`
/// means a writer is active — the error is real and is reported.
fn open_db(path: &Path) -> Result<Connection> {
    match open_with(path, false) {
        Ok(c) => Ok(c),
        Err(first) => {
            let mut wal = path.as_os_str().to_os_string();
            wal.push("-wal");
            let live_wal = std::fs::metadata(Path::new(&wal)).is_ok_and(|m| m.len() > 0);
            if live_wal {
                return Err(first.into());
            }
            open_with(path, true).map_err(|_| first.into())
        }
    }
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

/// A BLOB (or a TEXT holding raw bytes) column as bytes.
fn bytes_of(v: ValueRef<'_>) -> Option<Vec<u8>> {
    match v {
        ValueRef::Blob(b) | ValueRef::Text(b) => Some(b.to_vec()),
        _ => None,
    }
}

// ------------------------------------------------------------------- protobuf

/// Minimal protobuf wire reader — enough for "first occurrence of field N".
enum Wire<'a> {
    Varint(u64),
    Len(&'a [u8]),
    Skip,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut out = 0u64;
        let mut shift = 0u32;
        loop {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            out |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(out);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    fn advance(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len())?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    /// Next `(field, value)`, or `None` at the end / on malformed input (never
    /// resyncs: a corrupt blob yields what precedes the damage).
    fn next_field(&mut self) -> Option<(u64, Wire<'a>)> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let tag = self.varint()?;
        let wire = match tag & 7 {
            0 => Wire::Varint(self.varint()?),
            1 => {
                self.advance(8)?;
                Wire::Skip
            }
            2 => {
                let n = usize::try_from(self.varint()?).ok()?;
                Wire::Len(self.advance(n)?)
            }
            5 => {
                self.advance(4)?;
                Wire::Skip
            }
            _ => return None,
        };
        Some((tag >> 3, wire))
    }
}

fn msg(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut r = Reader::new(buf);
    while let Some((f, w)) = r.next_field() {
        if let (true, Wire::Len(b)) = (f == field, w) {
            return Some(b);
        }
    }
    None
}

fn varint(buf: &[u8], field: u64) -> Option<u64> {
    let mut r = Reader::new(buf);
    while let Some((f, w)) = r.next_field() {
        if let (true, Wire::Varint(v)) = (f == field, w) {
            return Some(v);
        }
    }
    None
}

fn string(buf: &[u8], field: u64) -> Option<&str> {
    msg(buf, field).and_then(|b| std::str::from_utf8(b).ok())
}

/// A string field, blank counted as absent.
fn text(buf: &[u8], field: u64) -> Option<&str> {
    string(buf, field).map(str::trim).filter(|s| !s.is_empty())
}

/// `{#1 seconds, #2 nanos}` → epoch ms; out-of-range nanos mean "malformed".
fn timestamp_ms(buf: &[u8]) -> Option<i64> {
    let secs = i64::try_from(varint(buf, 1)?).ok()?;
    let nanos = i64::try_from(varint(buf, 2).unwrap_or(0)).ok()?;
    if !(0..1_000_000_000).contains(&nanos) {
        return None;
    }
    secs.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}

// --------------------------------------------------------------------- models

/// Antigravity machine id → the price-book key of the model it serves.
///
/// Ids are opaque and were mapped by the community from the app's own model
/// registry (Antigravity Context Window Monitor `models.ts`, via tokscale's
/// alias table). Display labels are deliberately not used as keys — they are
/// renamed and localized server-side. Tier suffixes (`-high`/`-low`/`-medium`,
/// `-thinking`) share one price, so they collapse into the base model.
fn canonical_model(raw: &str) -> Option<&'static str> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "gemini-pro-default"
        | "gemini-pro-agent"
        | "gemini-3.1-pro-high"
        | "gemini-3.1-pro-low"
        | "model_placeholder_m16"
        | "model_placeholder_m36"
        | "model_placeholder_m37" => "gemini-3.1-pro",
        "gemini-3-pro-high" | "gemini-3-pro-low" => "gemini-3-pro",
        // The 3.5 Flash family: the `-a`/`-b`/`-agent` ids are retired
        // predecessors of "Gemini 3.5 Flash (High)".
        "gemini-3-flash-a"
        | "gemini-3-flash-b"
        | "gemini-3-flash-agent"
        | "gemini-3.5-flash-high"
        | "gemini-3.5-flash-medium"
        | "gemini-3.5-flash-low"
        | "gemini-3.5-flash-extra-low"
        | "model_placeholder_m20"
        | "model_placeholder_m132"
        | "model_placeholder_m133"
        | "model_placeholder_m187" => "gemini-3.5-flash",
        "gemini-3-flash"
        | "gemini-3-flash-c"
        | "model_placeholder_m18"
        | "model_placeholder_m47"
        | "model_placeholder_m84" => "gemini-3-flash-preview",
        "claude-opus-4-6-thinking" | "claude-opus-4.6-thinking" | "model_placeholder_m26" => {
            "claude-opus-4-6"
        }
        "claude-sonnet-4-6-thinking" | "claude-sonnet-4.6-thinking" | "model_placeholder_m35" => {
            "claude-sonnet-4-6"
        }
        "model_openai_gpt_oss_120b_medium" => "gpt-oss-120b",
        _ => return None,
    })
}

/// Display label → model, for rows that carry neither a usable `#19` nor a
/// sibling to borrow one from. Only labels seen in real databases.
fn label_model(label: &str) -> Option<&'static str> {
    match label.trim() {
        "Gemini 3.5 Flash (Low)" | "Gemini 3.5 Flash (Medium)" | "Gemini 3.5 Flash (High)" => {
            Some("gemini-3.5-flash")
        }
        "Gemini 3.1 Pro (High)" | "Gemini 3.1 Pro (Low)" => Some("gemini-3.1-pro"),
        _ => None,
    }
}

fn is_routing(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case(ROUTING_LABEL)
}

fn provider_of(model: &str) -> Option<&'static str> {
    let m = model.to_ascii_lowercase();
    if m.contains("gemini") {
        Some("google")
    } else if m.contains("claude") {
        Some("anthropic")
    } else if m.contains("gpt-oss") {
        Some("openai")
    } else {
        None
    }
}

/// Model attribution recovered from the conversation as a whole, for rows whose
/// `#19` is missing (continuation/tool turns) or only the routing label.
#[derive(Default)]
struct SessionModels {
    /// `#21` label → the concrete id seen next to it. A label seen with two
    /// *different* models is dropped: ambiguous evidence is no evidence.
    by_label: HashMap<String, String>,
    /// The one concrete id of the whole conversation — only when every label
    /// in it was identified by some row (otherwise an unlabelled row could
    /// silently inherit the wrong model after a model switch).
    sole: Option<String>,
}

impl SessionModels {
    fn from_rows<'a>(blobs: impl Iterator<Item = &'a [u8]>) -> Self {
        let mut by_label: HashMap<&str, Option<&str>> = HashMap::new();
        let mut distinct: HashSet<&str> = HashSet::new();
        let mut unresolved: Vec<&str> = Vec::new();
        for blob in blobs {
            let Some(chat) = msg(blob, 1) else { continue };
            let label = text(chat, 21);
            let concrete = text(chat, 19).filter(|m| !is_routing(m));
            let Some(model) = concrete else {
                unresolved.extend(label);
                continue;
            };
            distinct.insert(model);
            if let Some(label) = label {
                by_label
                    .entry(label)
                    .and_modify(|seen| {
                        if let Some(prev) = *seen {
                            let same = prev == model
                                || canonical_model(prev).unwrap_or(prev)
                                    == canonical_model(model).unwrap_or(model);
                            if !same {
                                *seen = None;
                            }
                        }
                    })
                    .or_insert(Some(model));
            }
        }
        let by_label: HashMap<String, String> = by_label
            .into_iter()
            .filter_map(|(l, m)| Some((l.to_string(), m?.to_string())))
            .collect();
        let all_labels_known = unresolved.iter().all(|l| by_label.contains_key(*l));
        let sole = match (distinct.len(), all_labels_known) {
            (1, true) => distinct.iter().next().map(|m| (*m).to_string()),
            _ => None,
        };
        Self { by_label, sole }
    }

    fn recover(&self, chat: &[u8]) -> Option<&str> {
        match text(chat, 21) {
            // A label never seen beside a concrete id is positive evidence of
            // a model this file never names — do not fall through to a guess.
            Some(label) => self.by_label.get(label).map(String::as_str),
            None => self.sole.as_deref(),
        }
    }
}

// -------------------------------------------------------------------- reading

/// Session-level facts from `trajectory_metadata_blob`.
struct Trajectory {
    created_ms: Option<i64>,
    workspace: Option<String>,
}

fn read_trajectory(conn: &Connection) -> Trajectory {
    let blob = has_table(conn, "trajectory_metadata_blob")
        .then(|| {
            conn.query_row(
                "SELECT data FROM trajectory_metadata_blob LIMIT 1",
                [],
                |r| Ok(bytes_of(r.get_ref(0)?)),
            )
            .ok()
            .flatten()
        })
        .flatten();
    let Some(blob) = blob else {
        return Trajectory {
            created_ms: None,
            workspace: None,
        };
    };
    Trajectory {
        created_ms: msg(&blob, 2).and_then(timestamp_ms).filter(|&ms| ms > 0),
        workspace: msg(&blob, 1)
            .and_then(|folder| string(folder, 1))
            .and_then(file_uri_to_path),
    }
}

/// Wall-clock times of model turns, keyed by response id and by generation
/// index. agy ≥ 1.1.18 keeps them nowhere else.
#[derive(Default)]
struct StepTimes {
    by_response: HashMap<String, i64>,
    by_gen_idx: HashMap<i64, i64>,
}

fn read_step_times(conn: &Connection) -> StepTimes {
    let mut out = StepTimes::default();
    if !has_table(conn, "steps") {
        return out;
    }
    let Ok(mut st) =
        conn.prepare("SELECT metadata FROM steps WHERE step_type = 15 AND metadata IS NOT NULL")
    else {
        return out;
    };
    let Ok(rows) = st.query_map([], |r| Ok(bytes_of(r.get_ref(0)?))) else {
        return out;
    };
    for blob in rows.flatten().flatten() {
        let Some(ms) = msg(&blob, 1).and_then(timestamp_ms).filter(|&m| m > 0) else {
            continue;
        };
        if let Some(id) = msg(&blob, 9).and_then(|m| text(m, 11)) {
            out.by_response.entry(id.to_string()).or_insert(ms);
        }
        if let Some(idx) = msg(&blob, 20)
            .and_then(|m| varint(m, 3))
            .and_then(|i| i64::try_from(i).ok())
        {
            out.by_gen_idx.entry(idx).or_insert(ms);
        }
    }
    out
}

fn file_mtime_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

/// `file:///C:/x` → `C:\x`; `file:///home/x` → `/home/x`; `file://host/s/x`
/// → `\\host\s\x`. Percent escapes are decoded (CJK paths arrive encoded).
fn file_uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    let b = decoded.as_bytes();
    let path = if b.first() == Some(&b'/') {
        if b.len() >= 3 && b[2] == b':' {
            decoded[1..].replace('/', "\\") // /C:/x → C:\x
        } else {
            decoded
        }
    } else {
        format!("//{decoded}").replace('/', "\\") // UNC
    };
    (!path.is_empty()).then_some(path)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Read one conversation database into events. Returns the outcome and the
/// number of `gen_metadata` rows seen (kept as the cursor's informational
/// offset). A database without `gen_metadata` is not a conversation: empty.
fn read_conversation(
    conn: &Connection,
    item: &SourceItem,
    store: &Store,
) -> Result<(ScanOutcome, u64)> {
    let mut out = ScanOutcome::default();
    if !has_table(conn, "gen_metadata") {
        return Ok((out, 0));
    }
    let session_id = item.path.file_stem().map_or_else(
        || "unknown".to_string(),
        |s| s.to_string_lossy().to_string(),
    );

    let mut rows: Vec<(Option<i64>, Vec<u8>)> = Vec::new();
    {
        let mut st = conn.prepare("SELECT idx, data FROM gen_metadata ORDER BY idx")?;
        let it = st.query_map([], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, bytes_of(r.get_ref(1)?)))
        })?;
        for row in it {
            // A damaged row must not sink the rest of the conversation.
            if let Ok((idx, Some(data))) = row {
                rows.push((idx, data));
            }
        }
    }
    let n_rows = rows.len() as u64;

    let traj = read_trajectory(conn);
    let steps = read_step_times(conn);
    let models = SessionModels::from_rows(rows.iter().map(|(_, d)| d.as_slice()));
    let fallback_ts = traj.created_ms.unwrap_or_else(|| file_mtime_ms(&item.path));
    let now = crate::store::now_ms();
    let believable = |ms: i64| ms >= MIN_TS_MS && ms <= now + SKEW_MS;

    let mut seen: HashSet<String> = HashSet::new();
    for (idx, blob) in &rows {
        let Some(chat) = msg(blob, 1) else { continue };
        let Some(usage) = msg(chat, 4) else { continue };

        let input = varint(usage, 1)
            .unwrap_or(0)
            .saturating_add(varint(usage, 2).unwrap_or(0));
        let cache_read = varint(usage, 5).unwrap_or(0);
        let text_out = varint(usage, 9).unwrap_or(0);
        let thinking = varint(usage, 10).unwrap_or(0);
        if input == 0 && cache_read == 0 && text_out == 0 && thinking == 0 {
            continue;
        }
        let response_id = text(usage, 11).map(str::to_string);
        if let Some(id) = &response_id {
            if !seen.insert(id.clone()) {
                out.skipped += 1;
                continue;
            }
            // A fork or an imported conversation carries the earlier turns
            // along. The ledger row already belongs to the conversation that
            // first reported it; letting the copy re-write it would flip the
            // session/project back and forth on every scan of either file.
            let owner = store
                .conn()
                .query_row(
                    "SELECT session_id FROM usage_events WHERE dedup_key = ?1",
                    [format!("agy:{id}")],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten();
            if owner.is_some_and(|o| o != session_id) {
                out.skipped += 1;
                continue;
            }
        }

        // Model: own concrete id → sibling-derived → label → (routing label).
        let own = text(chat, 19);
        let raw = own
            .filter(|m| !is_routing(m))
            .or_else(|| models.recover(chat))
            .or_else(|| text(chat, 21).and_then(label_model))
            .or(own);
        let canonical = raw.and_then(canonical_model);
        let model = canonical.or(raw).map(str::to_string);
        let request_model = raw
            .filter(|r| canonical.is_some_and(|c| !c.eq_ignore_ascii_case(r)))
            .map(str::to_string);

        // Time: per-generation stamp (agy ≤ 1.1.17) → the matching model-turn
        // step (agy ≥ 1.1.18) → conversation start → file mtime.
        let ts = msg(chat, 9)
            .and_then(|g| msg(g, 4))
            .and_then(timestamp_ms)
            .filter(|&ms| believable(ms))
            .or_else(|| {
                response_id
                    .as_deref()
                    .and_then(|id| steps.by_response.get(id))
                    .copied()
                    .filter(|&ms| believable(ms))
            })
            .or_else(|| {
                idx.and_then(|i| steps.by_gen_idx.get(&i))
                    .copied()
                    .filter(|&ms| believable(ms))
            })
            .unwrap_or(fallback_ts);

        let dedup_key = match &response_id {
            Some(id) => format!("agy:{id}"),
            None => format!(
                "agy:{session_id}:{}",
                idx.unwrap_or(out.events.len() as i64)
            ),
        };
        out.events.push(UsageEvent {
            dedup_key,
            app: apps::GEMINI_ANTIGRAVITY.into(),
            session_id: Some(session_id.clone()),
            project: traj.workspace.clone(),
            provider_id: model.as_deref().and_then(provider_of).map(str::to_string),
            model,
            request_model,
            ts_start: Some(ts),
            input_tokens: input,
            // Thinking is billed as output; `reasoning_tokens` is the subset.
            output_tokens: text_out.saturating_add(thinking),
            reasoning_tokens: thinking,
            cache_read_tokens: cache_read,
            provenance: Provenance::LocalSqlite,
            raw_ref: Some(format!(
                "{}#gen:{}",
                item.path.display(),
                idx.map_or_else(|| "?".to_string(), |i| i.to_string())
            )),
            ..Default::default()
        });
    }
    Ok((out, n_rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::scan_and_commit;
    use crate::pricing::{PriceBook, Resolution};

    // ---- tiny protobuf encoder for fixtures
    fn varint_bytes(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                return out;
            }
        }
    }
    fn f_varint(field: u64, v: u64) -> Vec<u8> {
        let mut o = varint_bytes(field << 3);
        o.extend(varint_bytes(v));
        o
    }
    fn f_len(field: u64, payload: &[u8]) -> Vec<u8> {
        let mut o = varint_bytes(field << 3 | 2);
        o.extend(varint_bytes(payload.len() as u64));
        o.extend_from_slice(payload);
        o
    }
    fn f_ts(field: u64, secs: i64) -> Vec<u8> {
        let mut t = f_varint(1, secs as u64);
        t.extend(f_varint(2, 500_000_000));
        f_len(field, &t)
    }

    /// Recent enough to pass the plausibility window whenever the tests run.
    fn recent_secs(back: i64) -> i64 {
        crate::store::now_ms() / 1000 - back
    }

    struct Gen<'a> {
        rid: &'a str,
        input: u64,
        new_input: u64,
        cache: u64,
        out: u64,
        think: u64,
        model: Option<&'a str>,
        label: Option<&'a str>,
        /// agy ≤ 1.1.17 per-generation stamp (`#9.#4`), seconds.
        stamp: Option<i64>,
    }

    impl Gen<'_> {
        fn blob(&self) -> Vec<u8> {
            let mut usage = f_varint(1, self.input);
            usage.extend(f_varint(2, self.new_input));
            usage.extend(f_varint(5, self.cache));
            usage.extend(f_varint(9, self.out));
            usage.extend(f_varint(10, self.think));
            if !self.rid.is_empty() {
                usage.extend(f_len(11, self.rid.as_bytes()));
            }
            let mut chat = f_len(4, &usage);
            match self.stamp {
                Some(s) => chat.extend(f_len(9, &f_ts(4, s))),
                // modern agy: #9 carries prompt-cache metadata, no timestamp
                None => chat.extend(f_len(9, &f_len(10, b"cache metadata, not a time"))),
            }
            if let Some(m) = self.model {
                chat.extend(f_len(19, m.as_bytes()));
            }
            if let Some(l) = self.label {
                chat.extend(f_len(21, l.as_bytes()));
            }
            f_len(1, &chat)
        }
    }

    fn turn<'a>(rid: &'a str, model: Option<&'a str>) -> Gen<'a> {
        Gen {
            rid,
            input: 1132,
            new_input: 900,
            cache: 5000,
            out: 300,
            think: 200,
            model,
            label: None,
            stamp: None,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gtt_agy_{tag}_{}_{}",
            std::process::id(),
            crate::store::now_ms()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn new_db(path: &Path, with_steps: bool) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE gen_metadata (idx integer, data blob, size integer);
             CREATE TABLE trajectory_metadata_blob (id text, data blob);",
        )
        .unwrap();
        if with_steps {
            c.execute_batch("CREATE TABLE steps (idx integer, step_type integer, metadata blob);")
                .unwrap();
        }
        c
    }

    fn put_gen(c: &Connection, idx: i64, blob: &[u8]) {
        c.execute(
            "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, ?3)",
            rusqlite::params![idx, blob, blob.len() as i64],
        )
        .unwrap();
    }

    fn put_traj(c: &Connection, created_secs: i64, workspace_uri: &str) {
        let mut b = f_len(1, &f_len(1, workspace_uri.as_bytes()));
        b.extend(f_ts(2, created_secs));
        c.execute(
            "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
            [b],
        )
        .unwrap();
    }

    fn item(path: &Path) -> SourceItem {
        SourceItem {
            key: path.to_string_lossy().into(),
            path: path.to_path_buf(),
            kind: SourceKind::Sqlite,
        }
    }

    #[test]
    fn parses_generations_into_billable_events() {
        let dir = temp_dir("parse");
        let db = dir.join("11111111-2222-3333-4444-555555555555.db");
        let c = new_db(&db, false);
        put_traj(&c, recent_secs(3600), "file:///C:/Users/me/My%20Proj");
        let mut g1 = turn("resp-1", Some("gemini-pro-default"));
        g1.stamp = Some(recent_secs(600));
        put_gen(&c, 0, &g1.blob());
        // Same response id again (a retried write) — counted once.
        put_gen(&c, 1, &g1.blob());
        // Zero-usage row (no tokens at all) — dropped.
        let mut empty = turn("resp-empty", Some("gemini-pro-default"));
        (
            empty.input,
            empty.new_input,
            empty.cache,
            empty.out,
            empty.think,
        ) = (0, 0, 0, 0, 0);
        put_gen(&c, 2, &empty.blob());
        let mut g3 = turn("resp-2", Some("claude-opus-4-6-thinking"));
        (g3.think, g3.cache) = (0, 0);
        g3.stamp = Some(recent_secs(300));
        put_gen(&c, 3, &g3.blob());
        drop(c);

        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert_eq!(out.events.len(), 2, "{:?}", out.events);
        assert_eq!(out.skipped, 1);

        let e = &out.events[0];
        assert_eq!(e.dedup_key, "agy:resp-1");
        assert_eq!(e.app, "gemini_antigravity");
        assert_eq!(
            e.session_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(e.project.as_deref(), Some(r"C:\Users\me\My Proj"));
        // input = fixed prompt (#1) + new input (#2); cache excluded.
        assert_eq!(e.input_tokens, 1132 + 900);
        assert_eq!(e.cache_read_tokens, 5000);
        // thinking is billed as output, and stays visible as the subset.
        assert_eq!(e.output_tokens, 300 + 200);
        assert_eq!(e.reasoning_tokens, 200);
        // machine id → price-book model, raw id kept for audit.
        assert_eq!(e.model.as_deref(), Some("gemini-3.1-pro"));
        assert_eq!(e.request_model.as_deref(), Some("gemini-pro-default"));
        assert_eq!(e.provider_id.as_deref(), Some("google"));
        assert_eq!(e.ts_start, Some(g1.stamp.unwrap() * 1000 + 500));
        assert_eq!(e.provenance, Provenance::LocalSqlite);

        let e2 = &out.events[1];
        assert_eq!(e2.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(e2.provider_id.as_deref(), Some("anthropic"));
        assert_eq!(e2.reasoning_tokens, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modern_turns_are_dated_from_the_steps_table() {
        let dir = temp_dir("steps");
        let db = dir.join("modern.db");
        let created = recent_secs(3 * 86_400);
        let (t_a, t_b) = (recent_secs(7200), recent_secs(60));
        let c = new_db(&db, true);
        put_traj(&c, created, "file:///home/me/proj");
        // No `#9.#4` on either row (agy ≥ 1.1.18).
        put_gen(&c, 3, &turn("resp-a", Some("gemini-3-flash-a")).blob());
        put_gen(&c, 7, &turn("", Some("gemini-3-flash-a")).blob()); // no response id → idx join
        let step = |secs: i64, rid: Option<&str>, gen_idx: Option<u64>| {
            let mut m = f_ts(1, secs);
            if let Some(r) = rid {
                m.extend(f_len(9, &f_len(11, r.as_bytes())));
            }
            if let Some(i) = gen_idx {
                m.extend(f_len(20, &f_varint(3, i)));
            }
            m
        };
        for (i, ty, meta) in [
            (1, 15, step(t_a, Some("resp-a"), Some(3))),
            (2, 15, step(t_b, None, Some(7))),
            // A non-model step must never date a generation.
            (3, 9, step(recent_secs(10), Some("resp-a"), Some(3))),
        ] {
            c.execute(
                "INSERT INTO steps (idx, step_type, metadata) VALUES (?1, ?2, ?3)",
                rusqlite::params![i, ty, meta],
            )
            .unwrap();
        }
        drop(c);

        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert_eq!(out.events.len(), 2);
        assert_eq!(out.events[0].ts_start, Some(t_a * 1000 + 500)); // by response id
        assert_eq!(out.events[1].ts_start, Some(t_b * 1000 + 500)); // by gen idx
        assert_eq!(out.events[1].dedup_key, "agy:modern:7");
        assert_eq!(out.events[0].model.as_deref(), Some("gemini-3.5-flash"));
        assert_eq!(out.events[0].project.as_deref(), Some("/home/me/proj"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn falls_back_to_conversation_start_when_nothing_dates_a_turn() {
        let dir = temp_dir("fallback");
        let db = dir.join("plain.db");
        let created = recent_secs(86_400);
        let c = new_db(&db, false);
        put_traj(&c, created, "file:///x");
        put_gen(&c, 0, &turn("r", Some("gemini-3-flash-a")).blob());
        drop(c);
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert_eq!(out.events[0].ts_start, Some(created * 1000 + 500));
        // An absurd stamp (year 2000) is not believed either.
        let db2 = dir.join("skewed.db");
        let c = new_db(&db2, false);
        put_traj(&c, created, "file:///x");
        let mut g = turn("r2", Some("gemini-3-flash-a"));
        g.stamp = Some(946_684_800);
        put_gen(&c, 0, &g.blob());
        drop(c);
        let out = scan_and_commit(&Antigravity, &store, &item(&db2));
        assert_eq!(out.events[0].ts_start, Some(created * 1000 + 500));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_model_ids_are_recovered_from_siblings_only() {
        let dir = temp_dir("models");
        let db = dir.join("m.db");
        let c = new_db(&db, false);
        let mut labelled = turn("r1", Some("gemini-pro-agent"));
        labelled.label = Some("Gemini 3.1 Pro (High)");
        put_gen(&c, 0, &labelled.blob());
        // Continuation turn: label only, no `#19` → borrows the sibling's id.
        let mut cont = turn("r2", None);
        cont.label = Some("Gemini 3.1 Pro (High)");
        put_gen(&c, 1, &cont.blob());
        // Routing label with a label nobody identified → stays as the router
        // name (unpriced later), never a guess from another row.
        let mut routed = turn("r3", Some("gemini-default"));
        routed.label = Some("Some Brand New Model");
        put_gen(&c, 2, &routed.blob());
        // Routing label whose display label is a known tier → concrete model.
        let mut known = turn("r4", Some("gemini-default"));
        known.label = Some("Gemini 3.5 Flash (Medium)");
        put_gen(&c, 3, &known.blob());
        drop(c);
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        let m: Vec<_> = out.events.iter().map(|e| e.model.as_deref()).collect();
        assert_eq!(
            m,
            [
                Some("gemini-3.1-pro"),
                Some("gemini-3.1-pro"),
                Some("gemini-default"),
                Some("gemini-3.5-flash")
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_sole_model_serves_unlabelled_rows_but_a_model_switch_blocks_it() {
        // One concrete id, every label identified → the unlabelled row inherits it.
        let blobs_ok = [
            turn("a", Some("gemini-pro-default")).blob(),
            turn("b", None).blob(),
        ];
        let sm = SessionModels::from_rows(blobs_ok.iter().map(Vec::as_slice));
        assert_eq!(
            sm.recover(msg(&blobs_ok[1], 1).unwrap()),
            Some("gemini-pro-default")
        );
        // A label that no row ever identified proves the file ran a model it
        // never names → no fallback for anyone.
        let mut orphan = turn("c", None);
        orphan.label = Some("Mystery");
        let blobs_bad = [turn("a", Some("gemini-pro-default")).blob(), orphan.blob()];
        let sm = SessionModels::from_rows(blobs_bad.iter().map(Vec::as_slice));
        assert_eq!(sm.recover(msg(&blobs_bad[1], 1).unwrap()), None);
        assert_eq!(sm.sole, None);
    }

    #[test]
    fn unchanged_databases_are_skipped_and_growth_is_picked_up() {
        let dir = temp_dir("fp");
        let db = dir.join("g.db");
        let c = new_db(&db, false);
        put_traj(&c, recent_secs(100), "file:///x");
        put_gen(&c, 0, &turn("r1", Some("gemini-pro-default")).blob());
        drop(c);
        let store = Store::open_memory().unwrap();
        let it = item(&db);
        assert_eq!(
            scan_and_commit(&Antigravity, &store, &it).events.len(),
            1
        );
        // Nothing changed → not even opened.
        assert!(scan_and_commit(&Antigravity, &store, &it).events.is_empty());

        std::thread::sleep(std::time::Duration::from_millis(30));
        let c = Connection::open(&db).unwrap();
        put_gen(&c, 1, &turn("r2", Some("gemini-pro-default")).blob());
        drop(c);
        // Re-read: the old generation comes back too (the ledger UPSERT makes
        // that idempotent) alongside the new one.
        let out = scan_and_commit(&Antigravity, &store, &it);
        let ids: Vec<_> = out.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(ids, ["agy:r1", "agy:r2"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_forked_copy_never_steals_or_double_counts_a_turn() {
        let dir = temp_dir("fork");
        let (a, b) = (dir.join("orig.db"), dir.join("fork.db"));
        let c = new_db(&a, false);
        put_gen(
            &c,
            0,
            &turn("shared-resp", Some("gemini-pro-default")).blob(),
        );
        drop(c);
        // The fork carries the shared turn plus one of its own.
        let c = new_db(&b, false);
        put_gen(
            &c,
            0,
            &turn("shared-resp", Some("gemini-pro-default")).blob(),
        );
        put_gen(&c, 1, &turn("fork-only", Some("gemini-pro-default")).blob());
        drop(c);

        let store = Store::open_memory().unwrap();
        let ingest = |events: &[UsageEvent]| {
            for e in events {
                store.upsert_event(e).unwrap();
            }
        };
        let ea = scan_and_commit(&Antigravity, &store, &item(&a));
        ingest(&ea.events);
        let eb = scan_and_commit(&Antigravity, &store, &item(&b));
        ingest(&eb.events);
        // The copy contributes only what is new to the ledger.
        let keys: Vec<_> = eb.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(keys, ["agy:fork-only"]);
        assert_eq!(eb.skipped, 1);

        // The original grows: it re-reads its own turn, which stays its own.
        std::thread::sleep(std::time::Duration::from_millis(30));
        let c = Connection::open(&a).unwrap();
        put_gen(&c, 1, &turn("orig-2", Some("gemini-pro-default")).blob());
        drop(c);
        let ea = scan_and_commit(&Antigravity, &store, &item(&a));
        ingest(&ea.events);
        let owner: String = store
            .conn()
            .query_row(
                "SELECT session_id FROM usage_events WHERE dedup_key='agy:shared-resp'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, "orig");
        let n: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM usage_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3); // shared, fork-only, orig-2 — nothing counted twice
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn databases_without_gen_metadata_are_ignored() {
        let dir = temp_dir("idx");
        let db = dir.join("idx.db");
        let c = Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE summaries (id text, title text);")
            .unwrap();
        drop(c);
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert!(out.events.is_empty());
        // Remembered: the second pass does not reopen it.
        assert!(store.load_cursor(&item(&db).key).unwrap().state.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_wal_database_left_without_sidecars_still_reads() {
        let dir = temp_dir("wal");
        let db = dir.join("w.db");
        let c = new_db(&db, false);
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        put_gen(&c, 0, &turn("wal-1", Some("gemini-pro-default")).blob());
        drop(c); // clean close checkpoints and removes -wal/-shm
        let mut wal = db.as_os_str().to_os_string();
        wal.push("-wal");
        assert!(!Path::new(&wal).exists(), "fixture must have no sidecars");
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert_eq!(out.events.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_blobs_degrade_to_nothing() {
        let dir = temp_dir("bad");
        let db = dir.join("bad.db");
        let c = new_db(&db, false);
        put_gen(
            &c,
            0,
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ],
        );
        put_gen(&c, 1, &[0x0a, 0x7f, 0x01]); // length runs past the buffer
        put_gen(&c, 2, &[]);
        put_gen(&c, 3, &turn("ok", Some("gemini-pro-default")).blob());
        drop(c);
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Antigravity, &store, &item(&db));
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].dedup_key, "agy:ok");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovery_covers_the_three_roots_and_skips_the_index() {
        let base = temp_dir("disc");
        let mk = |rel: &str, name: &str| {
            let d = base.join(rel);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(name), b"x").unwrap();
        };
        mk("antigravity-cli/conversations", "a.db");
        mk("antigravity/conversations", "b.db");
        mk("antigravity", "c.db");
        mk("antigravity", SUMMARIES_DB);
        mk("antigravity", "notes.txt");
        mk("antigravity/conversations", "old.pb");
        mk("antigravity", "c.db-wal");
        let names: Vec<_> = discover_in(&base)
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        for n in ["a.db", "b.db", "c.db"] {
            assert!(names.contains(&n.to_string()), "{n} in {names:?}");
        }
        assert!(discover_in(&base.join("nothing-here")).is_empty());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn gemini_cli_home_relocates_the_base_directory() {
        let home = Path::new("/home/u");
        assert_eq!(base_dir_from(None, home), home.join(".gemini"));
        assert_eq!(base_dir_from(Some("  "), home), home.join(".gemini"));
        assert_eq!(
            base_dir_from(Some("/data/g"), home),
            Path::new("/data/g").join(".gemini")
        );
    }

    #[test]
    fn workspace_uris_become_native_paths() {
        assert_eq!(
            file_uri_to_path("file:///C:/Users/me/proj").as_deref(),
            Some(r"C:\Users\me\proj")
        );
        assert_eq!(
            file_uri_to_path("file:///d%3A/%E9%A1%B9%E7%9B%AE").as_deref(),
            Some(r"d:\项目")
        );
        assert_eq!(
            file_uri_to_path("file:///home/me/p").as_deref(),
            Some("/home/me/p")
        );
        assert_eq!(
            file_uri_to_path("file://srv/share/x").as_deref(),
            Some(r"\\srv\share\x")
        );
        assert_eq!(file_uri_to_path("https://example.com"), None);
    }

    #[test]
    fn every_alias_target_is_priced_by_the_seed_book() {
        let store = Store::open_memory().unwrap();
        let book = PriceBook::load(&store).unwrap();
        let ids = [
            "gemini-pro-default",
            "gemini-pro-agent",
            "gemini-3.1-pro-high",
            "gemini-3-pro-low",
            "gemini-3-flash-a",
            "gemini-3-flash-b",
            "gemini-3-flash-agent",
            "gemini-3.5-flash-low",
            "gemini-3.5-flash-extra-low",
            "gemini-3-flash",
            "MODEL_PLACEHOLDER_M26",
            "MODEL_PLACEHOLDER_M35",
            "claude-opus-4-6-thinking",
            "claude-sonnet-4-6-thinking",
            "MODEL_OPENAI_GPT_OSS_120B_MEDIUM",
        ];
        for id in ids {
            let target = canonical_model(id).unwrap_or_else(|| panic!("{id} has no alias"));
            match book.resolve(target, Some(id), None) {
                Resolution::Priced(..) => {}
                Resolution::Unpriced => panic!("{id} → {target} is not in the seed price book"),
            }
        }
        // The router's placeholder must stay unpriced rather than be guessed.
        assert!(canonical_model("gemini-default").is_none());
        assert!(matches!(
            book.resolve("gemini-default", None, None),
            Resolution::Unpriced
        ));
    }
}
