//! DeepSeek Harness adapter (`deepseek-ai/deepseek-harness` CLI + DSH Desktop).
//!
//! Harness homes (packages/util/home-paths): `$DSH_HOME` > `~/.dsh`;
//! DSH Desktop points the home at `<userData>/dsh-desktop/harness`
//! (`dirs::data_dir()` on Windows/macOS, `dirs::config_dir()` on Linux);
//! `~/.dsh_desktop/<name>/` holds extra deployment homes. Under each home:
//!
//!   sessions/--<normalized-cwd>--/<encoded-session-id>/session.v<N>.jsonl.zstd
//!
//! (`.jsonl` when compression is disabled; generations v0…v4 coexist — only
//! the highest is read to avoid counting one session's usage several times.)
//!
//! Usage arrives as `assistant/message` events carrying `data.usage`
//! `{inputTokens,outputTokens,cacheReadTokens,cacheWriteTokens,…}` plus
//! `data.message.source.{provider,model}` (request/header config is the
//! fallback attribution). `step/start` supplies the call-start timestamp.
//!
//! Zstd frames cannot be tail-decoded, so the byte segment from the engine is
//! only used to advance the cursor: the whole file is re-read and re-decoded
//! on every change, and `last_seq` in adapter_state limits what is emitted
//! (dedup_key still guarantees idempotency if state is lost).

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{derived_duration, epoch_ms, num, num_opt, text};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct Dsh;

/// All harness homes that may hold `sessions/`, deduplicated.
fn homes() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("DSH_HOME")
        && !dir.trim().is_empty()
    {
        out.push(PathBuf::from(dir));
    }
    out.push(crate::sync::home(".dsh"));
    // DSH Desktop userData root (Windows %APPDATA%, macOS Application
    // Support) and the Linux config-dir variant.
    for base in [dirs::data_dir(), dirs::config_dir()].into_iter().flatten() {
        out.push(base.join("dsh-desktop").join("harness"));
    }
    // Additional per-deployment homes live under ~/.dsh_desktop/<name>/.
    if let Ok(rd) = std::fs::read_dir(crate::sync::home(".dsh_desktop")) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn session_roots() -> Vec<PathBuf> {
    homes().into_iter().map(|h| h.join("sessions")).collect()
}

/// Generation number of a canonical `session[.v<N>].jsonl[.zstd]` name.
/// `session.jsonl` is v0 (the first release carried no tag). Returns `None`
/// for any other file.
fn generation(name: &str) -> Option<u32> {
    let stem = name
        .strip_suffix(".jsonl.zstd")
        .or_else(|| name.strip_suffix(".jsonl"))?;
    if stem == "session" {
        return Some(0);
    }
    stem.strip_prefix("session.v")?.parse().ok()
}

/// Enumerate the current-generation session log under each `sessions/` root:
/// per session directory only the highest `session[.v<N>]` file is kept, so a
/// session's retained historical generations are never counted twice.
fn discover_in(roots: &[PathBuf]) -> Vec<SourceItem> {
    let mut by_dir: HashMap<PathBuf, (u32, PathBuf)> = HashMap::new();
    for root in roots {
        for p in crate::sync::collect_files(root, "zstd", 3)
            .into_iter()
            .chain(crate::sync::collect_files(root, "jsonl", 3))
        {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(ver) = generation(name) else {
                continue;
            };
            let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
            by_dir
                .entry(dir)
                .and_modify(|(g, old)| {
                    // Highest generation wins; a `.zstd` beats a stray
                    // plaintext twin of the same generation.
                    if ver > *g || (ver == *g && p.extension().is_some_and(|e| e == "zstd")) {
                        *g = ver;
                        *old = p.clone();
                    }
                })
                .or_insert((ver, p));
        }
    }
    let mut items: Vec<SourceItem> = by_dir
        .into_values()
        .map(|(_, path)| SourceItem {
            key: path.to_string_lossy().to_string(),
            path,
            kind: SourceKind::Jsonl,
        })
        .collect();
    items.sort_by(|a, b| a.key.cmp(&b.key));
    items
}

/// Decode a whole session file; `zstd` tails torn mid-frame keep whatever
/// decoded cleanly instead of failing the file.
fn decode(path: &Path) -> Result<(Vec<u8>, bool)> {
    let raw = std::fs::read(path)?;
    if path.extension().is_none_or(|e| e != "zstd") {
        return Ok((raw, false));
    }
    let mut dec = zstd::stream::read::Decoder::new(raw.as_slice())?;
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    let torn = loop {
        match dec.read(&mut buf) {
            Ok(0) => break false,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => break true,
        }
    };
    Ok((out, torn))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DshState {
    /// Highest `seq` already emitted — the whole log is re-parsed on every
    /// append, so this keeps rescan output to genuinely new rows.
    #[serde(default)]
    last_seq: u64,
}

impl SourceAdapter for Dsh {
    fn id(&self) -> &'static str {
        apps::DSH
    }
    fn display_name(&self) -> &'static str {
        "DeepSeek Harness"
    }
    /// Provider-reported per-call usage — exact.
    fn capability(&self) -> Capability {
        Capability::Precise
    }

    fn watch_roots(&self) -> Vec<PathBuf> {
        session_roots()
    }

    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(discover_in(&session_roots()))
    }

    fn parse_jsonl(
        &self,
        item: &SourceItem,
        _from: u64,
        data: &[u8],
        prior_state: Option<&str>,
    ) -> Result<ScanOutcome> {
        let (text_bytes, torn) = decode(&item.path)?;
        let mut out = ScanOutcome {
            // The engine's segment is acknowledged wholesale so the cursor
            // lands at EOF; parsing itself re-reads the file from byte 0.
            consumed: data.len() as u64,
            ..Default::default()
        };
        if torn {
            out.notes
                .push("zstd stream ends mid-frame — decoded the complete prefix".into());
        }
        let mut state: DshState = prior_state
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let mut session_id: Option<String> = None;
        let mut project: Option<String> = None;
        // Fallback attribution when message.source lacks provider/model.
        let mut last_provider: Option<String> = None;
        let mut last_model: Option<String> = None;
        // (turn, step) → start ms, for call duration.
        let mut step_start: HashMap<(u64, u64), i64> = HashMap::new();
        let mut max_seq = state.last_seq;

        for (idx, raw) in text_bytes.split(|&b| b == b'\n').enumerate() {
            let line = raw.strip_suffix(b"\r").unwrap_or(raw);
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let seq = num_opt(&v["seq"]).unwrap_or(u64::MAX);
            match v.get("type").and_then(Value::as_str) {
                Some("session") => {
                    session_id = text(&v["id"]);
                    project = text(&v["cwd"]);
                }
                Some("request/header") => {
                    let cfg = &v["data"]["header"]["config"];
                    if let Some(p) = text(&cfg["provider"]) {
                        last_provider = Some(p);
                    }
                    if let Some(m) = text(&cfg["model"]) {
                        last_model = Some(m);
                    }
                }
                Some("step/start") => {
                    let d = &v["data"];
                    if let (Some(turn), Some(step), Some(t)) = (
                        num_opt(&d["turn"]),
                        num_opt(&d["step"]),
                        epoch_ms(&v["time"]),
                    ) {
                        step_start.entry((turn, step)).or_insert(t);
                    }
                }
                Some("assistant/message") => {
                    if seq != u64::MAX && seq <= state.last_seq {
                        continue;
                    }
                    let d = &v["data"];
                    let u = &d["usage"];
                    if !u.is_object() {
                        continue;
                    }
                    let input = num(&u["inputTokens"]);
                    let output = num(&u["outputTokens"]);
                    let reasoning =
                        num(&u["reasoningTokens"]).max(num(&u["reasoningOutputTokens"]));
                    let cache_read = num(&u["cacheReadTokens"]);
                    let cache_write = num(&u["cacheWriteTokens"]);
                    if input + output + cache_read + cache_write == 0 {
                        out.skipped += 1;
                        continue;
                    }
                    let msg = &d["message"];
                    let src = &msg["source"];
                    let ts_end = epoch_ms(&v["time"]);
                    let ts_start = num_opt(&d["turn"])
                        .zip(num_opt(&d["step"]))
                        .and_then(|(t, s)| step_start.get(&(t, s)).copied());
                    let sid = session_id
                        .clone()
                        .or_else(|| {
                            item.path
                                .parent()
                                .and_then(|p| p.file_name())
                                .map(|s| s.to_string_lossy().to_string())
                        })
                        .unwrap_or_else(|| "unknown".into());
                    let key_part = if seq != u64::MAX {
                        seq.to_string()
                    } else {
                        format!("m{}", text(&msg["id"]).unwrap_or_else(|| format!("L{idx}")))
                    };
                    out.events.push(UsageEvent {
                        dedup_key: format!("dsh:{sid}:{key_part}"),
                        app: apps::DSH.into(),
                        session_id: Some(sid),
                        project: project.clone(),
                        provider_id: text(&src["provider"]).or_else(|| last_provider.clone()),
                        model: text(&src["model"]).or_else(|| last_model.clone()),
                        request_model: last_model.clone(),
                        ts_start: ts_start.or(ts_end),
                        ts_end,
                        input_tokens: input,
                        output_tokens: output,
                        reasoning_tokens: reasoning, // subset of output
                        cache_read_tokens: cache_read,
                        cache_write_5m_tokens: cache_write,
                        provenance: Provenance::LocalJsonl,
                        duration_ms: derived_duration(ts_start, ts_end),
                        raw_ref: Some(format!("{}#{}", item.path.display(), key_part)),
                        ..Default::default()
                    });
                    if seq != u64::MAX {
                        max_seq = max_seq.max(seq);
                    }
                }
                _ => {}
            }
        }

        state.last_seq = max_seq;
        out.new_state = Some(serde_json::to_string(&state)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TmpDir(PathBuf);
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `sessions/<wd>/<sid>/` under a fresh root; `tag` keeps parallel tests apart.
    fn session_dir(tag: &str, sid: &str) -> (TmpDir, PathBuf) {
        let root = std::env::temp_dir().join(format!("gtt-dsh-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions").join("--tmp-work--").join(sid);
        std::fs::create_dir_all(&dir).unwrap();
        (TmpDir(root), dir)
    }

    fn item(path: &Path) -> SourceItem {
        SourceItem {
            key: path.to_string_lossy().to_string(),
            path: path.to_path_buf(),
            kind: SourceKind::Jsonl,
        }
    }

    fn zstd_write(path: &Path, text: &str) {
        let f = std::fs::File::create(path).unwrap();
        let mut e = zstd::stream::write::Encoder::new(f, 0).unwrap();
        e.write_all(text.as_bytes()).unwrap();
        e.finish().unwrap();
    }

    /// Session header line for `sid` (epoch-ms timestamps — `epoch_ms`
    /// auto-scales, so tests must use realistic 1e12 values).
    fn header(sid: &str) -> String {
        format!(
            "{{\"type\":\"session\",\"version\":4,\"id\":\"{sid}\",\"createdAt\":1790693338495,\"cwd\":\"D:/proj/demo\",\"isSeeded\":false,\"delegationDepth\":0}}"
        )
    }

    const T0: i64 = 1_790_695_060_000;

    #[test]
    fn v4_zstd_message_emits_usage() {
        let (_r, dir) = session_dir("v4", "session-aaa");
        let file = dir.join("session.v4.jsonl.zstd");
        zstd_write(
            &file,
            &format!(
                "{}\n\
                 {{\"type\":\"step/start\",\"seq\":6,\"time\":{},\"data\":{{\"turn\":1,\"step\":1}}}}\n\
                 {{\"type\":\"request/header\",\"seq\":7,\"time\":{},\"data\":{{\"header\":{{\"config\":{{\"provider\":\"deepseek-official\",\"model\":\"deepseek-flash\"}}}}}}}}\n\
                 {{\"type\":\"assistant/message\",\"seq\":8,\"time\":{},\"data\":{{\"turn\":1,\"step\":1,\"message\":{{\"id\":\"m1\",\"source\":{{\"kind\":\"model\",\"provider\":\"deepseek-official\",\"model\":\"deepseek-flash\"}}}},\"usage\":{{\"inputTokens\":100,\"outputTokens\":40,\"cacheReadTokens\":7,\"cacheWriteTokens\":3,\"totalTokens\":150}}}}}}\n",
                header("session-aaa"),
                T0,
                T0 + 100,
                T0 + 2000
            ),
        );
        let data = std::fs::read(&file).unwrap();
        let out = Dsh.parse_jsonl(&item(&file), 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!(e.dedup_key, "dsh:session-aaa:8");
        assert_eq!(e.session_id.as_deref(), Some("session-aaa"));
        assert_eq!(e.project.as_deref(), Some("D:/proj/demo"));
        assert_eq!(e.provider_id.as_deref(), Some("deepseek-official"));
        assert_eq!(e.model.as_deref(), Some("deepseek-flash"));
        assert_eq!(e.input_tokens, 100);
        assert_eq!(e.output_tokens, 40);
        assert_eq!(e.cache_read_tokens, 7);
        assert_eq!(e.cache_write_5m_tokens, 3);
        assert_eq!(e.duration_ms, Some(2000));
        assert_eq!(e.ts_start, Some(T0));
        assert_eq!(e.ts_end, Some(T0 + 2000));
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn rescan_emits_only_new_seqs() {
        let (_r, dir) = session_dir("delta", "session-bbb");
        let file = dir.join("session.v4.jsonl.zstd");
        let head = format!(
            "{}\n\
             {{\"type\":\"step/start\",\"seq\":1,\"time\":{},\"data\":{{\"turn\":1,\"step\":1}}}}\n\
             {{\"type\":\"assistant/message\",\"seq\":2,\"time\":{},\"data\":{{\"turn\":1,\"step\":1,\"message\":{{\"id\":\"m1\"}},\"usage\":{{\"inputTokens\":1,\"outputTokens\":1}}}}}}\n",
            header("session-bbb"),
            T0,
            T0 + 100
        );
        zstd_write(&file, &head);
        let it = item(&file);
        let data = std::fs::read(&file).unwrap();
        let out = Dsh.parse_jsonl(&it, 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        let state = out.new_state.unwrap();

        // File grows: append a second step+message, re-read from scratch.
        let head = format!(
            "{head}{{\"type\":\"step/start\",\"seq\":3,\"time\":{},\"data\":{{\"turn\":1,\"step\":2}}}}\n\
             {{\"type\":\"assistant/message\",\"seq\":4,\"time\":{},\"data\":{{\"turn\":1,\"step\":2,\"message\":{{\"id\":\"m2\"}},\"usage\":{{\"inputTokens\":5,\"outputTokens\":6}}}}}}\n",
            T0 + 200,
            T0 + 300
        );
        zstd_write(&file, &head);
        let data = std::fs::read(&file).unwrap();
        let out2 = Dsh
            .parse_jsonl(&it, 500, &data[..50.min(data.len())], Some(&state))
            .unwrap();
        assert_eq!(out2.events.len(), 1);
        assert_eq!(out2.events[0].dedup_key, "dsh:session-bbb:4");
        assert_eq!(out2.events[0].input_tokens, 5);
        // Cursor acknowledges only the segment the engine handed us.
        assert_eq!(out2.consumed, 50.min(data.len()) as u64);
    }

    #[test]
    fn header_fallback_and_missing_seq_still_emit() {
        let (_r, dir) = session_dir("fb", "session-ccc");
        let file = dir.join("session.v4.jsonl"); // compression: none
        std::fs::write(
            &file,
            format!(
                "{}\n\
                 {{\"type\":\"request/header\",\"seq\":1,\"time\":{},\"data\":{{\"header\":{{\"config\":{{\"provider\":\"custom\",\"model\":\"m-x\"}}}}}}}}\n\
                 {{\"type\":\"assistant/message\",\"time\":{},\"data\":{{\"turn\":1,\"step\":1,\"message\":{{\"id\":\"mid9\",\"source\":{{\"kind\":\"model\"}}}},\"usage\":{{\"inputTokens\":3,\"outputTokens\":2}}}}}}\n",
                header("session-ccc"),
                T0,
                T0 + 100
            ),
        )
        .unwrap();
        let data = std::fs::read(&file).unwrap();
        let out = Dsh.parse_jsonl(&item(&file), 0, &data, None).unwrap();
        let e = &out.events[0];
        assert_eq!(e.model.as_deref(), Some("m-x"));
        assert_eq!(e.provider_id.as_deref(), Some("custom"));
        assert_eq!(e.dedup_key, "dsh:session-ccc:mmid9");
    }

    #[test]
    fn discover_picks_only_the_highest_generation() {
        let (root, dir) = session_dir("ver", "session-ddd");
        std::fs::write(dir.join("session.jsonl"), "{}\n").unwrap();
        std::fs::write(dir.join("session.v3.jsonl.zstd"), b"x").unwrap();
        zstd_write(&dir.join("session.v4.jsonl.zstd"), "{}\n");
        let found = discover_in(&[root.0.join("sessions")]);
        let mine: Vec<_> = found.iter().filter(|i| i.path.starts_with(&dir)).collect();
        assert_eq!(mine.len(), 1);
        assert!(mine[0].path.ends_with("session.v4.jsonl.zstd"));
    }

    /// Compress `text` into one frame and return the bytes (no file I/O).
    fn zstd_bytes(text: &str) -> Vec<u8> {
        zstd::stream::encode_all(text.as_bytes(), 0).unwrap()
    }

    #[test]
    fn torn_zstd_tail_decodes_the_prefix() {
        let (_r, dir) = session_dir("torn", "session-eee");
        let file = dir.join("session.v4.jsonl.zstd");
        // Realistic tear: appended chunks are separate frames — a crash or an
        // in-flight write leaves the LAST frame truncated while earlier frames
        // still decode.
        let mut bytes = zstd_bytes(&format!(
            "{}\n\
             {{\"type\":\"assistant/message\",\"seq\":2,\"time\":{},\"data\":{{\"message\":{{\"id\":\"m\"}},\"usage\":{{\"inputTokens\":9,\"outputTokens\":9}}}}}}\n",
            header("session-eee"),
            T0
        ));
        let mut tail = zstd_bytes(
            "{\"type\":\"assistant/message\",\"seq\":3,\"data\":{\"usage\":{\"inputTokens\":1}}}\n",
        );
        tail.truncate(tail.len() - 4);
        bytes.extend_from_slice(&tail);
        std::fs::write(&file, &bytes).unwrap();
        let data = std::fs::read(&file).unwrap();
        let out = Dsh.parse_jsonl(&item(&file), 0, &data, None).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].dedup_key, "dsh:session-eee:2");
        assert!(!out.notes.is_empty());
        assert_eq!(out.consumed, data.len() as u64);
    }

    #[test]
    fn zero_usage_is_skipped_not_counted() {
        let (_r, dir) = session_dir("zero", "session-fff");
        let file = dir.join("session.v4.jsonl.zstd");
        zstd_write(
            &file,
            &format!(
                "{}\n\
                 {{\"type\":\"assistant/message\",\"seq\":2,\"time\":{},\"data\":{{\"message\":{{\"id\":\"m\"}},\"usage\":{{\"inputTokens\":0,\"outputTokens\":0}}}}}}\n",
                header("session-fff"),
                T0
            ),
        );
        let data = std::fs::read(&file).unwrap();
        let out = Dsh.parse_jsonl(&item(&file), 0, &data, None).unwrap();
        assert!(out.events.is_empty());
        assert_eq!(out.skipped, 1);
    }
}
