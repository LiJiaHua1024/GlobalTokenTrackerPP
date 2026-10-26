//! CodeBuddy IDE/Agent request snapshots. Messages are mutable: the Agent
//! writes statsSnapshot after streaming into the same file. Fingerprints track
//! revisions; snapshots aggregate calls within a request, never a conversation.
//! Version-2 cursors recover legacy seen-only state and retain orphaned history.

use super::{Capability, ScanOutcome, SourceAdapter, SourceItem, SourceKind};
use crate::model::{Provenance, UsageEvent, apps};
use crate::normalize::{epoch_ms, num, text};
use crate::store::Store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

// Manual refresh forces a content check, including equal-size rewrites.
static REFRESH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn forget_scan_memo() {
    REFRESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn is_codebuddy(id: &str) -> bool {
    matches!(id, apps::CODEBUDDY_IDE | apps::CODEBUDDY_CLI)
}

/// A separate, durable snapshot precedes the first legacy replacement. This
/// does not prune quota rows or rotate the ordinary daily backup generations.
pub(crate) fn backup_legacy(store: &Store) -> Result<()> {
    let Some(path) = store.path() else {
        return Ok(());
    };
    let legacy: bool = store.conn().query_row(
        "SELECT EXISTS(SELECT 1 FROM usage_events WHERE app=?1 AND dedup_key NOT LIKE 'codebuddy_ide:v2:%')",
        [apps::CODEBUDDY_IDE], |r| r.get(0),
    )?;
    if !legacy {
        return Ok(());
    }
    if store
        .get_state("codebuddy_v2_backup")?
        .is_some_and(|p| Path::new(&p).is_file())
    {
        return Ok(());
    }
    let dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("backups");
    std::fs::create_dir_all(&dir)?;
    let backup = dir.join(format!("codebuddy-before-v2-{}.db", crate::store::now_ms()));
    store
        .conn()
        .execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
    let conn =
        rusqlite::Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    anyhow::ensure!(
        conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))? == "ok",
        "CodeBuddy backup verification failed"
    );
    store.set_state("codebuddy_v2_backup", backup.to_string_lossy().as_ref())?;
    Ok(())
}

pub struct CodeBuddyIde {
    /// `session:`→cwd map is per-scan stable; without this cache the IDE
    /// session db was walked once per conversation (the dominant scan cost).
    cwds: std::sync::Mutex<Option<BTreeMap<String, String>>>,
}

impl CodeBuddyIde {
    pub fn new() -> Self {
        Self {
            cwds: std::sync::Mutex::new(None),
        }
    }
}

impl Default for CodeBuddyIde {
    fn default() -> Self {
        Self::new()
    }
}

/// `…/CodeBuddyExtension/Data` — hosts `Data/<user>/<host>/<acct>/history`.
fn data_root() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| crate::sync::home(".local/share"))
        .join("CodeBuddyExtension/Data")
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// conversationId → cwd, best-effort from the IDE's own session index db(s)
/// (`<config>/<Product>/codebuddy-sessions.vscdb` next to globalStorage).
fn session_dirs() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(cfg) = dirs::config_dir() else {
        return map;
    };
    let Ok(products) = std::fs::read_dir(cfg) else {
        return map;
    };
    for db in products
        .flatten()
        .map(|e| e.path().join("codebuddy-sessions.vscdb"))
    {
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            format!("file:{}?mode=ro", db.to_string_lossy().replace('\\', "/")),
            rusqlite::OpenFlags::SQLITE_OPEN_URI | rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut st) = conn.prepare("SELECT key, value FROM ItemTable") else {
            continue;
        };
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (key, val) = row;
                let Some(id) = key.strip_prefix("session:") else {
                    continue;
                };
                if let Ok(v) = serde_json::from_str::<Value>(&val)
                    && let Some(cwd) = text(&v["cwd"])
                {
                    map.insert(id.to_string(), cwd);
                }
            }
        }
    }
    map
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    modified_ns: u128,
}

impl Stamp {
    fn read(path: &Path) -> Result<Self> {
        let m = std::fs::metadata(path)?;
        Ok(Self {
            size: m.len(),
            modified_ns: m
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos(),
        })
    }
}

pub(super) fn read_json(path: &Path) -> Result<Vec<u8>> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= LIMIT,
        "CodeBuddy record exceeds size limit"
    );
    Ok(bytes)
}

pub(super) fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    request: String,
    model: Option<String>,
    ts: i64,
    input: u64,
    output: u64,
    cached: u64,
    cache_write: u64,
    thinking: u64,
    elapsed_ms: Option<i64>,
    credit: Option<f64>,
}

impl Snapshot {
    fn parse(v: &Value, stem: &str) -> Option<Self> {
        if v["role"].as_str()? != "assistant" {
            return None;
        }
        let extra = match &v["extra"] {
            Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
            Value::Object(_) => v["extra"].clone(),
            _ => return None,
        };
        let s = &extra["statsSnapshot"];
        let input = s["inputTokens"].as_u64()?;
        let output = s["outputTokens"].as_u64()?;
        let credit = crate::normalize::fnum(&s["credit"]).filter(|x| x.is_finite() && *x > 0.0);
        if input == 0 && output == 0 && credit.is_none() {
            return None;
        }
        Some(Self {
            // responseId is often added later; requestId remains stable.
            request: text(&extra["requestId"])
                .or_else(|| text(&extra["responseId"]))
                .unwrap_or_else(|| format!("message:{stem}")),
            model: text(&extra["modelId"]).or_else(|| text(&extra["modelName"])),
            ts: epoch_ms(&v["createdAt"])?,
            input,
            output,
            cached: num(&s["cachedInputTokens"]),
            cache_write: num(&s["cacheWriteTokens"]),
            thinking: num(&s["thinkingTokens"]),
            elapsed_ms: s["elapsedMs"].as_i64().filter(|n| *n > 0),
            credit,
        })
    }

    fn event(&self, conv: &str, project: Option<String>, path: &Path) -> UsageEvent {
        UsageEvent {
            dedup_key: format!("codebuddy_ide:v2:{conv}:{}", self.request),
            app: apps::CODEBUDDY_IDE.into(),
            session_id: Some(conv.into()),
            project,
            model: self.model.clone(),
            ts_start: Some(self.ts),
            input_tokens: crate::normalize::input_excludes_cache(
                self.input,
                self.cached,
                self.cache_write,
            ),
            output_tokens: self.output,
            reasoning_tokens: self.thinking,
            cache_read_tokens: self.cached,
            cache_write_5m_tokens: self.cache_write,
            credits: self.credit,
            duration_ms: self.elapsed_ms,
            provenance: Provenance::LocalJsonl,
            raw_ref: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileState {
    stamp: Stamp,
    hash: String,
    snapshot: Option<Snapshot>,
}

#[derive(Default, Serialize, Deserialize)]
struct ConvState {
    version: u8,
    generation: u64,
    files: BTreeMap<String, FileState>,
    emitted: BTreeMap<String, String>,
}

impl SourceAdapter for CodeBuddyIde {
    fn id(&self) -> &'static str {
        apps::CODEBUDDY_IDE
    }
    fn display_name(&self) -> &'static str {
        "CodeBuddy"
    }
    // Each event aggregates the internal calls of one Agent request.
    fn capability(&self) -> Capability {
        Capability::Estimate
    }
    fn watch_roots(&self) -> Vec<PathBuf> {
        vec![
            data_root()
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(data_root),
        ]
    }
    fn discover(&self) -> Result<Vec<SourceItem>> {
        let mut out = Vec::new();
        for user in subdirs(&data_root()) {
            for host in subdirs(&user) {
                for acct in subdirs(&host) {
                    for ws in subdirs(&acct.join("history")) {
                        for conv in subdirs(&ws) {
                            if conv.join("messages").is_dir() {
                                out.push(SourceItem {
                                    key: conv.to_string_lossy().into_owned(),
                                    path: conv,
                                    kind: SourceKind::Sqlite,
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }
    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        self.scan_conversation(item, store)
    }
}

impl CodeBuddyIde {
    fn scan_conversation(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        let cur = store.load_cursor(&item.key)?;
        let mut state = cur
            .state
            .as_deref()
            .and_then(|s| serde_json::from_str::<ConvState>(s).ok())
            .filter(|s| s.version == 2)
            .unwrap_or_default();
        let generation = REFRESH.load(std::sync::atomic::Ordering::Relaxed);
        let force = state.version != 2 || state.generation != generation;
        let conv = item.path.file_name().unwrap_or_default().to_string_lossy();
        let project = {
            let mut g = self
                .cwds
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            g.get_or_insert_with(session_dirs)
                .get(conv.as_ref())
                .and_then(|cwd| {
                    Path::new(cwd)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                })
        };
        let mut changed = force;
        let mut fingerprint_file = None;
        for entry in std::fs::read_dir(item.path.join("messages"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let stamp = Stamp::read(&path)?;
            let old = state.files.get(&stem);
            if !force && old.is_some_and(|f| f.stamp == stamp) {
                continue;
            }
            let bytes = read_json(&path)?;
            let hash = fingerprint(&bytes);
            if let Some(old) = old
                && old.hash == hash
            {
                state.files.get_mut(&stem).unwrap().stamp = stamp;
                changed = true;
                continue;
            }
            // An in-flight/torn JSON is retried, never committed as "seen".
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let snapshot =
                Snapshot::parse(&value, &stem).or_else(|| old.and_then(|f| f.snapshot.clone()));
            if snapshot.is_some() {
                // Retire only legacy rows whose original snapshot is readable.
                // Missing/pruned source files and unrelated apps remain untouched.
                store.conn().execute(
                    "DELETE FROM usage_events WHERE app=?1 AND replace(raw_ref, char(92), '/')=?2
                     AND dedup_key NOT LIKE 'codebuddy_ide:v2:%'",
                    rusqlite::params![
                        apps::CODEBUDDY_IDE,
                        path.to_string_lossy().replace('\\', "/")
                    ],
                )?;
            }
            fingerprint_file = Some(path);
            state.files.insert(
                stem,
                FileState {
                    stamp,
                    hash,
                    snapshot,
                },
            );
            changed = true;
        }
        let mut requests: BTreeMap<&str, (&str, &FileState, &Snapshot)> = BTreeMap::new();
        for (stem, file) in &state.files {
            let Some(snap) = &file.snapshot else {
                continue;
            };
            let candidate = (snap.ts, file.stamp.modified_ns, stem.as_str());
            if requests
                .get(snap.request.as_str())
                .is_none_or(|(old_stem, old, old_snap)| {
                    candidate > (old_snap.ts, old.stamp.modified_ns, *old_stem)
                })
            {
                requests.insert(&snap.request, (stem, file, snap));
            }
        }
        let mut out = ScanOutcome::default();
        for (request, (stem, _, snap)) in &requests {
            let digest = fingerprint(&serde_json::to_vec(snap)?);
            if state.emitted.get(*request) == Some(&digest) {
                continue;
            }
            out.events.push(snap.event(
                &conv,
                project.clone(),
                &item.path.join("messages").join(format!("{stem}.json")),
            ));
            state.emitted.insert((*request).into(), digest);
            changed = true;
        }
        let retired: Vec<String> = state
            .emitted
            .keys()
            .filter(|id| !requests.contains_key(id.as_str()))
            .cloned()
            .collect();
        for id in retired {
            store.conn().execute(
                "DELETE FROM usage_events WHERE app=?1 AND dedup_key=?2",
                rusqlite::params![apps::CODEBUDDY_IDE, format!("codebuddy_ide:v2:{conv}:{id}")],
            )?;
            state.emitted.remove(&id);
        }
        if changed {
            state.version = 2;
            state.generation = generation;
            let fp = fingerprint_file.or_else(|| {
                std::fs::read_dir(item.path.join("messages"))
                    .ok()?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.is_file())
            });
            if let Some(fp) = fp {
                store.save_cursor(
                    self.id(),
                    &item.key,
                    &fp,
                    0,
                    0,
                    Some(&serde_json::to_string(&state)?),
                )?;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
