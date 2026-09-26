//! ZCode adapter (spec §6.4): `~/.zcode/cli/db/db.sqlite` → `model_usage`
//! (cleanest five-dimension source + duration/TTFT/status).
//! Dedup key: `logical_request_id + attempt_index` (ids carry retry suffixes).
//! Cross-source rule: rows whose provider is anthropic/openai/google are also
//! logged by those tools' own files — skip & count to prevent double counting.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::store::Store;
use anyhow::Result;

/// Providers whose usage is already ingested by their own adapters (§6.4).
const CROSS_PROVIDERS: &[&str] = &["anthropic", "openai", "google"];

pub struct ZCode;

impl SourceAdapter for ZCode {
    fn id(&self) -> &'static str {
        apps::ZCODE
    }
    fn display_name(&self) -> &'static str {
        "ZCode"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let p = crate::sync::home(".zcode/cli/db/db.sqlite");
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
        let since = (cur.offset as i64).saturating_sub(1000);
        let conn = super::opencode::open_ro(&item.path)?;
        let mut st = conn.prepare(
            "SELECT logical_request_id, attempt_index, session_id, turn_id,
                    provider_id, model_id, status, started_at, completed_at,
                    duration_ms, time_to_first_token_ms,
                    input_tokens, output_tokens, reasoning_tokens,
                    cache_creation_input_tokens, cache_read_input_tokens,
                    error_type
             FROM model_usage WHERE started_at > ?1",
        )?;
        let rows = st.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<i64>>(9)?,
                r.get::<_, Option<i64>>(10)?,
                r.get::<_, Option<i64>>(11)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(12)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(13)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(14)?.unwrap_or(0) as u64,
                r.get::<_, Option<i64>>(15)?.unwrap_or(0) as u64,
                r.get::<_, Option<String>>(16)?,
            ))
        })?;

        let mut out = ScanOutcome::default();
        let mut max_started = cur.offset as i64;
        for row in rows {
            let (
                lrid,
                attempt,
                session,
                turn,
                provider,
                model,
                status,
                t0,
                t1,
                dur,
                ttft,
                tin,
                tout,
                treason,
                tcw,
                tcr,
                err_ty,
            ) = row?;
            max_started = max_started.max(t0.unwrap_or(0));

            // Cross-provider rows are counted by the owning tool's adapter.
            let base = provider
                .as_deref()
                .unwrap_or("")
                .trim_start_matches("builtin:")
                .split(':')
                .next()
                .unwrap_or("");
            if CROSS_PROVIDERS.contains(&base) {
                out.skipped += 1;
                continue;
            }

            out.events.push(UsageEvent {
                dedup_key: format!("zcode:{lrid}:{attempt}"),
                app: apps::ZCODE.into(),
                session_id: session,
                provider_id: provider,
                model: model.clone(),
                request_model: model,
                ts_start: t0,
                ts_end: t1,
                input_tokens: tin,
                output_tokens: tout,
                reasoning_tokens: treason,
                cache_read_tokens: tcr,
                cache_write_5m_tokens: tcw,
                provenance: Provenance::LocalSqlite,
                duration_ms: dur,
                ttft_ms: ttft,
                status: status.or(turn).map(|s| s.to_string()),
                error: err_ty,
                raw_ref: Some(format!(
                    "{}#model_usage:{lrid}:{attempt}",
                    item.path.display()
                )),
                ..Default::default()
            });
        }
        if (max_started as u64) > cur.offset {
            store.save_cursor(
                self.id(),
                &item.key,
                &item.path,
                max_started as u64,
                0,
                None,
            )?;
        }
        Ok(out)
    }
}
