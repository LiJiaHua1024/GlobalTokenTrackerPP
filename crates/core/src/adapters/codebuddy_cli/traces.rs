//! Aggregate fallback for finalized CodeBuddy traces; exact JSONL wins on overlap.
use crate::adapters::{PendingCursor, ScanOutcome, SourceItem};
use crate::{
    Provenance, UsageEvent,
    model::apps,
    normalize::{epoch_ms, num, text},
    store::Store,
};
use anyhow::Result;
use serde_json::Value;
pub(super) fn scan(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
    let bytes = crate::adapters::codebuddy::read_json(&item.path)?;
    let hash = crate::adapters::codebuddy::fingerprint(&bytes);
    // Partial JSON remains retryable; a file can be finalized in place.
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(ScanOutcome::default());
    };
    let trace = &v["trace"];
    let mut out = ScanOutcome::default();
    if let (Some(id), Some(start), Some(end)) = (
        text(&trace["traceId"]),
        epoch_ms(&trace["startedAt"]),
        epoch_ms(&trace["endedAt"]),
    ) {
        let model = &trace["modelInfo"];
        let session = text(&trace["sessionId"]).or_else(|| text(&v["sessionId"]));
        let single_model = model["models"]
            .as_array()
            .filter(|a| a.len() == 1)
            .and_then(|a| text(&a[0]));
        // Exact transcripts take precedence. Without a shared session ID,
        // overlapping aggregates are ambiguous and must not be added twice.
        let (exact_count, exact_tokens): (u64, u64) = store.conn().query_row(
                "SELECT COUNT(*), COALESCE(SUM(input_tokens+output_tokens+cache_read_tokens+cache_write_5m_tokens+cache_write_1h_tokens+unclassified_tokens),0)
                 FROM usage_events WHERE app=?1 AND dedup_key NOT LIKE 'codebuddy_cli:trace:%'
                 AND ((?2 IS NOT NULL AND session_id=?2 AND (ts_start IS NULL OR (ts_start>=?3 AND ts_start<=?4)))
                      OR (?2 IS NULL AND ts_start>=?3 AND ts_start<=?4 AND (?5 IS NULL OR model=?5)))",
                rusqlite::params![apps::CODEBUDDY_CLI, session, start, end, single_model], |r| Ok((r.get::<_, i64>(0)? as u64,r.get::<_, i64>(1)? as u64)))?;
        let total = num(&trace["totalTokens"]);
        // Shared session identity permits subtracting a partial exact coverage.
        // Without that identity, temporal overlap remains ambiguous: retain
        // exact records rather than invent a relationship between concurrent runs.
        if exact_count > 0 && (session.is_none() || exact_tokens >= total) {
            let deleted = store.conn().execute(
                "DELETE FROM usage_events WHERE app=?1 AND dedup_key=?2",
                rusqlite::params![apps::CODEBUDDY_CLI, format!("codebuddy_cli:trace:{id}")],
            )?;
            if deleted > 0 {
                store.set_state("codebuddy_cli_rollups_pending", "1")?;
            }
            return Ok(out);
        }
        let revision = format!("{hash}:{exact_tokens}");
        if store.load_cursor(&item.key)?.state.as_deref() == Some(revision.as_str()) {
            return Ok(out);
        }
        let input = model["totalInputTokens"].as_u64();
        let output = model["totalOutputTokens"].as_u64();
        let cached = num(&model["totalCachedTokens"]);
        let classified = (exact_tokens == 0)
            .then_some(input.zip(output))
            .flatten()
            .filter(|(i, o)| i.checked_add(*o) == Some(total) && cached <= *i);
        let (input_tokens, output_tokens, unclassified_tokens) = classified
            .map_or((0, 0, total.saturating_sub(exact_tokens)), |(i, o)| {
                (i - cached, o, 0)
            });
        if total > 0 {
            out.events.push(UsageEvent {
                dedup_key: format!("codebuddy_cli:trace:{id}"),
                app: apps::CODEBUDDY_CLI.into(),
                session_id: Some(id),
                model: model["models"]
                    .as_array()
                    .filter(|a| a.len() == 1)
                    .and_then(|a| text(&a[0])),
                ts_start: Some(start),
                ts_end: (end >= start).then_some(end),
                duration_ms: trace["duration"].as_i64().filter(|d| *d > 0),
                input_tokens,
                output_tokens,
                cache_read_tokens: if classified.is_some() { cached } else { 0 },
                unclassified_tokens,
                status: text(&trace["status"]),
                provenance: Provenance::LocalJsonl,
                raw_ref: Some(item.path.to_string_lossy().into_owned()),
                ..Default::default()
            });
        }
        out.pending_cursor = Some(PendingCursor {
            key: item.key.clone(),
            path: item.path.clone(),
            offset: bytes.len() as u64,
            mtime_ms: 0,
            state: Some(revision),
        });
        return Ok(out);
    }
    out.pending_cursor = Some(PendingCursor {
        key: item.key.clone(),
        path: item.path.clone(),
        offset: bytes.len() as u64,
        mtime_ms: 0,
        state: Some(hash),
    });
    Ok(out)
}
#[cfg(test)]
mod tests;
