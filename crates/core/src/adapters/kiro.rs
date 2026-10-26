//! Kiro adapter — Kiro CLI + Kiro IDE (both generations).
//!
//! Kiro bills in **credits**. Credits stay in `UsageEvent::credits`, and —
//! because Kiro publishes a fixed list price per credit — each event also
//! carries `cost_usd = credits × USD_PER_CREDIT` as a provider-reported cost
//! (the book never re-prices it from tokens).
//!
//! Kiro writes **no token counters** locally (its `*_token_count` fields are
//! always 0). The CLI does record how full the context window was after each
//! turn, so a turn's billed input volume is reconstructed as
//! `requests × context fill × window` and booked as `unclassified_tokens`
//! (a vendor total with no trustworthy input/output split — never priced, and
//! kept out of speed figures). Real counters win whenever Kiro writes them.
//!
//! Three stores, all read-only:
//!
//! 1. **Kiro CLI** — `$KIRO_HOME/sessions/cli/<uuid>.json` (KIRO_HOME defaults
//!    to `~/.kiro`). The companion `.jsonl` is the transcript and carries no
//!    usage; the `.json` is *rewritten* after every turn and holds
//!    `session_state.conversation_metadata.user_turn_metadatas[]`, each with
//!    `metering_usage[]` (credits per request), token counters, model, turn
//!    duration and end timestamp. One event per user turn. Scanned through
//!    the Sqlite path with a `(len, mtime)` memo, like Cline's JSON files.
//!
//! 2. **Kiro IDE v2** — `$KIRO_HOME/sessions/<hash>/sess_<id>/messages.jsonl`
//!    (append-only `{id, timestamp, payload:{type,…}}` events) + `session.json`
//!    (`modelId`, `workspacePaths`). Credits ride on `usage_summary` events in
//!    `promptTurnSummaries[].usage`; `turn_start` carries the `executionId`.
//!    Incremental JSONL; the open turn survives segments via adapter state.
//!
//! 3. **Kiro IDE v1** — `<config>/Kiro/User/globalStorage/kiro.kiroagent/
//!    <32-hex workspace>/[<dir>/]<execution>` extensionless JSON execution
//!    files with `usageSummary[].usage` credits (plus
//!    `~/.kiro-server/data/...` on remote Linux boxes). Legacy `.chat` files and
//!    `workspace-sessions/` records carry no usage and are not ingested.

use super::{
    Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines,
};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, num, text};
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Kiro;

const CLI_PREFIX: &str = "kiro-cli:";
const IDE_V1_PREFIX: &str = "kiro-ide:";
/// v1 execution files can be large; anything beyond this is a transcript dump,
/// not a usage record worth holding in memory.
const MAX_V1_FILE: u64 = 64 * 1024 * 1024;
/// Kiro's published price per credit (add-on / overage rate, kiro.dev/pricing).
pub(crate) const USD_PER_CREDIT: f64 = 0.04;
/// Context window assumed when the session does not record one.
const DEFAULT_CONTEXT_WINDOW: f64 = 200_000.0;

/// `(cost_usd, cost_source)` for a credit amount.
fn credit_cost(credits: Option<f64>) -> (Option<f64>, Option<CostSource>) {
    match credits {
        Some(c) => (Some(c * USD_PER_CREDIT), Some(CostSource::ProviderReported)),
        None => (None, None),
    }
}

/// `$KIRO_HOME` or `~/.kiro`.
fn kiro_home() -> PathBuf {
    std::env::var_os("KIRO_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::sync::home(".kiro"))
}

fn sessions_root() -> PathBuf {
    kiro_home().join("sessions")
}

fn cli_root() -> PathBuf {
    sessions_root().join("cli")
}

/// `kiro.kiroagent` globalStorage dirs (local install + remote server).
fn agent_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("Kiro/User/globalStorage/kiro.kiroagent"));
    }
    v.push(crate::sync::home(
        ".kiro-server/data/User/globalStorage/kiro.kiroagent",
    ));
    v
}

fn workspace_storage(agent_dir: &Path) -> Option<PathBuf> {
    // kiro.kiroagent → globalStorage → User; workspaceStorage is User's child.
    Some(agent_dir.parent()?.parent()?.join("workspaceStorage"))
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// "Seen it, unchanged" memo for whole-file JSON sources.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct FileMemo {
    len: u64,
    mtime_ms: i64,
}

/// Returns `(meta, memo)` or `None` when the file is unchanged since last scan.
fn changed(item: &SourceItem, store: &Store) -> Result<Option<(std::fs::Metadata, FileMemo)>> {
    let meta = std::fs::metadata(&item.path)?;
    let memo = FileMemo {
        len: meta.len(),
        mtime_ms: mtime_ms(&meta),
    };
    let prior: Option<FileMemo> = store
        .load_cursor(&item.key)?
        .state
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());
    if prior.as_ref() == Some(&memo) {
        return Ok(None);
    }
    Ok(Some((meta, memo)))
}

fn memo_cursor(item: &SourceItem, memo: &FileMemo) -> PendingCursor {
    PendingCursor {
        key: item.key.clone(),
        path: item.path.clone(),
        offset: memo.len,
        mtime_ms: memo.mtime_ms,
        state: serde_json::to_string(memo).ok(),
    }
}

/// Sum of credit-unit entries in a `metering_usage`-style array.
fn sum_credits(arr: &Value) -> Option<f64> {
    let items = arr.as_array()?;
    let mut total = 0.0;
    let mut any = false;
    for m in items {
        let unit = m.get("unit").and_then(Value::as_str).unwrap_or("credit");
        if !unit.eq_ignore_ascii_case("credit") {
            continue;
        }
        if let Some(v) = fnum(&m["value"]).filter(|v| v.is_finite() && *v > 0.0) {
            total += v;
            any = true;
        }
    }
    any.then_some(total)
}

/// `"auto"` is Kiro's router, not a model — keep it visible but unpriceable.
fn model_or_auto(m: Option<String>) -> Option<String> {
    match m.as_deref() {
        None | Some("") => None,
        Some(s) if s.eq_ignore_ascii_case("auto") => Some("kiro-auto".into()),
        _ => m,
    }
}

// ───────────────────────────── Kiro CLI ─────────────────────────────

/// Parse one CLI session metadata document into per-turn events.
pub(crate) fn parse_cli_session(doc: &Value, path: &Path) -> Vec<UsageEvent> {
    let session_id = text(&doc["session_id"]).unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let project = text(&doc["cwd"]);
    let state = &doc["session_state"];
    let fallback_model = text(&state["rts_model_state"]["model_info"]["model_id"])
        .or_else(|| text(&state["conversation_metadata"]["last_context_usage"]["model_id"]));
    let Some(turns) = state["conversation_metadata"]["user_turn_metadatas"].as_array() else {
        return Vec::new();
    };
    let window = fnum(&state["rts_model_state"]["model_info"]["context_window_tokens"])
        .filter(|w| *w > 0.0)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let mut prev_fill: Option<f64> = None;
    let mut out = Vec::new();
    for (idx, t) in turns.iter().enumerate() {
        let credits = sum_credits(&t["metering_usage"]);
        let input = num(&t["input_token_count"]);
        let output = num(&t["output_token_count"]);
        let cache_read = num(&t["cache_read_input_token_count"]);
        let cache_write = num(&t["cache_write_input_token_count"]);
        // Every request of a turn re-sends the whole context; the context
        // grows from where the previous turn left it to this turn's final fill.
        let pct = |k: &str| fnum(&t[k]).filter(|p| p.is_finite() && *p > 0.0);
        let end_fill =
            pct("final_context_usage_percentage").or_else(|| pct("context_usage_percentage"));
        let start_fill = prev_fill
            .or_else(|| pct("context_usage_percentage"))
            .or(end_fill);
        if end_fill.is_some() {
            prev_fill = end_fill;
        }
        let requests = num(&t["total_request_count"])
            .max(t["metering_usage"].as_array().map_or(0, |a| a.len() as u64))
            .max(1);
        let recorded = input + output + cache_read + cache_write;
        let estimated = match (recorded, start_fill, end_fill) {
            (0, Some(s), Some(e)) => {
                ((s + e) / 2.0 / 100.0 * window * requests as f64).round() as u64
            }
            _ => 0,
        };
        if credits.is_none() && recorded + estimated == 0 {
            continue;
        }
        // First message id is stable for the life of the turn; the index is
        // the fallback (the turn list is append-only).
        let turn_id = t["message_ids"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(text)
            .unwrap_or_else(|| idx.to_string());
        let dur = t["turn_duration"]
            .as_object()
            .map(|d| num(&d["secs"]) as i64 * 1000 + num(&d["nanos"]) as i64 / 1_000_000);
        let ts_end = epoch_ms(&t["end_timestamp"]);
        let ts_start = match (ts_end, dur) {
            (Some(e), Some(d)) => Some(e - d),
            _ => ts_end,
        };
        let model = model_or_auto(text(&t["model"]).or_else(|| fallback_model.clone()));
        let (cost_usd, cost_source) = credit_cost(credits);
        out.push(UsageEvent {
            dedup_key: format!("{CLI_PREFIX}{session_id}:{turn_id}"),
            app: apps::KIRO.into(),
            session_id: Some(session_id.clone()),
            project: project.clone(),
            provider_id: Some("kiro".into()),
            model: model.clone(),
            request_model: model,
            ts_start,
            ts_end,
            // Counter names mirror Anthropic's: input excludes cache.
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_5m_tokens: cache_write,
            unclassified_tokens: estimated,
            credits,
            cost_usd,
            cost_source,
            duration_ms: dur,
            active_ms: dur,
            status: text(&t["end_reason"]),
            provenance: Provenance::LocalJsonl,
            raw_ref: Some(format!("{}#turn{idx}", path.display())),
            ..Default::default()
        });
    }
    out
}

fn scan_cli(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
    let mut out = ScanOutcome::default();
    let Some((_, memo)) = changed(item, store)? else {
        return Ok(out);
    };
    // The file is rewritten in place; a half-written read just fails to parse
    // and is retried next pass (memo not advanced).
    let doc = match std::fs::read(&item.path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        Some(d) => d,
        None => {
            out.notes.push(format!(
                "{}: unparseable kiro-cli session",
                item.path.display()
            ));
            return Ok(out);
        }
    };
    out.events = parse_cli_session(&doc, &item.path);
    out.pending_cursor = Some(memo_cursor(item, &memo));
    Ok(out)
}

// ─────────────────────────── Kiro IDE v2 ────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct V2State {
    exec: Option<String>,
    turn_ts: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct V2Meta {
    session_id: Option<String>,
    model: Option<String>,
    project: Option<String>,
}

fn v2_meta(messages: &Path) -> V2Meta {
    let Some(dir) = messages.parent() else {
        return V2Meta::default();
    };
    let doc: Value = std::fs::read(dir.join("session.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    V2Meta {
        session_id: text(&doc["id"])
            .or_else(|| dir.file_name().map(|s| s.to_string_lossy().to_string())),
        model: text(&doc["modelId"]),
        project: doc["workspacePaths"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(text),
    }
}

pub(crate) fn parse_v2(
    item: &SourceItem,
    from: u64,
    data: &[u8],
    prior_state: Option<&str>,
    meta: &V2Meta,
) -> ScanOutcome {
    let (seg, consumed) = complete_lines(data);
    let mut out = ScanOutcome {
        consumed,
        ..Default::default()
    };
    let mut st: V2State = prior_state
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let session_id = meta.session_id.clone().unwrap_or_default();
    let model = model_or_auto(meta.model.clone());

    let mut pos = 0u64;
    for line in seg.split(|&b| b == b'\n') {
        let line_start = from + pos;
        pos += line.len() as u64 + 1;
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let p = &v["payload"];
        match p.get("type").and_then(Value::as_str).unwrap_or("") {
            "turn_start" => {
                st.exec = text(&p["executionId"]);
                st.turn_ts = epoch_ms(&v["timestamp"]);
            }
            "turn_end" => {
                st.exec = None;
                st.turn_ts = None;
            }
            "usage_summary" => {
                let mut credits = 0.0;
                if let Some(arr) = p["promptTurnSummaries"].as_array() {
                    for s in arr {
                        if let Some(c) = fnum(&s["usage"]).filter(|c| c.is_finite() && *c > 0.0) {
                            credits += c;
                        }
                    }
                }
                if credits <= 0.0 {
                    continue;
                }
                let exec = text(&p["executionId"]).or_else(|| st.exec.clone());
                let ts = epoch_ms(&v["timestamp"]);
                let start = st.turn_ts.or(ts);
                // Offset keeps several summaries of one turn apart; the file
                // is append-only (rewrites are pinned by the cursor layer).
                let key = match &exec {
                    Some(e) => format!("kiro-v2:{session_id}:{e}@{line_start}"),
                    None => format!("kiro-v2:{session_id}@{line_start}"),
                };
                out.events.push(UsageEvent {
                    dedup_key: key,
                    app: apps::KIRO.into(),
                    session_id: Some(session_id.clone()),
                    project: meta.project.clone(),
                    provider_id: Some("kiro".into()),
                    model: model.clone(),
                    request_model: model.clone(),
                    ts_start: start,
                    ts_end: ts,
                    duration_ms: crate::normalize::derived_duration(start, ts),
                    credits: Some(credits),
                    cost_usd: Some(credits * USD_PER_CREDIT),
                    cost_source: Some(CostSource::ProviderReported),
                    provenance: Provenance::LocalJsonl,
                    raw_ref: Some(format!("{}@{line_start}", item.path.display())),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
    out.new_state = serde_json::to_string(&st).ok();
    out
}

fn discover_v2() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(sessions_root()) else {
        return out;
    };
    for hash in rd.flatten() {
        if hash.file_name() == "cli" || !hash.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(sess) = std::fs::read_dir(hash.path()) else {
            continue;
        };
        for s in sess.flatten() {
            if !s.file_name().to_string_lossy().starts_with("sess_") {
                continue;
            }
            let m = s.path().join("messages.jsonl");
            if m.is_file() {
                out.push(m);
            }
        }
    }
    out.sort();
    out
}

// ─────────────────────────── Kiro IDE v1 ────────────────────────────

fn str_field(v: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|n| text(&v[*n]).map(|s| s.trim().to_string()))
}

/// Parse one v1 execution document; `None` when it carries no metered credits.
pub(crate) fn parse_v1_execution(
    doc: &Value,
    path: &Path,
    project: Option<&str>,
) -> Option<UsageEvent> {
    if doc.get("executions").is_some_and(Value::is_array) {
        return None; // session index, not an execution
    }
    let mut credits = 0.0;
    for e in doc["usageSummary"].as_array()? {
        if let Some(c) = fnum(&e["usage"]).filter(|c| c.is_finite() && *c > 0.0) {
            credits += c;
        }
    }
    if credits <= 0.0 {
        return None;
    }
    let meta = &doc["metadata"];
    let exec = str_field(doc, &["executionId", "id"]).unwrap_or_else(|| {
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let session = str_field(
        doc,
        &["sessionId", "chatSessionId", "conversationId", "workflowId"],
    )
    .or_else(|| str_field(meta, &["workflowId", "sessionId"]))
    .or_else(|| {
        path.parent()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().to_string())
    });
    let model = model_or_auto(
        str_field(doc, &["modelId", "modelID", "modelName", "model"])
            .or_else(|| str_field(&doc["model"], &["id", "name"]))
            .or_else(|| str_field(meta, &["modelId", "modelID", "modelName"])),
    );
    let pick_ts = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| epoch_ms(&doc[*n]).or_else(|| epoch_ms(&meta[*n])))
    };
    let ts_start = pick_ts(&["startTime", "createdAt", "timestamp"]);
    let ts_end = pick_ts(&["endTime", "updatedAt"]);
    Some(UsageEvent {
        dedup_key: format!("{IDE_V1_PREFIX}{}:{exec}", session.as_deref().unwrap_or("")),
        app: apps::KIRO.into(),
        session_id: session,
        project: project.map(Into::into),
        provider_id: Some("kiro".into()),
        model: model.clone(),
        request_model: model,
        ts_start,
        ts_end,
        duration_ms: crate::normalize::derived_duration(ts_start, ts_end),
        credits: Some(credits),
        cost_usd: Some(credits * USD_PER_CREDIT),
        cost_source: Some(CostSource::ProviderReported),
        provenance: Provenance::LocalJsonl,
        raw_ref: Some(path.display().to_string()),
        ..Default::default()
    })
}

/// `workspaceStorage/<hash>/workspace.json` → `folder` as a plain path.
fn v1_project(agent_dir: &Path, hash: &str) -> Option<String> {
    let ws = workspace_storage(agent_dir)?
        .join(hash)
        .join("workspace.json");
    let doc: Value = serde_json::from_slice(&std::fs::read(ws).ok()?).ok()?;
    let folder = text(&doc["folder"])?;
    let raw = folder
        .strip_prefix("file:///")
        .or_else(|| folder.strip_prefix("file://"))
        .unwrap_or(&folder);
    Some(super::grok::pct_decode(raw))
}

fn is_ws_hash(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

fn extensionless_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_file() && p.extension().is_none() {
            out.push(p);
        } else if ft.is_dir() && depth > 0 {
            extensionless_files(&p, out, depth - 1);
        }
    }
}

fn discover_v1() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for agent in agent_dirs() {
        let Ok(rd) = std::fs::read_dir(&agent) else {
            continue;
        };
        for ws in rd.flatten() {
            if ws.file_type().is_ok_and(|t| t.is_dir())
                && is_ws_hash(&ws.file_name().to_string_lossy())
            {
                extensionless_files(&ws.path(), &mut out, 1);
            }
        }
    }
    out.sort();
    out
}

fn scan_v1(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
    let mut out = ScanOutcome::default();
    let Some((meta, memo)) = changed(item, store)? else {
        return Ok(out);
    };
    out.pending_cursor = Some(memo_cursor(item, &memo));
    if meta.len() > MAX_V1_FILE {
        out.skipped += 1;
        return Ok(out);
    }
    let bytes = std::fs::read(&item.path)?;
    // Cheap pre-filter: most execution files carry no metering block.
    const NEEDLE: &[u8] = b"usageSummary";
    if !bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
        return Ok(out);
    }
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        // Possibly mid-write: retry next pass.
        out.pending_cursor = None;
        return Ok(out);
    };
    // <agent>/<hash>/[<dir>/]<file>: find the workspace hash component.
    let (agent, hash) = item
        .path
        .ancestors()
        .skip(1)
        .find_map(|a| {
            let n = a.file_name()?.to_string_lossy().to_string();
            is_ws_hash(&n).then(|| (a.parent().map(Path::to_path_buf), n))
        })
        .unwrap_or((None, String::new()));
    let project = agent.as_deref().and_then(|a| v1_project(a, &hash));
    out.events
        .extend(parse_v1_execution(&doc, &item.path, project.as_deref()));
    Ok(out)
}

// ───────────────────────────── adapter ──────────────────────────────

impl SourceAdapter for Kiro {
    fn id(&self) -> &'static str {
        apps::KIRO
    }
    fn display_name(&self) -> &'static str {
        "Kiro"
    }
    /// Vendor-metered credits per turn — exact; tokens only when recorded.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        let mut v = vec![sessions_root()];
        v.extend(agent_dirs());
        v
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(cli_root()) {
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
                .collect();
            files.sort();
            out.extend(files.into_iter().map(|p| SourceItem {
                key: format!("{CLI_PREFIX}{}", p.to_string_lossy()),
                path: p,
                kind: SourceKind::Sqlite,
            }));
        }
        out.extend(discover_v2().into_iter().map(|p| SourceItem {
            key: p.to_string_lossy().to_string(),
            path: p,
            kind: SourceKind::Jsonl,
        }));
        out.extend(discover_v1().into_iter().map(|p| SourceItem {
            key: format!("{IDE_V1_PREFIX}{}", p.to_string_lossy()),
            path: p,
            kind: SourceKind::Sqlite,
        }));
        Ok(out)
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        from: u64,
        data: &[u8],
        prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        Ok(parse_v2(
            item,
            from,
            data,
            prior_state,
            &v2_meta(&item.path),
        ))
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        if item.key.starts_with(CLI_PREFIX) {
            scan_cli(item, store)
        } else {
            scan_v1(item, store)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cli_doc() -> Value {
        json!({
            "session_id": "s-1",
            "cwd": "F:\\Proj",
            "session_state": {
                "conversation_metadata": {
                    "user_turn_metadatas": [
                        {
                            "message_ids": ["m-1", "m-2"],
                            "turn_duration": {"secs": 76, "nanos": 326_689_400},
                            "end_reason": "UserTurnEnd",
                            "end_timestamp": "2026-10-26T08:52:35.557753500Z",
                            "input_token_count": 0,
                            "output_token_count": 0,
                            "cache_read_input_token_count": 0,
                            "cache_write_input_token_count": 0,
                            "model": "claude-sonnet-5.5",
                            "total_request_count": 2,
                            "context_usage_percentage": 10.0,
                            "final_context_usage_percentage": 20.0,
                            "metering_usage": [
                                {"value": 0.5, "unit": "credit", "unitPlural": "credits"},
                                {"value": 0.25, "unit": "credit", "unitPlural": "credits"}
                            ]
                        },
                        {
                            "message_ids": ["m-3"],
                            "end_timestamp": "2026-10-26T09:00:00Z",
                            "input_token_count": 100,
                            "output_token_count": 20,
                            "cache_read_input_token_count": 5,
                            "metering_usage": []
                        },
                        {
                            "message_ids": ["m-4"],
                            "end_timestamp": "2026-10-26T09:01:00Z",
                            "metering_usage": []
                        }
                    ]
                },
                "rts_model_state": {"model_info": {"model_id": "auto", "context_window_tokens": 100000}}
            }
        })
    }

    #[test]
    fn cli_turns_become_credit_events() {
        let evs = parse_cli_session(&cli_doc(), Path::new("/k/sessions/cli/s-1.json"));
        assert_eq!(evs.len(), 2, "empty third turn skipped");
        let a = &evs[0];
        assert_eq!(a.dedup_key, "kiro-cli:s-1:m-1");
        assert_eq!(a.app, apps::KIRO);
        assert_eq!(a.project.as_deref(), Some("F:\\Proj"));
        assert_eq!(a.model.as_deref(), Some("claude-sonnet-5.5"));
        assert_eq!(a.credits, Some(0.75));
        assert!((a.cost_usd.unwrap() - 0.75 * USD_PER_CREDIT).abs() < 1e-12);
        assert_eq!(a.cost_source, Some(CostSource::ProviderReported));
        // 2 requests × avg(10%, 20%) × 100k window.
        assert_eq!(a.unclassified_tokens, 30_000);
        assert_eq!(a.active_ms, Some(76_326));
        assert_eq!(a.duration_ms, Some(76_326));
        assert_eq!(a.ts_end.unwrap() - a.ts_start.unwrap(), 76_326);
        assert_eq!(a.status.as_deref(), Some("UserTurnEnd"));
        assert!(a.is_billable());

        let b = &evs[1];
        assert_eq!(b.dedup_key, "kiro-cli:s-1:m-3");
        assert_eq!(b.credits, None);
        assert_eq!(b.cost_usd, None);
        assert_eq!(b.unclassified_tokens, 0, "real counters are never padded");
        assert_eq!(
            (b.input_tokens, b.output_tokens, b.cache_read_tokens),
            (100, 20, 5)
        );
        // Falls back to the session model; "auto" is the router.
        assert_eq!(b.model.as_deref(), Some("kiro-auto"));
    }

    #[test]
    fn cli_doc_without_turns_is_empty() {
        let evs = parse_cli_session(&json!({"session_id": "x"}), Path::new("x.json"));
        assert!(evs.is_empty());
    }

    fn v2_item() -> SourceItem {
        SourceItem {
            key: "/k/sessions/h/sess_1/messages.jsonl".into(),
            path: PathBuf::from("/k/sessions/h/sess_1/messages.jsonl"),
            kind: SourceKind::Jsonl,
        }
    }

    #[test]
    fn v2_usage_summary_carries_turn_context_across_segments() {
        let meta = V2Meta {
            session_id: Some("sess_1".into()),
            model: Some("claude-opus-5".into()),
            project: Some("/w/proj".into()),
        };
        let seg1 = br#"{"id":"1","timestamp":"2026-10-26T08:00:00Z","payload":{"type":"user","content":"hi"}}
{"id":"2","timestamp":"2026-10-26T08:00:01Z","payload":{"type":"turn_start","executionId":"ex-9"}}
"#;
        let o1 = parse_v2(&v2_item(), 0, seg1, None, &meta);
        assert!(o1.events.is_empty());
        let st = o1.new_state.unwrap();

        let seg2 = br#"{"id":"3","timestamp":"2026-10-26T08:00:11Z","payload":{"type":"usage_summary","promptTurnSummaries":[{"usage":1.5,"usedTools":["readFile"]},{"usage":null}]}}
{"id":"4","timestamp":"2026-10-26T08:00:12Z","payload":{"type":"turn_end"}}
{"id":"5","timestamp":"2026-10-26T08:01:00Z","payload":{"type":"usage_summary","promptTurnSummaries":[{"usage":0}]}}
{"id":"6","ts":"#;
        let from = seg1.len() as u64;
        let o2 = parse_v2(&v2_item(), from, seg2, Some(&st), &meta);
        assert_eq!(o2.events.len(), 1, "zero-credit summary skipped");
        assert!(o2.consumed < seg2.len() as u64, "partial tail left");
        let e = &o2.events[0];
        assert_eq!(e.dedup_key, format!("kiro-v2:sess_1:ex-9@{from}"));
        assert_eq!(e.credits, Some(1.5));
        assert_eq!(e.duration_ms, Some(10_000));
        assert_eq!(e.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(e.project.as_deref(), Some("/w/proj"));
    }

    #[test]
    fn v1_execution_with_usage_summary() {
        let doc = json!({
            "executionId": "ex-1",
            "chatSessionId": "cs-1",
            "modelId": "claude-sonnet-4.5",
            "startTime": 1_793_000_000_000i64,
            "endTime": 1_793_000_030_000i64,
            "usageSummary": [{"usage": 2.0, "unit": "credit"}, {"usage": 0.5}]
        });
        let e = parse_v1_execution(&doc, Path::new("/a/h/f"), Some("F:/Proj")).unwrap();
        assert_eq!(e.dedup_key, "kiro-ide:cs-1:ex-1");
        assert_eq!(e.credits, Some(2.5));
        assert_eq!(e.duration_ms, Some(30_000));
        assert_eq!(e.model.as_deref(), Some("claude-sonnet-4.5"));
        assert_eq!(e.project.as_deref(), Some("F:/Proj"));
    }

    #[test]
    fn v1_index_and_unmetered_files_skipped() {
        assert!(parse_v1_execution(&json!({"executions": []}), Path::new("f"), None).is_none());
        assert!(parse_v1_execution(&json!({"executionId": "e"}), Path::new("f"), None).is_none());
        assert!(
            parse_v1_execution(
                &json!({"usageSummary": [{"usage": 0}]}),
                Path::new("f"),
                None
            )
            .is_none()
        );
    }

    #[test]
    fn cli_scan_is_memoized_until_file_changes() {
        let dir = std::env::temp_dir().join(format!("gtt-kiro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s-1.json");
        std::fs::write(&p, serde_json::to_vec(&cli_doc()).unwrap()).unwrap();
        let item = SourceItem {
            key: format!("{CLI_PREFIX}{}", p.display()),
            path: p.clone(),
            kind: SourceKind::Sqlite,
        };
        let store = Store::open_memory().unwrap();
        let o1 = super::super::scan_and_commit(&Kiro, &store, &item);
        assert_eq!(o1.events.len(), 2);
        let o2 = super::super::scan_and_commit(&Kiro, &store, &item);
        assert!(o2.events.is_empty(), "unchanged file is not re-read");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
