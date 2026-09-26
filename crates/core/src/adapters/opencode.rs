//! OpenCode adapter (spec §6.3): `~/.local/share/opencode/opencode.db`
//! (note: NOT `~/.opencode`). Session rows carry native USD cost →
//! `provider_reported`; cost==0 falls back to the price book.
//! Incremental via `time_updated` high-water mark (1 s overlap for races).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};

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
        let since = cur.offset.saturating_sub(1000); // watermark stored as ms
        let conn = open_ro(&item.path)?;
        let mut st = conn.prepare(
            "SELECT id, directory, cost, tokens_input, tokens_output, tokens_reasoning,
                    tokens_cache_read, tokens_cache_write, model, agent,
                    time_created, time_updated
             FROM session WHERE time_updated > ?1",
        )?;
        let rows = st.query_map(rusqlite::params![since as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(7)?.unwrap_or(0) as u64,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<i64>>(10)?,
                r.get::<_, Option<i64>>(11)?.unwrap_or(0),
            ))
        })?;

        let mut out = ScanOutcome::default();
        let mut max_updated = cur.offset as i64;
        for row in rows {
            let (id, dir, cost, tin, tout, treason, tcr, tcw, model_json, agent, t0, t1) = row?;
            max_updated = max_updated.max(t1);
            let (model, provider) = parse_model(model_json.as_deref());
            // cost>0 is provider-reported USD; 0 → let the price book try.
            let (cost_usd, cost_source) = if cost > 0.0 {
                (Some(cost), Some(CostSource::ProviderReported))
            } else {
                (None, None)
            };
            out.events.push(UsageEvent {
                dedup_key: format!("opencode:session:{id}"),
                app: apps::OPENCODE.into(),
                session_id: Some(id.clone()),
                project: dir,
                provider_id: provider,
                model: model.clone(),
                request_model: model,
                ts_start: t0,
                ts_end: Some(t1),
                input_tokens: tin,
                output_tokens: tout,
                reasoning_tokens: treason,
                cache_read_tokens: tcr,
                cache_write_5m_tokens: tcw,
                cost_usd,
                cost_source,
                provenance: Provenance::LocalSqlite,
                duration_ms: match t0 {
                    Some(a) if t1 > a => Some(t1 - a),
                    _ => None,
                },
                status: agent,
                raw_ref: Some(format!("{}#session:{id}", item.path.display())),
                ..Default::default()
            });
        }
        if (max_updated as u64) > cur.offset {
            store.save_cursor(
                self.id(),
                &item.key,
                &item.path,
                max_updated as u64,
                0,
                None,
            )?;
        }
        out.consumed = max_updated.max(0) as u64;
        Ok(out)
    }
}

/// `model` column is JSON: `{"id":"glm-5.3-flash","providerID":"opencode-go","variant":"max"}`.
fn parse_model(json: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(s) = json else { return (None, None) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
        return (Some(s.to_string()), None);
    };
    (
        v.get("id").and_then(|x| x.as_str()).map(String::from),
        v.get("providerID")
            .and_then(|x| x.as_str())
            .map(String::from),
    )
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
