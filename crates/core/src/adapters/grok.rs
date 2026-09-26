//! Grok CLI adapter (spec §6.5): `~/.grok/sessions/<urlenc-cwd>/<uuid>/updates.jsonl`.
//!
//! `session/update` and `_x.ai/session/update` lines with
//! `update.sessionUpdate == "turn_completed"` carry per-turn usage in
//! camelCase — incl. `costUsdTicks` (nano-USD, provider-reported) and
//! `apiDurationMs`. `usage.modelUsage` is a per-model map; one event is
//! emitted per (prompt_id, model) so multi-model turns don't conflate.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind, complete_lines};
use crate::model::{CostSource, Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, input_excludes_cache, num, text};
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// costUsdTicks are fixed-point nano-dollars (1e9 ticks = $1).
const TICKS_PER_USD: f64 = 1e9;

pub struct Grok;

impl SourceAdapter for Grok {
    fn id(&self) -> &'static str {
        apps::GROK
    }
    fn display_name(&self) -> &'static str {
        "Grok CLI"
    }
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![crate::sync::home(".grok/sessions")]
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        let root = crate::sync::home(".grok/sessions");
        Ok(crate::sync::collect_files(&root, "jsonl", 3)
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "updates.jsonl"))
            .map(|p| SourceItem {
                key: p.to_string_lossy().to_string(),
                path: p,
                kind: SourceKind::Jsonl,
            })
            .collect())
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        from: u64,
        data: &[u8],
        _prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (seg, consumed) = complete_lines(data);
        let mut out = ScanOutcome {
            consumed,
            ..Default::default()
        };
        let project = pct_decode(
            &item
                .path
                .parent()
                .and_then(Path::parent)
                .and_then(|d| d.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        let dir_session = item
            .path
            .parent()
            .and_then(|d| d.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut pos = 0u64;
        for line in seg.split(|&b| b == b'\n') {
            let line_start = from + pos;
            pos += line.len() as u64 + 1;
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            const NEEDLE: &[u8] = b"session/update\"";
            if line.is_empty() || !line.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let method = v.get("method").and_then(Value::as_str).unwrap_or("");
            if method != "session/update" && method != "_x.ai/session/update" {
                continue;
            }
            let params = &v["params"];
            let upd = &params["update"];
            if upd.get("sessionUpdate").and_then(Value::as_str) != Some("turn_completed") {
                continue;
            }
            let usage = &upd["usage"];
            if !usage.is_object() {
                continue;
            }
            let session_id = text(&params["sessionId"]).unwrap_or_else(|| dir_session.clone());
            let prompt_id = text(&upd["prompt_id"]).unwrap_or_else(|| line_start.to_string());
            let ts = epoch_ms(&v["timestamp"]);
            let elapsed = upd["elapsed_ms"].as_i64();

            let mut emit = |model: Option<String>, u: &Value| {
                let input = num(&u["inputTokens"]);
                let cached = num(&u["cachedReadTokens"]);
                let cw = num(&u["cacheCreationTokens"]);
                let ticks = u["costUsdTicks"].as_f64();
                let key = match &model {
                    Some(m) => format!("grok:{session_id}:{prompt_id}:{m}"),
                    None => format!("grok:{session_id}:{prompt_id}"),
                };
                out.events.push(UsageEvent {
                    dedup_key: key,
                    app: apps::GROK.into(),
                    session_id: Some(session_id.clone()),
                    project: (!project.is_empty()).then(|| project.clone()),
                    model: model.clone(),
                    request_model: model,
                    ts_start: ts,
                    ts_end: match (ts, elapsed) {
                        (Some(t0), Some(d)) => Some(t0 + d),
                        _ => None,
                    },
                    input_tokens: input_excludes_cache(input, cached, cw),
                    output_tokens: num(&u["outputTokens"]),
                    reasoning_tokens: num(&u["reasoningTokens"]),
                    cache_read_tokens: cached,
                    cache_write_5m_tokens: cw,
                    cost_usd: ticks.map(|t| t / TICKS_PER_USD),
                    cost_source: ticks.map(|_| CostSource::ProviderReported),
                    provenance: Provenance::LocalJsonl,
                    duration_ms: elapsed,
                    active_ms: u["apiDurationMs"].as_i64(),
                    raw_ref: Some(format!("{}@{}", item.path.display(), line_start)),
                    ..Default::default()
                });
            };

            let mu = &usage["modelUsage"];
            if let Some(map) = mu.as_object().filter(|m| !m.is_empty()) {
                for (model, u) in map {
                    emit(Some(model.clone()), u);
                }
            } else {
                emit(None, usage);
            }
        }
        Ok(out)
    }
}

/// Minimal `%XX` decoder for the urlencoded cwd path segment (no url dep).
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
