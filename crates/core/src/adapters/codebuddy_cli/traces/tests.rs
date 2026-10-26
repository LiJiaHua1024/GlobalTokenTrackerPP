use super::*;
use crate::Engine;
use crate::adapters::{SourceKind, codebuddy_cli::CodeBuddyCli};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "gtt-cb-cli-{}-{}.json",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        Self(p)
    }
    fn item(&self) -> SourceItem {
        SourceItem {
            key: self.0.display().to_string(),
            path: self.0.clone(),
            kind: SourceKind::Sqlite,
        }
    }
    fn write(&self, v: &Value) {
        std::fs::write(&self.0, v.to_string()).unwrap();
    }
    fn scan(&self, e: &Engine) -> crate::ScanReport {
        e.scan_items(&CodeBuddyCli, &[self.item()]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn trace() -> Value {
    json!({"trace":{"traceId":"trace-a","startedAt":"2026-10-11T08:03:47.248Z","endedAt":"2026-10-11T08:05:45.221Z","duration":117973,"status":"ok","totalTokens":1200,"modelInfo":{"models":["glm-5.3"],"totalInputTokens":1000,"totalOutputTokens":200,"totalCachedTokens":800,"lastCallInputTokens":99999,"callCount":3}},"spans":[{"inputTokens":99999}]})
}
fn engine() -> Engine {
    Engine::new(Store::open_memory().unwrap()).unwrap()
}

#[test]
fn exact_transcript_arriving_later_replaces_the_overlapping_trace() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"]["sessionId"] = json!("session-a");
    f.write(&v);
    f.scan(&e);
    e.store
        .upsert_event(&UsageEvent {
            dedup_key: "codebuddy_cli:precise-request".into(),
            app: apps::CODEBUDDY_CLI.into(),
            session_id: Some("session-a".into()),
            input_tokens: 200,
            output_tokens: 200,
            cache_read_tokens: 800,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(f.scan(&e).events_ingested, 0);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (
            t.events,
            t.input_tokens,
            t.output_tokens,
            t.cache_read_tokens
        ),
        (1, 200, 200, 800)
    );
    assert_eq!(f.scan(&e).events_ingested, 0);
}

#[test]
fn unknown_totals_survive_queries_cube_repricing_and_archive() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"]["modelInfo"] = json!({"models":["gpt-5"]});
    f.write(&v);
    f.scan(&e);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (t.input_tokens, t.output_tokens, t.unclassified_tokens),
        (0, 0, 1200)
    );
    assert_eq!(
        e.store.export_rows(None, None).unwrap()[0].unclassified_tokens,
        1200
    );
    let cube = crate::Cube::build(&e.store).unwrap();
    let p = cube
        .overview_parts(crate::viewmodel::Range::All, None, None)
        .unwrap();
    assert_eq!(p.all.unclassified_tokens, 1200);
    assert_eq!(p.by_model[0].tokens, 1200);
    e.store
        .rebuild_source_rollups("+00:00", apps::CODEBUDDY_CLI)
        .unwrap();
    assert_eq!(
        e.store
            .conn()
            .query_row("SELECT unclassified_tokens FROM daily_rollups", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1200
    );
    let book = crate::pricing::PriceBook::load(&e.store).unwrap();
    crate::pricing::reprice_unpriced(&e.store, &book).unwrap();
    assert_eq!(
        e.store.export_rows(None, None).unwrap()[0]
            .cost_source
            .as_deref(),
        Some("unpriced")
    );
    let archive = f.0.with_extension("archive.db");
    e.store.archive_events(i64::MAX, &archive).unwrap();
    let archived = rusqlite::Connection::open(&archive).unwrap();
    assert_eq!(
        archived
            .query_row("SELECT unclassified_tokens FROM usage_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1200
    );
    drop(archived);
    std::fs::remove_file(archive).unwrap();
}
#[test]
fn aggregate_trace_ignores_spans_and_last_call_counters_and_is_idempotent() {
    let f = Fixture::new();
    let e = engine();
    f.write(&trace());
    assert_eq!(f.scan(&e).events_ingested, 1);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (
            t.events,
            t.input_tokens,
            t.output_tokens,
            t.cache_read_tokens
        ),
        (1, 200, 200, 800)
    );
    assert_eq!(f.scan(&e).events_ingested, 0);
}
#[test]
fn duplicate_trace_files_share_identity() {
    let a = Fixture::new();
    let b = Fixture::new();
    let e = engine();
    a.write(&trace());
    b.write(&trace());
    a.scan(&e);
    b.scan(&e);
    assert_eq!(e.store.totals(None, None, None, None).unwrap().events, 1);
}
#[test]
fn missing_dimensions_keep_reported_total_unclassified() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"].as_object_mut().unwrap().remove("modelInfo");
    f.write(&v);
    f.scan(&e);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (t.input_tokens, t.output_tokens, t.unclassified_tokens),
        (0, 0, 1200)
    );
}
#[test]
fn inconsistent_dimensions_do_not_invent_token_splits() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"]["modelInfo"]["totalInputTokens"] = json!(9999);
    f.write(&v);
    f.scan(&e);
    assert_eq!(
        e.store
            .totals(None, None, None, None)
            .unwrap()
            .unclassified_tokens,
        1200
    );
}
#[test]
fn in_flight_and_zero_error_traces_do_not_create_usage() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"].as_object_mut().unwrap().remove("endedAt");
    f.write(&v);
    assert_eq!(f.scan(&e).events_ingested, 0);
    v["trace"]["endedAt"] = json!("2026-10-11T08:05:45.221Z");
    v["trace"]["totalTokens"] = json!(0);
    v["trace"]["status"] = json!("error");
    f.write(&v);
    assert_eq!(f.scan(&e).events_ingested, 0);
}
#[test]
fn paid_failed_trace_is_retained_and_multiple_models_are_not_guessed() {
    let f = Fixture::new();
    let e = engine();
    let mut v = trace();
    v["trace"]["status"] = json!("error");
    v["trace"]["modelInfo"]["models"] = json!(["a", "b"]);
    f.write(&v);
    f.scan(&e);
    let rows = e.store.export_rows(None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model, None);
}
#[test]
fn write_failure_does_not_advance_cursor() {
    let f = Fixture::new();
    let e = engine();
    f.write(&trace());
    e.store.conn().execute_batch("CREATE TRIGGER block_cli BEFORE INSERT ON usage_events BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert_ne!(f.scan(&e).errors, Vec::<String>::new());
    assert_eq!(e.store.load_cursor(&f.item().key).unwrap().state, None);
    e.store
        .conn()
        .execute_batch("DROP TRIGGER block_cli")
        .unwrap();
    assert_eq!(f.scan(&e).events_ingested, 1);
}
