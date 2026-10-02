//! OTLP/HTTP+JSON receiver on 127.0.0.1:4318 (spec §③).
//!
//! Minimal HTTP/1.1 over `std::net` — no tokio/axum. Claude Code exports
//! cumulative `claude_code.*` metrics; each data point upserts one
//! `otel_metrics` row keyed (metric, session_id, attr_sig) so re-exports of
//! the same cumulative series overwrite in place instead of double-counting.
//! These rows stay OUT of `usage_events`: official cost/active_time figures
//! are shown alongside (never merged with) price-book estimates.

use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::store::{Store, now_ms};

pub const DEFAULT_ADDR: &str = "127.0.0.1:4318";
const MAX_HEADER: usize = 8 << 10;
const MAX_BODY: usize = 8 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Spawn the receiver on a dedicated OS thread (it blocks forever — must NOT
/// go through reactor's task pool). Returns `None` if the port is taken or
/// `GTT_NO_OTEL` is set; the app then runs file-based sources only.
pub fn spawn(db: PathBuf) -> Option<std::thread::JoinHandle<()>> {
    if std::env::var_os("GTT_NO_OTEL").is_some() {
        return None;
    }
    let listener = match TcpListener::bind(DEFAULT_ADDR) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("otel: bind {DEFAULT_ADDR} failed ({e}) — receiver off");
            return None;
        }
    };
    tracing::info!("otel: OTLP/HTTP+JSON receiver on http://{DEFAULT_ADDR}/v1/metrics");
    Some(std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let db = db.clone();
            std::thread::spawn(move || {
                let _ = handle(stream, &db);
            });
        }
    }))
}

/// Standalone (CLI `globaltokentracker otel`) blocking variant.
pub fn serve(db: &Path) -> Result<()> {
    let listener =
        TcpListener::bind(DEFAULT_ADDR).with_context(|| format!("otel: bind {DEFAULT_ADDR}"))?;
    tracing::info!("otel: serving on http://{DEFAULT_ADDR}/v1/metrics");
    for stream in listener.incoming().flatten() {
        let _ = handle(stream, db);
    }
    Ok(())
}

fn handle(stream: TcpStream, db: &Path) -> Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = String::new();
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            anyhow::bail!("eof");
        }
        head.push_str(&line);
        if head.len() > MAX_HEADER {
            anyhow::bail!("header too large");
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut lines = head.lines();
    let req = lines.next().unwrap_or("");
    let mut parts = req.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let len: usize = lines
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        })
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    let len = len.min(MAX_BODY);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;

    let (status, out) = if method == "POST" && path == "/v1/metrics" {
        match ingest_metrics(&body, db) {
            Ok(n) => (200, format!(r#"{{"partialSuccess":{{}},"accepted":{n}}}"#)),
            Err(e) => {
                tracing::warn!("otel: bad request: {e}");
                (400, r#"{"error":"bad otlp payload"}"#.to_string())
            }
        }
    } else {
        (404, r#"{"error":"not found"}"#.to_string())
    };
    let mut s = stream;
    write!(
        s,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len(),
        reason = if status == 200 { "OK" } else { "ERR" }
    )?;
    s.flush()?;
    Ok(())
}

/// Walk OTLP/JSON resourceMetrics→scopeMetrics→metrics→(sum|gauge).dataPoints.
/// Returns data points upserted into `otel_metrics`.
fn ingest_metrics(body: &[u8], db: &Path) -> Result<usize> {
    let v: Value = serde_json::from_slice(body).context("otlp json")?;
    let store = Store::open(db)?;
    let mut n = 0usize;
    for rm in v["resourceMetrics"].as_array().into_iter().flatten() {
        for sm in rm["scopeMetrics"].as_array().into_iter().flatten() {
            for m in sm["metrics"].as_array().into_iter().flatten() {
                let Some(name) = m["name"].as_str() else {
                    continue;
                };
                let points = m["sum"]["dataPoints"]
                    .as_array()
                    .or_else(|| m["gauge"]["dataPoints"].as_array());
                for dp in points.into_iter().flatten() {
                    if upsert_point(&store, name, dp)? {
                        n += 1;
                    }
                }
            }
        }
    }
    Ok(n)
}

fn upsert_point(store: &Store, metric: &str, dp: &Value) -> Result<bool> {
    let attrs = dp["attributes"].as_array();
    let mut session = String::new();
    let mut sig_parts: Vec<String> = Vec::new();
    if let Some(attrs) = attrs {
        for a in attrs {
            let (Some(k), Some(v)) = (a["key"].as_str(), attr_value(&a["value"])) else {
                continue;
            };
            if k == "session.id" || k == "session_id" {
                session = v;
            } else {
                sig_parts.push(format!("{k}={v}"));
            }
        }
    }
    sig_parts.sort();
    // A point carrying neither asDouble nor asInt would land as 0.0 and drag
    // cumulative totals down — skip it instead of writing a fake zero.
    let Some(value) = dp["asDouble"]
        .as_f64()
        .or_else(|| dp["asInt"].as_str().and_then(|s| s.parse().ok()))
        .or_else(|| dp["asInt"].as_i64().map(|i| i as f64))
    else {
        return Ok(false);
    };
    let ts_ms = dp["timeUnixNano"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .map(|ns| ns / 1_000_000)
        .unwrap_or_else(now_ms);
    let attrs_json = dp["attributes"].to_string();
    store.upsert_otel_metric(
        metric,
        &session,
        &sig_parts.join(","),
        value,
        ts_ms,
        &attrs_json,
    )
}

fn attr_value(v: &Value) -> Option<String> {
    v["stringValue"]
        .as_str()
        .map(String::from)
        .or_else(|| v["intValue"].as_str().map(String::from))
        .or_else(|| v["intValue"].as_i64().map(|i| i.to_string()))
        .or_else(|| v["doubleValue"].as_f64().map(|f| f.to_string()))
        .or_else(|| v["boolValue"].as_bool().map(|b| b.to_string()))
}

/// Merge OTEL_* vars into `~/.claude/settings.json`'s `env` block (spec M2
/// bootstrap). Existing keys are preserved — cc-switch co-manages this file
/// (spec risk #3) and `preserve_order` keeps its key layout intact.
pub fn install_claude_env() -> Result<PathBuf> {
    let path = crate::sync::home(".claude/settings.json");
    let mut doc: Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?).context("settings.json parse")?
    } else {
        serde_json::json!({})
    };
    if !doc.is_object() {
        anyhow::bail!("settings.json root is not an object");
    }
    let env = doc
        .as_object_mut()
        .unwrap()
        .entry("env")
        .or_insert_with(|| serde_json::json!({}));
    if !env.is_object() {
        anyhow::bail!("settings.json env block is not an object");
    }
    let env = env.as_object_mut().unwrap();
    for (k, v) in [
        ("OTEL_METRICS_EXPORTER", serde_json::json!("otlp")),
        (
            "OTEL_EXPORTER_OTLP_PROTOCOL",
            serde_json::json!("http/json"),
        ),
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            serde_json::json!(format!("http://{DEFAULT_ADDR}")),
        ),
        ("OTEL_METRIC_EXPORT_INTERVAL", serde_json::json!("60000")),
    ] {
        env.insert(k.into(), v);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::store::atomic_write(&path, (serde_json::to_string_pretty(&doc)? + "\n").as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[
      {"name":"claude_code.cost.usage","sum":{"dataPoints":[
        {"attributes":[{"key":"session.id","value":{"stringValue":"s1"}},
                       {"key":"model","value":{"stringValue":"opus-5"}}],
         "asDouble":1.5,"timeUnixNano":"1736983800000000000"}]}},
      {"name":"claude_code.active_time.total","gauge":{"dataPoints":[
        {"attributes":[{"key":"session.id","value":{"stringValue":"s1"}}],
         "asInt":"120","timeUnixNano":"1736983800000000000"}]}}
    ]}]}]}"#;

    #[test]
    fn cumulative_points_upsert_in_place() {
        let dir = std::env::temp_dir().join(format!("cl-otel-test-{}", now_ms()));
        let db = dir.join("t.db");
        // Same point pushed twice with a higher value → one row, latest wins.
        let _ = ingest_metrics(PAYLOAD.as_bytes(), &db).unwrap();
        let _ = ingest_metrics(PAYLOAD.replace("1.5", "2.75").as_bytes(), &db).unwrap();
        let s = Store::open(&db).unwrap();
        let (rows, val): (i64, f64) = s
            .conn()
            .query_row(
                "SELECT COUNT(*), SUM(value) FROM otel_metrics WHERE metric='claude_code.cost.usage'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, val), (1, 2.75));
        let active: f64 = s
            .conn()
            .query_row(
                "SELECT value FROM otel_metrics WHERE metric='claude_code.active_time.total'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 120.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn valueless_points_are_skipped() {
        let dir = std::env::temp_dir().join(format!("cl-otel-novalue-{}", now_ms()));
        let db = dir.join("t.db");
        // Neither asDouble nor asInt on the point → not written, not counted.
        let n = ingest_metrics(
            br#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[
              {"name":"claude_code.cost.usage","sum":{"dataPoints":[
                {"attributes":[],"timeUnixNano":"1736983800000000000"}]}}]}]}]}"#,
            &db,
        )
        .unwrap();
        assert_eq!(n, 0);
        let s = Store::open(&db).unwrap();
        let rows: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM otel_metrics", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
