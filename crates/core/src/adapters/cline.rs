//! Cline adapter (cline/cline): task transcripts live as whole-file JSON
//! arrays — NOT append-only — so this uses the Sqlite-kind (adapter-managed
//! watermark) path with a {len, mtime} freshness check.
//!
//! Hosts and roots (verified against cline source):
//!   - CLI / standalone: `$CLINE_DATA_DIR` > `$CLINE_DIR/data` >
//!     `~/.cline/data/tasks/<taskId>/ui_messages.json`
//!     (`apps/vscode/src/sdk/legacy-state-reader.ts`)
//!   - VS Code-family editors: `<config>/<Product>/User/globalStorage/
//!     saoudrizwan.claude-dev/tasks/<taskId>/ui_messages.json`
//!     (`core/storage/disk.ts` → `globalStorageFsPath/tasks/<id>`)
//!
//! Usage rows in `ui_messages.json` (`shared/ExtensionMessage.ts`,
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
//! Model/provider attribution: `task_metadata.json` `model_usage[]`
//! ({ts, model_id, model_provider_id}) picks the latest entry ≤ row ts;
//! `state/taskHistory.json` `cwdOnTaskInitialization`/`apiProvider` fill
//! project/provider (CLI data dir; editor hosts keep history in state.vscdb
//! which we deliberately don't read — project stays None there).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
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
        Ok(task_dirs()
            .into_iter()
            .map(|(dir, _)| SourceItem {
                key: dir.to_string_lossy().to_string(),
                path: dir,
                kind: SourceKind::Sqlite,
            })
            .collect())
    }

    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
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
        let emit =
            |out: &mut Vec<UsageEvent>, msg: &Value, info: &Value, _i: usize, suffix: &str| {
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
            let info: Value = serde_json::from_str(msgs[p.idx]["text"].as_str().unwrap_or("{}"))
                .unwrap_or_default();
            emit(&mut out.events, &msgs[p.idx], &info, p.idx, ":started");
        }

        state.cold = false;
        let fp = [item.path.join("task_metadata.json"), ui.clone()]
            .into_iter()
            .find(|p| p.is_file());
        if let Some(fp) = fp {
            store.save_cursor(
                self.id(),
                &item.key,
                &fp,
                state.len,
                state.mtime_ms,
                Some(&serde_json::to_string(&state)?),
            )?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let out = Cline.scan_sqlite(&item, &store).unwrap();
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
        let out2 = Cline.scan_sqlite(&item, &store).unwrap();
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
        Cline.scan_sqlite(&item, &store).unwrap();
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
        let out = Cline.scan_sqlite(&item, &store).unwrap();
        let keys: Vec<&str> = out.events.iter().map(|e| e.dedup_key.as_str()).collect();
        assert_eq!(keys, vec!["cline:t-1:1779256800500:started"]);
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
        let out = Cline.scan_sqlite(&item, &store).unwrap();
        assert!(out.events.is_empty());
    }
}
