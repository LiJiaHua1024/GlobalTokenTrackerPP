//! Freebuff desktop adapter: `%USERPROFILE%\.config\freebuff-desktop\`
//!   `projects/<name>-<projectId>/desktop-v2.db`   — SQLite (WAL) session store
//!   `state.json.session-refunds.json`             — settled freebucks spends
//!
//! ## Sessions (token-exact)
//!
//! `messages` rows carry a `metrics_json` blob `{"usage":{"inputTokens",
//! "cachedInputTokens","outputTokens","reasoningOutputTokens",
//! "totalTokens"},"costUsd":0}`. Measured invariants (three real samples):
//!
//! - `totalTokens = inputTokens + outputTokens` → `inputTokens` INCLUDES
//!   cache reads (input − cached = fresh input), exactly the OpenAI semantic.
//! - `reasoningOutputTokens ⊆ outputTokens` → subtracted back out so the
//!   headline `input+output+cache_read` convention stays vendor-consistent.
//!
//! Free-mode threads report an opaque model id (`m-…` / `fbm1.…`) and
//! `costUsd: 0` — tokens stay exact, cost stays honestly unpriced (these runs
//! bill daily freebucks, not dollars). BYOK threads carry regular model names
//! that flow into the normal price-book path.
//!
//! Watermark: `messages.seq` (AUTOINCREMENT monotonic). Assistant rows may
//! have their `metrics_json` rewritten in place while a turn streams, so the
//! `REWRITE_WINDOW` rows behind the watermark are re-read on every changed
//! fingerprint against the SAME dedup keys — the completeness-UPSERT makes
//! that idempotent instead of double-counting.
//!
//! ## freebucks quota
//!
//! `state.json.session-refunds.json` holds settled per-run spends
//! (`{receiptId, updatedAt, status:"settled", freebucks, sources:{daily,
//! wallet}}`). The adapter folds settled receipts into a UTC-day accumulator
//! kept in `adapter_state` and emits a `daily` QuotaSnapshot. The daily
//! allowance total lives only server-side, so `limit_value`/`used_percent`
//! stay unset — the UI shows the used amount, not a fabricated percentage.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, QuotaSnapshot, UsageEvent, apps};
use crate::normalize::{input_excludes_cache, num};
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const SESSION_DB: &str = "desktop-v2.db";
const REFUNDS_FILE: &str = "state.json.session-refunds.json";

/// Rows behind the seq watermark re-read on every changed fingerprint so
/// in-place `metrics_json` rewrites stream their final numbers.
const REWRITE_WINDOW: i64 = 50;

pub struct Freebuff {
    /// Adapter base dir — `~/.config/freebuff-desktop` in production,
    /// a fabricated temp dir in tests.
    root: PathBuf,
}

impl Freebuff {
    pub fn new() -> Self {
        Self {
            root: crate::sync::home(".config/freebuff-desktop"),
        }
    }

    /// Test-only constructor — production points at the real self.root.
    #[cfg(test)]
    fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Default for Freebuff {
    fn default() -> Self {
        Self::new()
    }
}

fn open_ro(path: &Path, immutable_fallback: bool) -> Result<Connection> {
    let attempt = |uri: String| -> rusqlite::Result<Connection> {
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(1500))?;
        // Force header/WAL read now so an unusable open fails here.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })?;
        Ok(conn)
    };
    let p = path.to_string_lossy().replace('\\', "/");
    let first = match attempt(format!("file:{p}?mode=ro")) {
        Ok(conn) => return Ok(conn),
        Err(e) => e,
    };
    // A cleanly closed WAL database has no `-wal`; a read-only handle cannot
    // create the `-shm` SQLite then demands. With no live WAL there is nothing
    // to miss, so an immutable open is safe.
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let live_wal = std::fs::metadata(Path::new(&wal)).is_ok_and(|m| m.len() > 0);
    if !immutable_fallback || live_wal {
        return Err(first.into());
    }
    attempt(format!("file:{p}?immutable=1")).map_err(|_| first.into())
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// `len:mtime` of the database and its `-wal` — a live WAL database takes
/// writes there long before a checkpoint touches the main file.
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

/// Free-mode model ids carry no price-book identity; route them to Unpriced
/// instead of letting characters like `fbm1.AAEA…` match some real model.
fn resolvable_model(v: &str) -> Option<String> {
    if v.starts_with("m-") || v.starts_with("fbm1.") || v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

impl SourceAdapter for Freebuff {
    fn id(&self) -> &'static str {
        apps::FREEBUFF
    }

    fn display_name(&self) -> &'static str {
        "Freebuff"
    }

    /// Vendor-exact per-turn token buckets straight out of `metrics_json`.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone(), self.root.join("projects")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut items = Vec::new();
        let projects = self.root.join("projects");
        let Ok(rd) = std::fs::read_dir(&projects) else {
            return Ok(items);
        };
        for e in rd.flatten() {
            let db = e.path().join(SESSION_DB);
            if !db.is_file() {
                continue;
            }
            items.push(SourceItem {
                key: db.to_string_lossy().to_string(),
                path: db,
                kind: SourceKind::Sqlite,
            });
        }
        items.sort_by(|a, b| a.key.cmp(&b.key));
        // The freebucks ledger is one file for the whole installation — one
        // synthetic cursor row, same scan pipeline as the databases.
        let refunds = self.root.join(REFUNDS_FILE);
        if refunds.is_file() {
            items.push(SourceItem {
                key: refunds.to_string_lossy().to_string(),
                path: refunds,
                kind: SourceKind::Sqlite,
            });
        }
        Ok(items)
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        if item.path.extension().is_some_and(|x| x == "json") {
            self.scan_freebucks(item, store)
        } else {
            self.scan_sessions(item, store)
        }
    }
}

impl Freebuff {
    fn scan_sessions(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let fp = fingerprint(&item.path);
        let cur = store.load_cursor(&item.key)?;
        if cur.state.as_deref() == Some(fp.as_str()) && cur.offset > 0 {
            return Ok(ScanOutcome::default());
        }

        let conn = open_ro(&item.path, true)?;
        if !has_table(&conn, "messages")? {
            return Ok(ScanOutcome::default());
        }
        let project = self.project_name(item);
        let from = (cur.offset as i64).saturating_sub(REWRITE_WINDOW);

        // Watermark advances past EVERY role (user rows can't gain usage), so
        // the window only ever re-covers the live tail of the newest turn.
        let mut st = conn.prepare(
            "SELECT m.seq, t.id, t.model, m.metrics_json, m.ts
             FROM messages m LEFT JOIN threads t ON t.id = m.thread_id
             WHERE m.seq > ?1 ORDER BY m.seq",
        )?;
        let rows = st.query_map([from], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;

        let mut out = ScanOutcome::default();
        let mut max_seq = cur.offset as i64;
        for row in rows {
            let (seq, thread, model, metrics_json, ts) = row?;
            max_seq = max_seq.max(seq);
            let Ok(v) = serde_json::from_str::<Value>(&metrics_json) else {
                continue;
            };
            let usage = &v["usage"];
            if num(&usage["totalTokens"]) == 0 {
                continue; // assistant row with no LLM call attached
            }
            let input = num(&usage["inputTokens"]);
            let cached = num(&usage["cachedInputTokens"]);
            let output = num(&usage["outputTokens"]);
            let reasoning = num(&usage["reasoningOutputTokens"]).min(output);
            out.events.push(UsageEvent {
                dedup_key: format!("freebuff:{thread}:{seq}"),
                app: apps::FREEBUFF.into(),
                session_id: Some(thread.clone()),
                project: project.clone(),
                model: model.as_deref().and_then(resolvable_model),
                request_model: model.as_deref().and_then(resolvable_model),
                ts_start: Some(ts),
                ts_end: Some(ts),
                input_tokens: input_excludes_cache(input, cached, 0),
                output_tokens: output - reasoning,
                reasoning_tokens: reasoning,
                cache_read_tokens: cached,
                provenance: Provenance::LocalSqlite,
                // Free mode bills daily freebucks server-side; BYOK spend is
                // computed from the price book — never adopt `costUsd: 0` as
                // an official figure.
                cost_source: None,
                raw_ref: Some(format!("{}#seq:{}", item.path.display(), seq)),
                ..Default::default()
            });
        }

        let changed = fp != cur.state.as_deref().unwrap_or_default();
        if changed || max_seq > cur.offset as i64 {
            out.pending_cursor = Some(PendingCursor {
                key: item.key.clone(),
                path: item.path.clone(),
                offset: max_seq.max(0) as u64,
                mtime_ms: 0,
                state: Some(fp),
            });
        } else {
            out.pending_cursor = None;
        }
        Ok(out)
    }

    fn scan_freebucks(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let fp = fingerprint(&item.path);
        let cur = store.load_cursor(&item.key)?;
        let mut state = cur
            .state
            .as_deref()
            .and_then(|s| serde_json::from_str::<FreebucksState>(s).ok())
            .unwrap_or_default();
        if state.fingerprint == fp && !state.fingerprint.is_empty() && !state.days.is_empty() {
            return Ok(ScanOutcome::default());
        }

        let Some(data) = std::fs::read_to_string(&item.path).ok() else {
            return Ok(ScanOutcome::default());
        };
        let Ok(list) = serde_json::from_str::<Vec<Refund>>(&data) else {
            return Ok(ScanOutcome::default());
        };
        let mut seen: HashSet<String> = state.seen.into_iter().collect();
        for r in list {
            if !seen.insert(r.receipt_id.clone()) {
                continue;
            }
            if r.status != "settled" || r.freebucks <= 0.0 {
                continue;
            }
            let day = jiff::Timestamp::from_millisecond(r.updated_at)
                .ok()
                .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).date().to_string());
            if let Some(day) = day {
                *state.days.entry(day).or_default() += r.freebucks;
            }
        }
        // Bounded state: a week of daily buckets, receipt ids capped.
        let now = jiff::Timestamp::now();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date().to_string();
        let cutoff = (now - std::time::Duration::from_secs(7 * 86_400))
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
            .to_string();
        state.days.retain(|d, _| d.as_str() >= cutoff.as_str());
        state.seen = seen.into_iter().collect();
        if state.seen.len() > 2048 {
            state.seen = state.seen.split_off(state.seen.len() - 2048);
        }
        state.fingerprint = fp;

        let today_used = *state.days.get(&today).unwrap_or(&0.0);
        let mut out = ScanOutcome::default();
        out.quotas.push(QuotaSnapshot {
            app: apps::FREEBUFF.into(),
            account: None,
            captured_at: crate::store::now_ms(),
            window_kind: "daily".into(),
            used: Some(today_used),
            // The daily allowance total only lives server-side — show what was
            // spent, never invent a percentage.
            limit_value: None,
            used_percent: None,
            resets_at: None,
            raw_json: serde_json::to_string(&state.days).ok(),
        });
        out.pending_cursor = Some(PendingCursor {
            key: item.key.clone(),
            path: item.path.clone(),
            offset: 0,
            mtime_ms: 0,
            state: serde_json::to_string(&state).ok(),
        });
        Ok(out)
    }

    /// Desktop project label from the sibling `project.json` (`projectPath`
    /// basename), falling back to the folder `<name>-<projectId>`.
    fn project_name(&self, item: &SourceItem) -> Option<String> {
        let dir = item.path.parent()?;
        let basename = |p: &Path| {
            p.file_name().map(|s| s.to_string_lossy().to_string())
        };
        let from_json = std::fs::read_to_string(dir.join("project.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| text(&v, "projectPath"))
            .and_then(|p| basename(Path::new(&p)));
        from_json.or_else(|| basename(dir)).filter(|s| !s.is_empty())
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    match &v[key] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FreebucksState {
    fingerprint: String,
    /// UTC day → settled freebucks total.
    days: BTreeMap<String, f64>,
    seen: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Refund {
    #[serde(rename = "receiptId")]
    receipt_id: String,
    #[serde(default, rename = "updatedAt")]
    updated_at: i64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    freebucks: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::scan_and_commit;

    fn make_db(path: &Path, model: Option<&str>) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY);
             CREATE TABLE threads (
               id TEXT PRIMARY KEY, model TEXT, agent_mode TEXT DEFAULT 'build');
             CREATE TABLE messages (
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
               role TEXT NOT NULL,
               parts_json TEXT NOT NULL DEFAULT '[]',
               metrics_json TEXT NOT NULL DEFAULT '{}',
               ts INTEGER NOT NULL);",
        )
        .unwrap();
        conn.execute("INSERT INTO threads (id, model) VALUES ('t1', ?1)", [model])
            .unwrap();
    }

    fn msgs(path: &Path, usage: &[(&str, u64, u64, u64, u64, i64)]) {
        let conn = rusqlite::Connection::open(path).unwrap();
        for (role, inp, cached, out, reasoning, ts) in usage {
            let m = if *inp == 0 && *out == 0 {
                "{}".to_string()
            } else {
                serde_json::json!({
                    "usage": {
                        "inputTokens": inp, "cachedInputTokens": cached,
                        "outputTokens": out, "reasoningOutputTokens": reasoning,
                        "totalTokens": inp + out,
                    },
                    "costUsd": 0
                })
                .to_string()
            };
            conn.execute(
                "INSERT INTO messages (thread_id, role, metrics_json, ts) VALUES ('t1', ?1, ?2, ?3)",
                rusqlite::params![role, m, ts],
            )
            .unwrap();
        }
    }

    #[test]
    fn token_buckets_and_model_resolution() {
        let dir = std::env::temp_dir().join(format!("gtt-fb-{}", std::process::id()));
        let proj = dir.join("MyProj-abc-123");
        std::fs::create_dir_all(&proj).unwrap();
        let db = proj.join(SESSION_DB);
        make_db(&db, Some("z-ai/glm-5.3-flash"));
        msgs(
            &db,
            &[
                // role, input, cached, output, reasoning, ts
                ("user", 0, 0, 0, 0, 1791884000000),
                ("assistant", 39427, 28288, 293, 60, 1791884002000),
                ("assistant", 20017, 19584, 478, 155, 1791884010000),
            ],
        );

        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let a = Freebuff::with_root(dir.clone());
        let out = scan_and_commit(&a, &store, &item);
        assert_eq!(out.events.len(), 2);
        let e1 = &out.events[0];
        assert_eq!(e1.dedup_key, "freebuff:t1:2");
        assert_eq!(e1.app, apps::FREEBUFF);
        assert_eq!(e1.session_id.as_deref(), Some("t1"));
        assert_eq!(e1.project.as_deref(), Some("MyProj-abc-123"));
        assert_eq!(e1.model.as_deref(), Some("z-ai/glm-5.3-flash"));
        // input excludes cache read: 39427 − 28288
        assert_eq!(e1.input_tokens, 39427 - 28288);
        assert_eq!(e1.cache_read_tokens, 28288);
        // output reported WITH reasoning tucked in → split per headline rules
        assert_eq!(e1.output_tokens, 293 - 60);
        assert_eq!(e1.reasoning_tokens, 60);
        assert_eq!(e1.ts_start, Some(1791884002000));
        assert!(e1.cost_usd.is_none());

        let e2 = &out.events[1];
        assert_eq!(e2.input_tokens, 20017 - 19584);
        assert_eq!(e2.output_tokens, 478 - 155);

        // Unchanged fingerprint → zero work, zero events.
        let out2 = scan_and_commit(&a, &store, &item);
        assert!(out2.events.is_empty() && out2.pending_cursor.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_mode_model_is_unpriced_and_watermark_skips_old_rows() {
        let dir = std::env::temp_dir().join(format!("gtt-fb2-{}", std::process::id()));
        let proj = dir.join("P-uuid");
        std::fs::create_dir_all(&proj).unwrap();
        let db = proj.join(SESSION_DB);
        make_db(&db, Some("m-7e20df6765"));
        msgs(
            &db,
            &[
                ("user", 0, 0, 0, 0, 1),
                ("assistant", 1000, 500, 50, 0, 2),
            ],
        );
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let a = Freebuff::with_root(dir.clone());
        let out = scan_and_commit(&a, &store, &item);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].model, None);
        assert_eq!(out.events[0].input_tokens, 500);

        // New turn after the watermark: user rows advance nothing usable and
        // the fingerprint change re-opens the rewrite window — the re-read
        // seq-2 row MUST re-emit onto the SAME dedup key (no double count).
        msgs(&db, &[("user", 0, 0, 0, 0, 3), ("assistant", 300, 100, 20, 5, 4)]);
        let out2 = scan_and_commit(&a, &store, &item);
        let mut keys: Vec<&str> =
            out2.events.iter().map(|e| e.dedup_key.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["freebuff:t1:2", "freebuff:t1:4"]);
        let fresh = &out2.events[1];
        assert_eq!(fresh.reasoning_tokens, 5);
        assert_eq!(fresh.input_tokens, 300 - 100);
        // Watermark persisted past every role, user rows included.
        assert_eq!(store.load_cursor(&item.key).unwrap().offset, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn freebucks_daily_quota_accumulates_settled_receipts() {
        let dir = std::env::temp_dir().join(format!("gtt-fb3-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        let refunds = dir.join(REFUNDS_FILE);
        std::fs::write(
            &refunds,
            serde_json::json!([
                {
                    "receiptId": "r1", "updatedAt": 1791884056386i64,
                    "status": "settled", "freebucks": 16.0,
                    "sources": {"daily": 16, "wallet": 0}
                },
                {
                    "receiptId": "r2", "updatedAt": 1791889000000i64,
                    "status": "settled", "freebucks": 4.5,
                    "sources": {"daily": 4, "wallet": 0.5}
                },
                {
                    "receiptId": "r3", "updatedAt": 1791889000001i64,
                    "status": "revoked", "freebucks": 99.0
                },
                {
                    "receiptId": "r1", "updatedAt": 1791884056386i64,
                    "status": "settled", "freebucks": 16.0
                }
            ])
            .to_string(),
        )
        .unwrap();

        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: refunds.to_string_lossy().to_string(),
            path: refunds.clone(),
            kind: SourceKind::Sqlite,
        };
        let a = Freebuff::with_root(dir.clone());
        let out = scan_and_commit(&a, &store, &item);
        assert_eq!(out.quotas.len(), 1);
        let q = &out.quotas[0];
        assert_eq!(q.app, apps::FREEBUFF);
        assert_eq!(q.window_kind, "daily");
        assert_eq!(q.used, Some(20.5)); // 16 + 4.5, duplicate skipped, revoked ignored
        assert!(q.limit_value.is_none() && q.used_percent.is_none());
        assert!(q.raw_json.is_some());

        // Unchanged file → quiet, and the day bucket survives in adapter state.
        let out2 = scan_and_commit(&a, &store, &item);
        assert!(out2.quotas.is_empty() || out2.quotas[0].used == Some(20.5));

        // A new settlement lands: total goes up, old receipts aren't re-counted.
        std::fs::write(
            &refunds,
            serde_json::json!([
                {"receiptId": "r4", "updatedAt": 1791892000000i64,
                 "status": "settled", "freebucks": 10.0}
            ])
            .to_string(),
        )
        .unwrap();
        let out3 = scan_and_commit(&a, &store, &item);
        assert_eq!(out3.quotas[0].used, Some(30.5));
        assert_eq!(out3.quotas[0].used_percent, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_and_foreign_databases_scan_clean() {
        let dir = std::env::temp_dir().join(format!("gtt-fb4-{}", std::process::id()));
        let proj = dir.join("E-uuid");
        std::fs::create_dir_all(&proj).unwrap();
        let db = proj.join(SESSION_DB);
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE other (x);")
            .unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&Freebuff::with_root(dir.clone()), &store, &item);
        assert!(out.events.is_empty());
        assert!(out.notes.is_empty());
        // No cursor row for a database that has nothing to scan.
        assert!(store.load_cursor(&item.key).unwrap().offset == 0);

        let empty = out.events.len();
        assert_eq!(empty, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolvable_model_gate() {
        assert_eq!(resolvable_model("z-ai/glm-5.3-flash").as_deref(), Some("z-ai/glm-5.3-flash"));
        assert_eq!(resolvable_model("openai/gpt-6-luna"), Some("openai/gpt-6-luna".into()));
        assert_eq!(resolvable_model("m-7e20df6765"), None);
        assert_eq!(resolvable_model("fbm1.AAEA…"), None);
        assert_eq!(resolvable_model(""), None);
    }

    #[test]
    fn rewrite_window_reaches_rows_behind_the_watermark() {
        let dir = std::env::temp_dir().join(format!("gtt-fb5-{}", std::process::id()));
        let proj = dir.join("R-uuid");
        std::fs::create_dir_all(&proj).unwrap();
        let db = proj.join(SESSION_DB);
        make_db(&db, None);
        msgs(&db, &[("user", 0, 0, 0, 0, 1)]);
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: db.to_string_lossy().to_string(),
            path: db.clone(),
            kind: SourceKind::Sqlite,
        };
        let a = Freebuff::with_root(dir.clone());
        assert!(scan_and_commit(&a, &store, &item).events.is_empty());
        // Simulate a streamed turn: a user row and a new assistant seq appear.
        msgs(&db, &[("user", 0, 0, 0, 0, 2), ("assistant", 10, 0, 5, 0, 3)]);
        let out = scan_and_commit(&a, &store, &item);
        assert_eq!(out.events.len(), 1); // the new assistant seq, not the user one
        let _ = std::fs::remove_dir_all(&dir);
    }
}
