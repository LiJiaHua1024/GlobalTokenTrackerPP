//! Cursor adapter (spec §6.x): `<data_dir>/Cursor/User/globalStorage/state.vscdb`.
//!
//! Cursor does server-side metering — every `bubbleId:*` record carries a
//! `tokenCount` struct whose values are all zero, and chat-request payloads
//! never land on disk. The honest local signal is the **user-turn bubble**
//! (`type == 1`): one per submitted prompt, with `requestId`, `createdAt`
//! and sometimes `modelInfo.modelName`. Type-2 bubbles (assistant/tool
//! blocks) fan out several per turn and carry no usage — deliberately
//! skipped to keep event granularity at "one composer turn = one event".
//!
//! `aiCodeTracking.dailyStats.*` rows in `ItemTable` are accepted-line
//! counters, not tokens — not ingested.
//!
//! Events carry zero tokens and price as `Unpriced`; they make Cursor
//! visible in tool filters/detail rows without fabricating usage.
//!
//! High-water mark: `cursorDiskKV.rowid`. Keys are `UNIQUE ON CONFLICT
//! REPLACE`, so in-place rewrites get a FRESH rowid — updated bubbles are
//! re-scanned naturally and collapse onto the same `cursor:{requestId}`
//! dedup key via the completeness-UPSERT.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, text};
use crate::store::Store;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

pub struct Cursor;

fn db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("Cursor/User/globalStorage/state.vscdb")
}

impl SourceAdapter for Cursor {
    fn id(&self) -> &'static str {
        apps::CURSOR
    }
    fn display_name(&self) -> &'static str {
        "Cursor"
    }
    fn capability(&self) -> Capability {
        Capability::Metadata
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![
            db_path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default(),
        ]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let db = db_path();
        Ok(if db.exists() {
            vec![SourceItem {
                key: db.to_string_lossy().to_string(),
                path: db,
                kind: SourceKind::Sqlite,
            }]
        } else {
            vec![]
        })
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let since = cur.offset as i64;
        let conn = super::opencode::open_ro(&item.path)?;
        // value is BLOB (utf-8 JSON) — fetch and parse in Rust; json_extract
        // on a blob column errors out ("malformed JSON").
        let mut st = conn.prepare(
            "SELECT rowid, key, CAST(value AS BLOB) FROM cursorDiskKV
             WHERE rowid > ?1 AND key LIKE 'bubbleId:%'
             ORDER BY rowid",
        )?;
        let rows = st.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut out = ScanOutcome::default();
        let mut max_row = cur.offset as i64;
        for row in rows {
            let (row_id, key, value) = row?;
            max_row = max_row.max(row_id);
            let Ok(v) = serde_json::from_slice::<Value>(&value) else {
                continue;
            };
            // User turns only — assistant bubbles have no usage and would
            // inflate the event count ~120×.
            if v.get("type").and_then(Value::as_i64) != Some(1) {
                continue;
            }
            // Key shape: `bubbleId:{composerId}:{bubbleId}`.
            let mut parts = key.splitn(3, ':');
            let _ = parts.next();
            let composer_id = parts.next().unwrap_or_default();
            let bubble_id = parts.next().unwrap_or_default();
            let rid = text(&v["requestId"]).unwrap_or_else(|| bubble_id.to_string());
            if rid.is_empty() {
                continue;
            }
            out.events.push(UsageEvent {
                dedup_key: format!("cursor:{rid}"),
                app: apps::CURSOR.into(),
                session_id: (!composer_id.is_empty()).then(|| composer_id.to_string()),
                model: text(&v["modelInfo"]["modelName"]),
                ts_start: epoch_ms(&v["createdAt"]),
                ts_end: epoch_ms(&v["createdAt"]),
                provenance: Provenance::LocalSqlite,
                status: Some("user_turn".into()),
                raw_ref: Some(format!("{}#key:{key}", item.path.display())),
                ..Default::default()
            });
        }
        if (max_row as u64) > cur.offset {
            store.save_cursor(self.id(), &item.key, &item.path, max_row as u64, 0, None)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &std::path::Path) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
            [],
        )
        .unwrap();
        conn
    }

    fn bubble(request_id: &str, model: Option<&str>) -> String {
        format!(
            r#"{{"_v":3,"type":1,"text":"hi","requestId":"{request_id}",{}
"createdAt":"2026-09-16T04:56:09.893Z","tokenCount":{{"inputTokens":0,"outputTokens":0}}}}"#,
            model
                .map(|m| format!(r#""modelInfo":{{"modelName":"{m}"}},"#))
                .unwrap_or_default()
        )
    }

    #[test]
    fn user_bubbles_only_and_dedup_stable() {
        let dir = std::env::temp_dir().join(format!("gtt-cur-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dbp = dir.join("state.vscdb");
        {
            let conn = make_db(&dbp);
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES ('bubbleId:comp1:b1', ?1)",
                [bubble("r-1", Some("grok-4.6"))],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES ('bubbleId:comp1:b2', ?1)",
                [r#"{"type":2,"requestId":"r-2","createdAt":"2026-09-16T04:56:30Z"}"#],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES ('otherKey', 'not json')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&dir.join("t.db")).unwrap();
        let item = SourceItem {
            key: "k".into(),
            path: dbp.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = Cursor.scan_sqlite(&item, &store).unwrap();
        assert_eq!(out.events.len(), 1);
        let ev = &out.events[0];
        assert_eq!(ev.dedup_key, "cursor:r-1");
        assert_eq!(ev.session_id.as_deref(), Some("comp1"));
        assert_eq!(ev.model.as_deref(), Some("grok-4.6"));
        assert_eq!(ev.input_tokens, 0);
        assert!(ev.ts_start.is_some());
        assert!(!ev.is_billable()); // metadata-only: engine gate admits via capability

        // REPLACE rewrite of the same key gets a new rowid → re-scanned,
        // same dedup key.
        {
            let conn = rusqlite::Connection::open(&dbp).unwrap();
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES ('bubbleId:comp1:b1', ?1)",
                [bubble("r-1", Some("grok-4.6"))],
            )
            .unwrap();
        }
        let out2 = Cursor.scan_sqlite(&item, &store).unwrap();
        assert_eq!(out2.events.len(), 1);
        assert_eq!(out2.events[0].dedup_key, "cursor:r-1");

        // Second scan with no new rows emits nothing.
        let out3 = Cursor.scan_sqlite(&item, &store).unwrap();
        assert!(out3.events.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
