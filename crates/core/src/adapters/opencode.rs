//! OpenCode adapter (spec §6.3): `~/.local/share/opencode/opencode.db`.
//!
//! One event per ASSISTANT `message` row — the source-of-truth grain:
//! `message.data.tokens`/`cost`/`modelID`/`providerID`/`time.created/completed`
//! are per-API-call values. `session.tokens_*`/`cost` are running sums of the
//! same messages (verified 1:1), but a session row collapses every request to
//! the LAST model used and stamps all usage at `time_created` — a session
//! spanning days lands entirely on its creation day and mislabels any
//! mid-session model switch. Messages fix both, and give real per-call
//! durations instead of session-lifetime spans.
//!
//! `cost > 0` is OpenCode's own computed USD → `provider_reported`;
//! `cost == 0` falls back to the price book (free-tier models price to NULL).
//! High-water mark: `message.time_updated` (1 s overlap for races).
//! Migration: `adapter_state="msg-v2"` — on first encounter the legacy
//! session-grain events (`opencode:session:*`) are purged and the watermark
//! reset, otherwise old + new rows would double-count.

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

/// Adapter-state marker persisted in `sync_cursors.adapter_state` once the
/// session-grain → message-grain migration has run for this source.
const MSG_SCHEMA_STATE: &str = "msg-v2";

pub struct OpenCode;

impl SourceAdapter for OpenCode {
    fn id(&self) -> &'static str {
        apps::OPENCODE
    }
    fn display_name(&self) -> &'static str {
        "OpenCode"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".local/share/opencode")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let p = crate::sync::home(".local/share/opencode/opencode.db");
        Ok(p.exists()
            .then(|| SourceItem {
                key: p.to_string_lossy().to_string(),
                path: p,
                kind: SourceKind::Sqlite,
            })
            .into_iter()
            .collect())
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let conn = open_ro(&item.path)?;
        let mut out = ScanOutcome::default();

        // The message table is the source of truth; fail loudly on any other
        // schema era instead of silently reporting zero usage.
        let core_tables: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name IN ('session','message')",
            [],
            |r| r.get(0),
        )?;
        if core_tables != 2 {
            bail!(
                "{}: unsupported opencode schema (no session/message tables)",
                item.path.display()
            );
        }

        // One-time migration: drop legacy session-grain rows and rewind the
        // watermark so every message is ingested under `opencode:msg:*`.
        let mut since = cur.offset.saturating_sub(1000);
        if cur.state.as_deref() != Some(MSG_SCHEMA_STATE) {
            let n = store.delete_app_events(apps::OPENCODE)?;
            if n > 0 {
                out.notes
                    .push(format!("purged {n} legacy session-level opencode events"));
            }
            since = 0;
        }

        let mut st = conn.prepare(
            "SELECT m.id, m.session_id, s.directory,
                    json_extract(m.data,'$.cost'),
                    json_extract(m.data,'$.tokens.input'),
                    json_extract(m.data,'$.tokens.output'),
                    json_extract(m.data,'$.tokens.reasoning'),
                    json_extract(m.data,'$.tokens.cache.read'),
                    json_extract(m.data,'$.tokens.cache.write'),
                    json_extract(m.data,'$.tokens.total'),
                    json_extract(m.data,'$.modelID'),
                    json_extract(m.data,'$.providerID'),
                    json_extract(m.data,'$.agent'),
                    json_extract(m.data,'$.time.created'),
                    json_extract(m.data,'$.time.completed'),
                    json_extract(m.data,'$.error'),
                    m.time_created,
                    m.time_updated
             FROM message m JOIN session s ON s.id = m.session_id
             WHERE json_extract(m.data,'$.role') = 'assistant'
               AND m.time_updated > ?1",
        )?;
        let rows = st.query_map(rusqlite::params![since as i64], |r| {
            Ok(MsgRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                directory: r.get(2)?,
                cost: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                tin: r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                tout: r.get::<_, Option<i64>>(5)?.unwrap_or(0) as u64,
                treason: r.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
                tcr: r.get::<_, Option<i64>>(7)?.unwrap_or(0) as u64,
                tcw: r.get::<_, Option<i64>>(8)?.unwrap_or(0) as u64,
                ttotal: r.get::<_, Option<i64>>(9)?,
                model: r.get(10)?,
                provider: r.get(11)?,
                agent: r.get(12)?,
                created: r.get(13)?,
                completed: r.get(14)?,
                error: r.get::<_, Option<String>>(15)?,
                created_col: r.get::<_, Option<i64>>(16)?.unwrap_or(0),
                updated: r.get::<_, Option<i64>>(17)?.unwrap_or(0),
            })
        })?;

        let mut max_updated = cur.offset as i64;
        let mut total_mismatches = 0u64;
        for row in rows {
            let m = row?;
            max_updated = max_updated.max(m.updated);
            // Self-audit: OpenCode records tokens.total as the billed sum;
            // flag when it disagrees with the component fields we ingest.
            if let Some(t) = m.ttotal
                && t != (m.tin + m.tout + m.treason + m.tcr + m.tcw) as i64
            {
                total_mismatches += 1;
            }
            let (cost_usd, cost_source) = if m.cost > 0.0 {
                (Some(m.cost), Some(CostSource::ProviderReported))
            } else {
                (None, None)
            };
            out.events.push(UsageEvent {
                dedup_key: format!("opencode:msg:{}", m.id),
                app: apps::OPENCODE.into(),
                session_id: Some(m.session_id),
                project: m.directory,
                provider_id: m.provider,
                model: m.model.clone(),
                request_model: m.model,
                ts_start: m
                    .created
                    .or_else(|| (m.created_col > 0).then_some(m.created_col)),
                ts_end: m.completed.or_else(|| (m.updated > 0).then_some(m.updated)),
                input_tokens: m.tin,
                output_tokens: m.tout + m.treason,
                reasoning_tokens: m.treason,
                cache_read_tokens: m.tcr,
                cache_write_5m_tokens: m.tcw,
                cost_usd,
                cost_source,
                provenance: Provenance::LocalSqlite,
                duration_ms: match (m.created, m.completed) {
                    (Some(a), Some(b)) if b > a => Some(b - a),
                    _ => None,
                },
                status: m.agent,
                error: m.error.map(|e| e.chars().take(300).collect()),
                raw_ref: Some(format!("{}#message:{}", item.path.display(), m.id)),
                ..Default::default()
            });
        }
        if total_mismatches > 0 {
            out.notes.push(format!(
                "{total_mismatches} message(s) where tokens.total != component sum"
            ));
        }
        if (max_updated as u64) > cur.offset || cur.state.as_deref() != Some(MSG_SCHEMA_STATE) {
            out.pending_cursor = Some(PendingCursor {
                key: item.key.clone(),
                path: item.path.clone(),
                offset: max_updated.max(0) as u64,
                mtime_ms: 0,
                state: Some(MSG_SCHEMA_STATE.to_string()),
            });
        }
        out.consumed = max_updated.max(0) as u64;
        Ok(out)
    }
}

struct MsgRow {
    id: String,
    session_id: String,
    directory: Option<String>,
    cost: f64,
    tin: u64,
    tout: u64,
    treason: u64,
    tcr: u64,
    tcw: u64,
    ttotal: Option<i64>,
    model: Option<String>,
    provider: Option<String>,
    agent: Option<String>,
    created: Option<i64>,
    completed: Option<i64>,
    error: Option<String>,
    created_col: i64,
    updated: i64,
}

pub(crate) fn open_ro(path: &std::path::Path) -> Result<Connection> {
    let uri = format!("file:{}?mode=ro", path.to_string_lossy().replace('\\', "/"));
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    conn.busy_timeout(std::time::Duration::from_millis(1500))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::scan_and_commit;

    fn fixture(path: &std::path::Path) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE session(
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL DEFAULT 'p',
                directory TEXT NOT NULL, agent TEXT, model TEXT,
                time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL);
             CREATE TABLE message(
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL,
                data TEXT NOT NULL);",
        )
        .unwrap();
        c
    }

    /// `(in, out, cache_read)` + `(created, completed)` — mirrors the real
    /// `message.data` shape for one assistant row.
    fn msg_json(
        model: &str,
        provider: &str,
        cost: f64,
        tok: (i64, i64, i64),
        time: (i64, i64),
    ) -> String {
        let (tin, tout, cr) = tok;
        let (t0, t1) = time;
        format!(
            r#"{{"role":"assistant","agent":"build","modelID":"{model}","providerID":"{provider}",
            "cost":{cost},"tokens":{{"total":{},"input":{tin},"output":{tout},"reasoning":0,
            "cache":{{"write":0,"read":{cr}}}}},"time":{{"created":{t0},"completed":{t1}}}}}"#,
            tin + tout + cr
        )
    }

    fn item(path: &std::path::Path) -> SourceItem {
        SourceItem {
            key: path.to_string_lossy().into(),
            path: path.to_path_buf(),
            kind: SourceKind::Sqlite,
        }
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gtt_oc_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn message_level_events_per_model_and_day() {
        let dir = tmpdir("msglvl");
        let src = dir.join("opencode.db");
        let c = fixture(&src);
        // One session created day 1, but messages land on two later days
        // under two different models — the old session-grain adapter
        // misattributed both dimensions.
        c.execute(
            "INSERT INTO session VALUES('s1','p','D:\\proj','build',
             '{\"id\":\"glm-x\"}', 1_000_000, 1_000_000)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES('m1','s1',2_000_000,2_000_000,?1)",
            [msg_json(
                "glm-x",
                "opencode-go",
                0.5,
                (100, 10, 500),
                (2_000_000, 2_005_000),
            )],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES('m2','s1',3_000_000,3_000_000,?1)",
            [msg_json(
                "ox-free",
                "opencode",
                0.0,
                (200, 20, 0),
                (3_000_000, 3_002_000),
            )],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES('u1','s1',2_500_000,2_500_000,'{\"role\":\"user\"}')",
            [],
        )
        .unwrap();
        drop(c);

        let store = Store::open_memory().unwrap();
        let it = item(&src);
        let out = scan_and_commit(&OpenCode, &store, &it);
        assert_eq!(out.events.len(), 2); // user row skipped
        let e0 = &out.events[0];
        assert_eq!(e0.dedup_key, "opencode:msg:m1");
        assert_eq!(e0.model.as_deref(), Some("glm-x"));
        assert_eq!(e0.provider_id.as_deref(), Some("opencode-go"));
        assert_eq!(e0.ts_start, Some(2_000_000));
        assert_eq!(e0.ts_end, Some(2_005_000));
        assert_eq!(e0.duration_ms, Some(5_000));
        assert_eq!(e0.input_tokens, 100);
        assert_eq!(e0.cache_read_tokens, 500);
        assert_eq!(e0.cost_usd, Some(0.5));
        assert_eq!(e0.cost_source, Some(CostSource::ProviderReported));
        assert_eq!(e0.project.as_deref(), Some(r"D:\proj"));
        let e1 = &out.events[1];
        assert_eq!(e1.model.as_deref(), Some("ox-free")); // per-message model!
        assert_eq!(e1.cost_usd, None); // free → price book, not fabricated
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_session_events_purged_on_migration() {
        let dir = tmpdir("mig");
        let src = dir.join("opencode.db");
        let c = fixture(&src);
        c.execute(
            "INSERT INTO session VALUES('s1','p','/proj','build',NULL,1,1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES('m1','s1',100,100,?1)",
            [msg_json(
                "glm-x",
                "opencode-go",
                0.01,
                (10, 5, 0),
                (100, 200),
            )],
        )
        .unwrap();
        drop(c);

        let store = Store::open_memory().unwrap();
        // Simulate the old adapter's state: session-grain event + watermark.
        store
            .upsert_event(&UsageEvent {
                dedup_key: "opencode:session:s1".into(),
                app: apps::OPENCODE.into(),
                input_tokens: 999_999,
                provenance: Provenance::LocalSqlite,
                ..Default::default()
            })
            .unwrap();
        store
            .save_cursor(apps::OPENCODE, &src.to_string_lossy(), &src, 500, 0, None)
            .unwrap();

        let out = scan_and_commit(&OpenCode, &store, &item(&src));
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].dedup_key, "opencode:msg:m1");
        // Legacy row must be gone once the engine ingests the scan.
        let gone: i64 = store
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE dedup_key='opencode:session:s1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gone, 0);
        assert_eq!(
            store.load_cursor(&item(&src).key).unwrap().state.as_deref(),
            Some(MSG_SCHEMA_STATE)
        );
        // Incremental: the 1 s watermark overlap refetches the newest row,
        // but the same dedup_key merges in place — no double counting.
        let again = scan_and_commit(&OpenCode, &store, &item(&src));
        assert_eq!(again.events.len(), 1);
        assert_eq!(again.events[0].dedup_key, "opencode:msg:m1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_includes_reasoning_as_a_subset() {
        let dir = tmpdir("reason");
        let src = dir.join("opencode.db");
        let c = fixture(&src);
        c.execute(
            "INSERT INTO session VALUES('s1','p','/proj','build',NULL,1,1)",
            [],
        )
        .unwrap();
        let data = r#"{"role":"assistant","modelID":"glm-x","providerID":"opencode-go","cost":0,
            "tokens":{"total":47,"input":10,"output":5,"reasoning":30,
            "cache":{"write":2,"read":0}},"time":{"created":100,"completed":200}}"#;
        c.execute("INSERT INTO message VALUES('m1','s1',100,100,?1)", [data])
            .unwrap();
        drop(c);

        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&OpenCode, &store, &item(&src));
        let e = &out.events[0];
        assert_eq!(e.output_tokens, 35); // 5 output + 30 reasoning
        assert_eq!(e.reasoning_tokens, 30);
        assert_eq!(e.cache_write_5m_tokens, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn updated_message_rescans() {
        let dir = tmpdir("upd");
        let src = dir.join("opencode.db");
        let c = fixture(&src);
        c.execute(
            "INSERT INTO session VALUES('s1','p','/proj','build',NULL,1,1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES('m1','s1',100,100,?1)",
            [msg_json("glm-x", "opencode-go", 0.0, (10, 5, 0), (100, 0))],
        )
        .unwrap();
        drop(c);

        let store = Store::open_memory().unwrap();
        let it = item(&src);
        let out1 = scan_and_commit(&OpenCode, &store, &it);
        assert_eq!(out1.events[0].output_tokens, 5);

        // Streaming completes: same row updated in place.
        let c = Connection::open(&src).unwrap();
        c.execute(
            "UPDATE message SET time_updated=500, data=?1 WHERE id='m1'",
            [msg_json(
                "glm-x",
                "opencode-go",
                0.07,
                (10, 50, 900),
                (100, 500),
            )],
        )
        .unwrap();
        drop(c);

        let out2 = scan_and_commit(&OpenCode, &store, &it);
        assert_eq!(out2.events.len(), 1);
        assert_eq!(out2.events[0].dedup_key, "opencode:msg:m1");
        assert_eq!(out2.events[0].output_tokens, 50);
        assert_eq!(out2.events[0].cache_read_tokens, 900);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_message_table_is_error() {
        let dir = tmpdir("bad");
        let src = dir.join("opencode.db");
        let c = Connection::open(&src).unwrap();
        c.execute_batch("CREATE TABLE legacy(x TEXT)").unwrap();
        drop(c);
        let store = Store::open_memory().unwrap();
        let err = OpenCode.scan_sqlite(&item(&src), &store).unwrap_err();
        assert!(format!("{err:#}").contains("unsupported opencode schema"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
