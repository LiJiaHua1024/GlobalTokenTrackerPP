//! Cline adapter (cline/cline): two on-disk generations.
//!
//! - **v3 sessions** (current CLI + Desktop, same store): whole-file JSON docs
//!   `~/.cline/data/sessions/<sessionId>/<sessionId>.messages.json`
//!   (`{version, agent, sessionId, messages: [...]}`).
//!   Every assistant message carries its own per-request **delta**
//!   `metrics: {inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens}`
//!   (verified against live data: values fluctuate up AND down across messages,
//!   message `ts` is epoch ms with the same far-future clock skew seen in
//!   session filenames, sums match the sidecar `<id>.json`
//!   `metadata.usage/aggregateUsage` on completed sessions, running sessions
//!   have no sidecar totals yet). Per-message `modelInfo {id, provider}` plus
//!   sidecar top-level `model/provider/cwd` fill attribution; `ts` fills
//!   `ts_start`. No cost anywhere — rows are `CostSource`-less so pricing falls
//!   back to the book.
//! - **v2 tasks** (legacy CLI / VS Code extension): task transcripts live as
//!   whole-file JSON arrays — NOT append-only —
//!   `~/.cline/data/tasks/<taskId>/ui_messages.json`
//!   (`apps/vscode/src/sdk/legacy-state-reader.ts`) and
//!   `<config>/<Product>/User/globalStorage/saoudrizwan.claude-dev/tasks/<taskId>/ui_messages.json`
//!   (`core/storage/disk.ts` → `globalStorageFsPath/tasks/<id>`).
//!
//! Both generations use the Sqlite-kind (adapter-managed watermark) path with
//! a {len, mtime} freshness check: files are rewritten wholesale, never
//! appended.
//!
//! Usage rows in v2 `ui_messages.json` (`shared/ExtensionMessage.ts`,
//! `sdk/message-translator.ts`, `shared/getApiMetrics.ts`):
//!   {type:"say", say:"api_req_started",  text:'{"tokensIn":…,"cost":…}', ts}
//!   {type:"say", say:"api_req_finished", text:'{"tokensIn":…,"cost":…}', ts}
//!   {type:"say", say:"subagent_usage",   text:'{"tokensIn":…}', ts}
//!   {type:"say", say:"deleted_api_reqs", text:'{"tokensIn":…}', ts}
//!
//! Semantics (sdk `agent-events.ts` — the task.tokens telemetry contract):
//!   `tokensIn` is UNCACHED input only — tokensIn + cacheReads + cacheWrites
//!   is the total input, so `input_tokens` takes it verbatim.
//!   `cost` is Cline's own USD estimate → ProviderReported.
//!
//! Pairing: legacy tasks keep spinner `api_req_started` + metrics
//! `api_req_finished` as separate rows; the render pipeline pairs them
//! (`combineApiRequests`). Count the FINISHED row and drop the paired
//! started row's metrics; a started row whose metrics survive to EOF is a
//! modern SDK usage row — count it.
//!
//! `deleted_api_reqs` aggregates usage of deleted messages: emit it only on
//! the FIRST scan of a file (covers history imported after deletion). Later
//! scans skip it — the per-request rows it replaces are already in the
//! ledger and consumed tokens don't un-consume when a row is deleted.
//!
//! Model/provider attribution (v2): `task_metadata.json` `model_usage[]`
//! ({ts, model_id, model_provider_id}) picks the latest entry ≤ row ts;
//! `state/taskHistory.json` `cwdOnTaskInitialization`/`apiProvider` fill
//! project/provider (CLI data dir; editor hosts keep history in state.vscdb
//! which we deliberately don't read — project stays None there).

use super::{Capability, PendingCursor, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, fnum, num};
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Cline;

/// `~/.cline/data` (env `CLINE_DATA_DIR` > `CLINE_DIR/data`).
fn cli_data_root() -> PathBuf {
    if let Ok(d) = std::env::var("CLINE_DATA_DIR")
        && !d.is_empty()
    {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("CLINE_DIR")
        && !d.is_empty()
    {
        return PathBuf::from(d).join("data");
    }
    crate::sync::home(".cline/data")
}

/// All task dirs across hosts: CLI data dir + every editor's globalStorage.
/// Legacy v2 layout; kept while v2 installs still exist.
fn task_dirs() -> Vec<(PathBuf, PathBuf)> {
    // (tasks_root, data_root-for-state/taskHistory.json)
    let mut roots: Vec<(PathBuf, PathBuf)> = vec![(cli_data_root().join("tasks"), cli_data_root())];
    if let Some(cfg) = dirs::config_dir()
        && let Ok(rd) = std::fs::read_dir(&cfg)
    {
        for prod in rd.flatten() {
            let gs = prod
                .path()
                .join("User/globalStorage/saoudrizwan.claude-dev");
            if gs.is_dir() {
                roots.push((gs.join("tasks"), gs));
            }
        }
    }
    let mut out = Vec::new();
    for (tasks, data_root) in roots {
        if let Ok(rd) = std::fs::read_dir(&tasks) {
            for e in rd.flatten() {
                let d = e.path();
                if d.is_dir() && d.join("ui_messages.json").is_file() {
                    out.push((d, data_root.clone()));
                }
            }
        }
    }
    out
}

/// All v3 session files: every `<sessionId>.messages.json` under the CLI data
/// dir's `sessions/` (Desktop and CLI share this store). The v3 session file
/// is the authority — no fallback to `sessions.db` (it only indexes a subset
/// of live sessions) or `session-search.db` (FTS content index, no metrics).
fn session_files() -> Vec<PathBuf> {
    let root = cli_data_root().join("sessions");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let dir = e.path();
        if !dir.is_dir() {
            continue;
        }
        let id = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // Session dirs are `<sessionId>`; the transcript is
        // `<sessionId>.messages.json`. An exact-name match avoids picking up
        // stray sidecars.
        let cand = dir.join(format!("{id}.messages.json"));
        if cand.is_file() {
            out.push(cand);
            continue;
        }
        // Tolerate layout drift: any `*.messages.json` inside the dir.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for f in rd.flatten() {
                let p = f.path();
                if p.is_file()
                    && p.extension().is_some_and(|e| e == "json")
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(".messages.json"))
                {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// Sidecar `<sessionId>.json` next to a v3 messages file: session-level
/// model/provider/cwd attribution (`model`, `provider`, `cwd`).
fn session_sidecar(messages_path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let Some(stem) = messages_path.file_stem().and_then(|s| s.to_str()) else {
        return (None, None, None);
    };
    let id = stem.strip_suffix(".messages").unwrap_or(stem);
    let side = messages_path.parent().map(|d| d.join(format!("{id}.json")));
    let Some(side) = side.filter(|p| p.is_file()) else {
        return (None, None, None);
    };
    let Ok(s) = std::fs::read_to_string(&side) else {
        return (None, None, None);
    };
    let Ok(v) = serde_json::from_str::<Value>(&s) else {
        return (None, None, None);
    };
    (
        v.get("model").and_then(Value::as_str).map(String::from),
        v.get("provider").and_then(Value::as_str).map(String::from),
        v.get("cwd").and_then(Value::as_str).map(String::from),
    )
}

#[derive(Debug, Serialize, Deserialize)]
struct ClineState {
    /// First scan of this file? Controls `deleted_api_reqs` emission.
    cold: bool,
    len: u64,
    mtime_ms: i64,
}
impl Default for ClineState {
    fn default() -> Self {
        // No persisted state → this is the cold import; deleted_api_reqs
        // aggregates count (they cover history we never saw).
        Self {
            cold: true,
            len: 0,
            mtime_ms: 0,
        }
    }
}

/// task_metadata.json `model_usage[]` — nearest entry with `ts <= row_ts`.
fn model_usage_map(task_dir: &Path) -> Vec<(i64, String, Option<String>)> {
    let Ok(s) = std::fs::read_to_string(task_dir.join("task_metadata.json")) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<Value>(&s) else {
        return vec![];
    };
    let mut out: Vec<(i64, String, Option<String>)> = v["model_usage"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some((
                        epoch_ms(&e["ts"])?,
                        e["model_id"].as_str()?.to_string(),
                        e["model_provider_id"].as_str().map(String::from),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(ts, ..)| *ts);
    out
}

/// `state/taskHistory.json` — taskId → (cwd, apiProvider).
fn task_history(data_root: &Path) -> Vec<(String, Option<String>, Option<String>)> {
    let Ok(s) = std::fs::read_to_string(data_root.join("state/taskHistory.json")) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_str::<Value>(&s) else {
        return vec![];
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|h| {
                    Some((
                        h["id"].as_str()?.to_string(),
                        h["cwdOnTaskInitialization"].as_str().map(String::from),
                        h["apiProvider"].as_str().map(String::from),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

struct Pending {
    idx: usize,
}

impl SourceAdapter for Cline {
    fn id(&self) -> &'static str {
        apps::CLINE
    }
    fn display_name(&self) -> &'static str {
        "Cline"
    }
    /// Per-request token deltas + Cline's own cost estimate — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![cli_data_root()];
        if let Some(cfg) = dirs::config_dir()
            && let Ok(rd) = std::fs::read_dir(&cfg)
        {
            for prod in rd.flatten() {
                let gs = prod
                    .path()
                    .join("User/globalStorage/saoudrizwan.claude-dev");
                if gs.is_dir() {
                    roots.push(gs);
                }
            }
        }
        roots
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut out: Vec<SourceItem> = session_files()
            .into_iter()
            .map(|p| SourceItem {
                key: format!("cline-session:{}", p.to_string_lossy()),
                path: p,
                kind: SourceKind::Sqlite,
            })
            .collect();
        out.extend(task_dirs().into_iter().map(|(dir, _)| SourceItem {
            key: dir.to_string_lossy().to_string(),
            path: dir,
            kind: SourceKind::Sqlite,
        }));
        Ok(out)
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        // v3 session transcript vs legacy v2 task dir — routed by filename.
        if item
            .path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".messages.json"))
        {
            return scan_session(item, store);
        }
        scan_task(item, store)
    }
}

/// Legacy v2 task scan: `tasks/<id>/ui_messages.json`.
fn scan_task(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
    let ui = item.path.join("ui_messages.json");
    let meta = std::fs::metadata(&ui)?;
    let mut state: ClineState = store
        .load_cursor(&item.key)?
        .state
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut out = ScanOutcome::default();
    if !state.cold && state.len == meta.len() && state.mtime_ms == mtime_ms {
        return Ok(out); // unchanged since last scan
    }
    state.len = meta.len();
    state.mtime_ms = mtime_ms;

    let task_id = item
        .path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let usage_models = model_usage_map(&item.path);
    // data root is ../../.. relative to tasks/<id> only for CLI layout;
    // editor layout has the same structure under globalStorage.
    let data_root = item.path.parent().and_then(Path::parent);
    let history = data_root.map(task_history).unwrap_or_default();
    let hist = history.iter().find(|(id, ..)| *id == task_id);
    let (project, provider) = hist
        .map(|(_, cwd, prov)| (cwd.clone(), prov.clone()))
        .unwrap_or_default();

    let msgs: Vec<Value> = match std::fs::read_to_string(&ui)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
    {
        Some(m) => m,
        None => {
            out.notes
                .push(format!("{}: unparseable ui_messages", ui.display()));
            return Ok(out);
        }
    };

    // Greedy sequential pairing à la combineApiRequests: a metric-bearing
    // api_req_started is superseded by the next api_req_finished.
    let mut pending: Option<Pending> = None;
    let emit = |out: &mut Vec<UsageEvent>, msg: &Value, info: &Value, _i: usize, suffix: &str| {
        let ts = epoch_ms(&msg["ts"]);
        let tokens_in = num(&info["tokensIn"]);
        let tokens_out = num(&info["tokensOut"]);
        let cache_reads = num(&info["cacheReads"]);
        let cache_writes = num(&info["cacheWrites"]);
        let cost = fnum(&info["cost"]);
        if tokens_in + tokens_out + cache_reads + cache_writes == 0
            && !cost.is_some_and(|c| c > 0.0)
        {
            return false;
        }
        let model_at = ts.and_then(|t| {
            usage_models
                .iter()
                .rev()
                .find(|(m_ts, ..)| *m_ts <= t)
                .map(|(_, m, p)| (m.clone(), p.clone()))
        });
        out.push(UsageEvent {
            dedup_key: format!("cline:{task_id}:{}{suffix}", ts.unwrap_or(0)),
            app: apps::CLINE.into(),
            session_id: Some(task_id.clone()),
            project: project.clone(),
            provider_id: provider
                .clone()
                .or_else(|| model_at.clone().and_then(|x| x.1)),
            model: model_at.map(|x| x.0),
            ts_start: ts,
            input_tokens: tokens_in,
            output_tokens: tokens_out,
            reasoning_tokens: num(&info["reasoningTokenCount"]),
            cache_read_tokens: cache_reads,
            cache_write_5m_tokens: cache_writes,
            cost_usd: cost,
            cost_source: cost.map(|_| CostSource::ProviderReported),
            provenance: Provenance::LocalJsonl,
            raw_ref: Some(format!("{}#ts={}", ui.display(), ts.unwrap_or(0))),
            ..Default::default()
        });
        true
    };

    for (i, msg) in msgs.iter().enumerate() {
        if msg.get("type").and_then(Value::as_str) != Some("say") {
            continue;
        }
        let say = msg.get("say").and_then(Value::as_str).unwrap_or("");
        let info: Value = msg
            .get("text")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        match say {
            "api_req_started" => {
                let has_metrics = num(&info["tokensIn"])
                    + num(&info["tokensOut"])
                    + num(&info["cacheReads"])
                    + num(&info["cacheWrites"])
                    > 0
                    || fnum(&info["cost"]).is_some_and(|c| c > 0.0);
                if has_metrics {
                    // Replace any earlier pending metrics row (unpaired).
                    if let Some(p) = pending.take() {
                        emit(
                            &mut out.events,
                            &msgs[p.idx],
                            &serde_json::from_str(msgs[p.idx]["text"].as_str().unwrap_or("{}"))
                                .unwrap_or_default(),
                            p.idx,
                            ":started",
                        );
                    }
                    pending = Some(Pending { idx: i });
                }
                // Spinner rows carry no metrics — ignore entirely.
            }
            "api_req_finished" => {
                // Finished wins the pair: drop pending started metrics.
                pending = None;
                emit(&mut out.events, msg, &info, i, ":finished");
            }
            "subagent_usage" => {
                emit(&mut out.events, msg, &info, i, ":sub");
            }
            "deleted_api_reqs" if state.cold => {
                emit(&mut out.events, msg, &info, i, ":deleted");
            }
            _ => {}
        }
    }
    if let Some(p) = pending.take() {
        let info: Value =
            serde_json::from_str(msgs[p.idx]["text"].as_str().unwrap_or("{}")).unwrap_or_default();
        emit(&mut out.events, &msgs[p.idx], &info, p.idx, ":started");
    }

    state.cold = false;
    let fp = [item.path.join("task_metadata.json"), ui.clone()]
        .into_iter()
        .find(|p| p.is_file());
    if let Some(fp) = fp {
        out.pending_cursor = Some(PendingCursor {
            key: item.key.clone(),
            path: fp,
            offset: state.len,
            mtime_ms: state.mtime_ms,
            state: Some(serde_json::to_string(&state)?),
        });
    }
    Ok(out)
}

/// v3 session scan: `<sessionId>.messages.json`.
///
/// Whole-file doc `{messages: [...]}` rewritten on every turn — same
/// {len, mtime} fast path as v2. Each assistant message's `metrics` is one
/// per-request delta row (dedup key `cline:<session>:<msgId>`; the id is
/// stable across rewrites, unlike row `ts`). No cost is recorded anywhere
/// in v3 — rows carry no `CostSource` so the price book prices them.
fn scan_session(item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
    let ui = &item.path;
    let meta = std::fs::metadata(ui)?;
    let mut state: ClineState = store
        .load_cursor(&item.key)?
        .state
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut out = ScanOutcome::default();
    if !state.cold && state.len == meta.len() && state.mtime_ms == mtime_ms {
        return Ok(out); // unchanged since last scan
    }
    state.cold = false;
    state.len = meta.len();
    state.mtime_ms = mtime_ms;

    let session_id = ui
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_suffix(".messages"))
        .or_else(|| {
            ui.parent()
                .and_then(|d| d.file_name())
                .and_then(|s| s.to_str())
        })
        .unwrap_or_default()
        .to_string();
    let (side_model, side_provider, project) = session_sidecar(ui);

    let doc: Value = match std::fs::read_to_string(ui)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(d) => d,
        None => {
            out.notes
                .push(format!("{}: unparseable messages", ui.display()));
            return Ok(out);
        }
    };
    let msgs: &[Value] = match &doc {
        Value::Array(a) => a,
        Value::Object(m) => m
            .get("messages")
            .and_then(Value::as_array)
            .map_or(&[], |a| a),
        _ => &[],
    };
    if doc.is_array() {
        out.notes
            .push(format!("{}: legacy top-level array shape", ui.display()));
    }

    for msg in msgs {
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let met = &msg["metrics"];
        if !met.is_object() {
            continue;
        }
        let tokens_in = num(&met["inputTokens"]);
        let tokens_out = num(&met["outputTokens"]);
        let cache_reads = num(&met["cacheReadTokens"]);
        let cache_writes = num(&met["cacheWriteTokens"]);
        // Live data: every assistant turn carries nonzero output; error-only
        // rows (e.g. socket disconnect, displayOnly) carry no metrics object
        // values at all. All-zero rows are not billable — skip.
        if tokens_in + tokens_out + cache_reads + cache_writes == 0 {
            continue;
        }
        let msg_id = msg.get("id").and_then(Value::as_str).unwrap_or_default();
        let ts = epoch_ms(&msg["ts"]);
        let info = &msg["modelInfo"];
        let model = info
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| side_model.clone());
        let provider = info
            .get("provider")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| side_provider.clone());
        out.events.push(UsageEvent {
            dedup_key: format!("cline:{session_id}:{msg_id}"),
            app: apps::CLINE.into(),
            session_id: Some(session_id.clone()),
            project: project.clone(),
            provider_id: provider,
            model,
            ts_start: ts,
            input_tokens: tokens_in,
            output_tokens: tokens_out,
            cache_read_tokens: cache_reads,
            cache_write_5m_tokens: cache_writes,
            cost_usd: None,
            cost_source: None,
            provenance: Provenance::LocalJsonl,
            raw_ref: Some(format!("{}#{}", ui.display(), msg_id)),
            ..Default::default()
        });
    }

    out.pending_cursor = Some(PendingCursor {
        key: item.key.clone(),
        path: ui.clone(),
        offset: state.len,
        mtime_ms: state.mtime_ms,
        state: Some(serde_json::to_string(&state)?),
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::scan_and_commit;

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn task_fixture(tag: &str) -> (TmpDir, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("gtt-cline-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let task = root.join("tasks").join("t-1");
        std::fs::create_dir_all(&task).unwrap();
        (TmpDir(root.clone()), task, root)
    }

    fn say(say: &str, ts: i64, text: &str) -> String {
        format!(
            r#"{{"type":"say","say":"{say}","ts":{ts},"text":{}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    #[test]
    fn metrics_rows_paired_and_counted() {
        let (_root, task, _data) = task_fixture("pair");
        // legacy pair: started(metrics) + finished(metrics) → count finished only
        // modern: a lone metric-bearing started → counted
        // spinner: started without metrics → skipped
        // subagent_usage + deleted_api_reqs (cold) → counted
        std::fs::write(
            task.join("task_metadata.json"),
            r#"{"model_usage":[{"ts":1779256800090,"model_id":"claude-sonnet-4.6","model_provider_id":"cline"}],"files_in_context":[],"environment_history":[]}"#,
        )
        .unwrap();
        let msgs = format!(
            "[{},{},{},{},{},{}]",
            say("api_req_started", 1779256800100, r#"{"request":"do x"}"#), // spinner
            say(
                "api_req_started",
                1779256800101,
                r#"{"tokensIn":50,"tokensOut":10,"cacheReads":5,"cacheWrites":2,"cost":0.01,"reasoningTokenCount":3}"#
            ), // paired legacy started (in-place updated)
            say(
                "api_req_finished",
                1779256800102,
                r#"{"tokensIn":55,"tokensOut":12,"cacheReads":6,"cacheWrites":3,"cost":0.02}"#
            ), // finished wins
            say(
                "api_req_started",
                1779256800200,
                r#"{"tokensIn":70,"tokensOut":8,"cost":0.03}"#
            ), // unpaired modern usage row
            say(
                "subagent_usage",
                1779256800300,
                r#"{"source":"subagents","tokensIn":500,"tokensOut":40,"cacheWrites":0,"cacheReads":0,"cost":0.1}"#
            ),
            say(
                "deleted_api_reqs",
                1779256800400,
                r#"{"tokensIn":900,"tokensOut":60,"cost":0.5}"#
            ),
        );
        std::fs::write(task.join("ui_messages.json"), msgs).unwrap();
        let item = SourceItem {
            key: task.to_string_lossy().to_string(),
            path: task.clone(),
            kind: SourceKind::Sqlite,
        };
        let store = Store::open_memory().unwrap();
        let out = scan_and_commit(&Cline, &store, &item);
        let keys: Vec<&str> = out.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "cline:t-1:1779256800102:finished",
                "cline:t-1:1779256800300:sub",
                "cline:t-1:1779256800400:deleted",
                // Unpaired started survives to EOF → emitted last.
                "cline:t-1:1779256800200:started",
            ],
            "paired started row must be superseded by the finished row"
        );
        let e = &out.events[0];
        assert_eq!(e.input_tokens, 55);
        assert_eq!(e.output_tokens, 12);
        assert_eq!(e.cache_read_tokens, 6);
        assert_eq!(e.cache_write_5m_tokens, 3);
        assert_eq!(e.cost_usd, Some(0.02));
        assert_eq!(e.cost_source, Some(CostSource::ProviderReported));
        // model_usage entry at ts=…090 applies (ts ≤ row ts)
        assert_eq!(e.model.as_deref(), Some("claude-sonnet-4.6"));
        assert_eq!(e.provider_id.as_deref(), Some("cline"));
        let e2 = out
            .events
            .iter()
            .find(|e| e.dedup_key.ends_with(":started"))
            .unwrap();
        assert_eq!(e2.input_tokens, 70);
        assert_eq!(e2.reasoning_tokens, 0);

        // Second scan, unchanged file → fast path, nothing re-emitted.
        let out2 = scan_and_commit(&Cline, &store, &item);
        assert!(out2.events.is_empty());
    }

    #[test]
    fn deleted_api_reqs_skipped_after_first_scan() {
        let (_root, task, _data) = task_fixture("warm");
        std::fs::write(task.join("ui_messages.json"), "[]").unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: task.to_string_lossy().to_string(),
            path: task.clone(),
            kind: SourceKind::Sqlite,
        };
        // First scan establishes cold=false (empty file).
        scan_and_commit(&Cline, &store, &item);
        // User then deletes messages → file rewritten with a deleted_api_reqs
        // aggregate; the already-counted originals must not double.
        let msgs = format!(
            "[{},{}]",
            say(
                "api_req_started",
                1779256800500,
                r#"{"tokensIn":10,"tokensOut":5,"cost":0.001}"#
            ),
            say(
                "deleted_api_reqs",
                1779256800600,
                r#"{"tokensIn":999,"tokensOut":9,"cost":9.0}"#
            ),
        );
        std::fs::write(task.join("ui_messages.json"), msgs).unwrap();
        let out = scan_and_commit(&Cline, &store, &item);
        let keys: Vec<&str> = out.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(keys, vec!["cline:t-1:1779256800500:started"]);
    }

    fn session_fixture(tag: &str) -> (TmpDir, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("gtt-cline-sess-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("session_abc");
        std::fs::create_dir_all(&dir).unwrap();
        (TmpDir(root), dir)
    }

    fn assist_msg(id: &str, ts: i64, metrics: &str, model: &str, provider: &str) -> String {
        format!(
            r#"{{"id":"{id}","role":"assistant","content":[{{"type":"text","text":"hi"}}],"ts":{ts},"modelInfo":{{"id":"{model}","provider":"{provider}"}},"metrics":{metrics}}}"#
        )
    }

    #[test]
    fn session_messages_counted_per_request() {
        let (_root, dir) = session_fixture("count");
        // Sidecar carries session-level attribution + cwd.
        std::fs::write(
            dir.join("session_abc.json"),
            r#"{"version":1,"session_id":"session_abc","source":"cli","provider":"cline","model":"cline/m","cwd":"/proj"}"#,
        )
        .unwrap();
        let doc = format!(
            r#"{{"version":1,"sessionId":"session_abc","messages":[{},{},{},{},{},{},{}]}}"#,
            // user row: never counted
            r#"{"id":"u1","role":"user","content":[{"type":"text","text":"hi"}],"ts":1779256800001}"#,
            // two delta rows, fluctuating values (NOT cumulative)
            assist_msg(
                "m1",
                1779256800002,
                r#"{"inputTokens":100,"outputTokens":10,"cacheReadTokens":50,"cacheWriteTokens":0}"#,
                "cline/m1",
                "cline"
            ),
            assist_msg(
                "m2",
                1779256800003,
                r#"{"inputTokens":40,"outputTokens":5,"cacheReadTokens":10,"cacheWriteTokens":2}"#,
                "cline/m2",
                "cline"
            ),
            // error-only row: assistant without metrics values → skipped
            r#"{"id":"e1","role":"assistant","content":[{"type":"text","text":"boom"}],"ts":1779256800004,"metadata":{"displayOnly":true},"modelInfo":{"id":"cline/m2","provider":"cline"}}"#,
            // all-zero metrics → skipped
            assist_msg(
                "m3",
                1779256800005,
                r#"{"inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0}"#,
                "cline/m2",
                "cline"
            ),
            // per-message modelInfo missing → sidecar fallback /proj
            r#"{"id":"m4","role":"assistant","content":[{"type":"text","text":"hi"}],"ts":1779256800006,"metrics":{"inputTokens":7,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0}}"#,
            // missing id → still counted
            r#"{"role":"assistant","content":[{"type":"text","text":"hi"}],"ts":1779256800007,"metrics":{"inputTokens":3,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0}}"#,
        );
        let path = dir.join("session_abc.messages.json");
        std::fs::write(&path, doc).unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: format!("cline-session:{}", path.to_string_lossy()),
            path: path.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&Cline, &store, &item);
        let keys: Vec<&str> = out.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "cline:session_abc:m1",
                "cline:session_abc:m2",
                "cline:session_abc:m4",
                "cline:session_abc:",
            ]
        );
        let e1 = &out.events[0];
        assert_eq!(e1.input_tokens, 100);
        assert_eq!(e1.output_tokens, 10);
        assert_eq!(e1.cache_read_tokens, 50);
        assert_eq!(e1.cache_write_5m_tokens, 0);
        assert_eq!(e1.model.as_deref(), Some("cline/m1"));
        assert_eq!(e1.provider_id.as_deref(), Some("cline"));
        assert_eq!(e1.session_id.as_deref(), Some("session_abc"));
        assert_eq!(e1.project.as_deref(), Some("/proj"));
        assert_eq!(e1.ts_start, Some(1779256800002));
        // v3 records no cost → price book fallback, not ProviderReported.
        assert_eq!(e1.cost_usd, None);
        assert_eq!(e1.cost_source, None);
        // Sidecar fallback for the row without modelInfo.
        let e3 = &out.events[2];
        assert_eq!(e3.model.as_deref(), Some("cline/m"));
        assert_eq!(e3.provider_id.as_deref(), Some("cline"));

        // Second scan, unchanged file → fast path, nothing re-emitted.
        let out2 = scan_and_commit(&Cline, &store, &item);
        assert!(out2.events.is_empty());
    }

    #[test]
    fn missing_usage_fields_tolerated() {
        let (_root, task, _data) = task_fixture("sparse");
        let msgs = format!(
            "[{},{}]",
            say("api_req_started", 1779256800001, r#"{"request":"x"}"#), // spinner only
            say("api_req_started", 1779256800002, r#"{"cost":0.0}"#),    // zero cost → skipped
        );
        std::fs::write(task.join("ui_messages.json"), msgs).unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: task.to_string_lossy().to_string(),
            path: task.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&Cline, &store, &item);
        assert!(out.events.is_empty());
    }

    #[test]
    fn session_rewrite_replays_without_dup_keys() {
        let (_root, dir) = session_fixture("replay");
        let path = dir.join("session_abc.messages.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"version":1,"sessionId":"session_abc","messages":[{}]}}"#,
                assist_msg(
                    "m1",
                    1779256800001,
                    r#"{"inputTokens":10,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0}"#,
                    "cline/m",
                    "cline"
                )
            ),
        )
        .unwrap();
        let store = Store::open_memory().unwrap();
        let item = SourceItem {
            key: format!("cline-session:{}", path.to_string_lossy()),
            path: path.clone(),
            kind: SourceKind::Sqlite,
        };
        let out = scan_and_commit(&Cline, &store, &item);
        assert_eq!(out.events.len(), 1);
        // New turn rewrites the whole file: BOTH rows are re-emitted (v3
        // files are whole-file snapshots), but stable dedup keys let the
        // store UPSERT collapse the replay instead of double-counting.
        std::fs::write(
            &path,
            format!(
                r#"{{"version":1,"sessionId":"session_abc","messages":[{},{}]}}"#,
                assist_msg(
                    "m1",
                    1779256800001,
                    r#"{"inputTokens":10,"outputTokens":2,"cacheReadTokens":0,"cacheWriteTokens":0}"#,
                    "cline/m",
                    "cline"
                ),
                assist_msg(
                    "m2",
                    1779256800002,
                    r#"{"inputTokens":20,"outputTokens":3,"cacheReadTokens":5,"cacheWriteTokens":0}"#,
                    "cline/m",
                    "cline"
                )
            ),
        )
        .unwrap();
        // mtime granularity can hide fast rewrites — force a visible change.
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2))
            .unwrap();
        drop(f);
        let out2 = scan_and_commit(&Cline, &store, &item);
        let keys: Vec<&str> = out2.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(keys, vec!["cline:session_abc:m1", "cline:session_abc:m2"]);
    }
}
