use super::*;
use crate::Engine;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQ: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "gtt-cb-v2-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(p.join("messages")).unwrap();
        Self(p)
    }
    fn item(&self) -> SourceItem {
        SourceItem {
            key: self.0.display().to_string(),
            path: self.0.clone(),
            kind: SourceKind::Sqlite,
        }
    }
    fn msg(&self, name: &str, request: &str, input: u64, output: u64, cached: u64) -> PathBuf {
        let path = self.0.join("messages").join(format!("{name}.json"));
        let v = json!({"role":"assistant","createdAt":format!("2026-10-17T10:00:{name:0>2}Z"),"extra": json!({"requestId":request,"modelId":"fixture-model", "statsSnapshot":{"inputTokens":input,"outputTokens":output,"cachedInputTokens":cached}}).to_string()});
        std::fs::write(&path, v.to_string()).unwrap();
        path
    }
    fn scan(&self, e: &Engine) -> crate::ScanReport {
        e.scan_items(&CodeBuddyIde::new(), &[self.item()]).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn engine() -> Engine {
    Engine::new(Store::open_memory().unwrap()).unwrap()
}

#[test]
fn late_snapshot_rewrites_same_message_and_replay_is_idempotent() {
    let f = Fixture::new();
    let e = engine();
    let path = f.0.join("messages/01.json");
    std::fs::write(
        &path,
        json!({"role":"assistant","createdAt":"2026-10-17T10:00:01Z"}).to_string(),
    )
    .unwrap();
    assert_eq!(f.scan(&e).events_ingested, 0);
    let before = std::fs::metadata(f.0.join("messages"))
        .unwrap()
        .modified()
        .unwrap();
    f.msg("01", "req-a", 1000, 100, 800);
    assert_eq!(
        before,
        std::fs::metadata(f.0.join("messages"))
            .unwrap()
            .modified()
            .unwrap()
    );
    assert_eq!(f.scan(&e).events_ingested, 1);
    let totals = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (
            totals.events,
            totals.input_tokens,
            totals.output_tokens,
            totals.cache_read_tokens
        ),
        (1, 200, 100, 800)
    );
    assert_eq!(f.scan(&e).events_ingested, 0);
    assert_eq!(f.scan(&e).events_ingested, 0);
}

#[test]
fn different_requests_are_independent_when_counters_fall() {
    let f = Fixture::new();
    let e = engine();
    f.msg("01", "req-a", 1000, 100, 800);
    f.msg("02", "req-b", 600, 50, 400);
    assert_eq!(f.scan(&e).events_ingested, 2);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (
            t.events,
            t.input_tokens,
            t.output_tokens,
            t.cache_read_tokens
        ),
        (2, 400, 150, 1200)
    );
}

#[test]
fn latest_snapshot_of_one_request_wins_and_response_id_does_not_change_identity() {
    let f = Fixture::new();
    let e = engine();
    f.msg("01", "req-a", 1000, 100, 800);
    f.msg("02", "req-a", 1600, 150, 1000);
    f.scan(&e);
    let p = f.msg("02", "req-a", 1600, 150, 1000);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let mut x: Value = serde_json::from_str(v["extra"].as_str().unwrap()).unwrap();
    x["responseId"] = json!("added-after-stream");
    v["extra"] = x;
    std::fs::write(&p, v.to_string()).unwrap();
    f.scan(&e);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!((t.events, t.input_tokens, t.output_tokens), (1, 600, 150));
}

#[test]
fn torn_json_and_incomplete_snapshot_remain_retryable() {
    let f = Fixture::new();
    let e = engine();
    let p = f.0.join("messages/01.json");
    std::fs::write(&p, b"{\"role\":").unwrap();
    assert_eq!(f.scan(&e).events_ingested, 0);
    std::fs::write(&p,json!({"role":"assistant","createdAt":"2026-10-17T10:00:01Z","extra":{"requestId":"a","statsSnapshot":{"inputTokens":100}}}).to_string()).unwrap();
    assert_eq!(f.scan(&e).events_ingested, 0);
    f.msg("01", "a", 100, 20, 0);
    assert_eq!(f.scan(&e).events_ingested, 1);
}

#[test]
fn manual_refresh_detects_equal_size_equal_timestamp_rewrite() {
    let f = Fixture::new();
    let e = engine();
    let p = f.msg("01", "a", 100, 20, 0);
    f.scan(&e);
    let stamp = std::fs::metadata(&p).unwrap().modified().unwrap();
    f.msg("01", "a", 100, 30, 0);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    forget_scan_memo();
    f.scan(&e);
    assert_eq!(
        e.store
            .totals(None, None, None, None)
            .unwrap()
            .output_tokens,
        30
    );
}

#[test]
fn legacy_seen_cursor_is_recovered_without_touching_other_apps_or_orphans() {
    let f = Fixture::new();
    let e = engine();
    let p = f.msg("01", "a", 1000, 100, 800);
    for (key, app, raw) in [
        ("old", apps::CODEBUDDY_IDE, p.display().to_string()),
        ("orphan", apps::CODEBUDDY_IDE, "missing.json".into()),
        ("foreign", apps::CLAUDE, p.display().to_string()),
    ] {
        e.store
            .upsert_event(&UsageEvent {
                dedup_key: key.into(),
                app: app.into(),
                input_tokens: 7,
                raw_ref: Some(raw),
                ..Default::default()
            })
            .unwrap();
    }
    e.store
        .save_cursor(
            apps::CODEBUDDY_IDE,
            &f.item().key,
            &p,
            0,
            0,
            Some(r#"{"seen":["01"],"cum":{"input":99999999}}"#),
        )
        .unwrap();
    assert_eq!(f.scan(&e).events_ingested, 1);
    let rows: i64 = e
        .store
        .conn()
        .query_row(
            "SELECT count(*) FROM usage_events WHERE dedup_key IN ('foreign','orphan')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2);
    assert_eq!(e.store.totals(None, None, None, None).unwrap().events, 3);
    assert_eq!(f.scan(&e).events_ingested, 0);
}

#[test]
fn failed_ingest_rolls_back_legacy_deletion_and_cursor_then_retries() {
    let f = Fixture::new();
    let e = engine();
    let p = f.msg("01", "a", 100, 20, 0);
    e.store
        .upsert_event(&UsageEvent {
            dedup_key: "old".into(),
            app: apps::CODEBUDDY_IDE.into(),
            input_tokens: 7,
            raw_ref: Some(p.display().to_string()),
            ..Default::default()
        })
        .unwrap();
    e.store.conn().execute_batch("CREATE TRIGGER block_cb BEFORE INSERT ON usage_events WHEN NEW.app='codebuddy_ide' BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    let r = f.scan(&e);
    assert_ne!(r.errors, Vec::<String>::new());
    assert_eq!(e.store.load_cursor(&f.item().key).unwrap().state, None);
    assert_eq!(
        e.store.totals(None, None, None, None).unwrap().input_tokens,
        7
    );
    e.store
        .conn()
        .execute_batch("DROP TRIGGER block_cb")
        .unwrap();
    assert_eq!(f.scan(&e).events_ingested, 1);
    assert_eq!(
        e.store.totals(None, None, None, None).unwrap().input_tokens,
        100
    );
}

#[test]
fn authoritative_correction_can_lower_completeness_without_affecting_foreign_row() {
    let f = Fixture::new();
    let e = engine();
    let p = f.msg("01", "a", 1000, 100, 800);
    f.scan(&e);
    f.msg("01", "a", 100, 50, 0);
    f.scan(&e);
    let t = e.store.totals(None, None, None, None).unwrap();
    assert_eq!(
        (
            t.events,
            t.input_tokens,
            t.output_tokens,
            t.cache_read_tokens
        ),
        (1, 100, 50, 0)
    );
    std::fs::remove_file(p).unwrap();
    forget_scan_memo();
    assert_eq!(f.scan(&e).events_ingested, 0);
    assert_eq!(e.store.totals(None, None, None, None).unwrap().events, 1);
}

#[test]
fn rebuilding_codebuddy_rollups_preserves_archived_other_source_totals() {
    let f = Fixture::new();
    let e = engine();
    f.msg("01", "a", 100, 20, 0);
    f.scan(&e);
    e.store
        .conn()
        .execute(
            "INSERT INTO daily_rollups(date,app,input_tokens) VALUES('2026-01-01','claude',999)",
            [],
        )
        .unwrap();
    e.store
        .rebuild_source_rollups("+08:00", apps::CODEBUDDY_IDE)
        .unwrap();
    assert_eq!(
        e.store
            .conn()
            .query_row(
                "SELECT input_tokens FROM daily_rollups WHERE app='claude'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        999
    );
    assert_eq!(
        e.store
            .conn()
            .query_row(
                "SELECT input_tokens FROM daily_rollups WHERE app='codebuddy_ide'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        100
    );
}

struct OnlyConversation<'a>(&'a Fixture);
impl SourceAdapter for OnlyConversation<'_> {
    fn id(&self) -> &'static str {
        apps::CODEBUDDY_IDE
    }
    fn display_name(&self) -> &'static str {
        "fixture"
    }
    fn capability(&self) -> Capability {
        Capability::Estimate
    }
    fn discover(&self) -> Result<Vec<SourceItem>> {
        Ok(vec![self.0.item()])
    }
    fn scan_sqlite(&self, item: &SourceItem, store: &Store) -> Result<ScanOutcome> {
        CodeBuddyIde::new().scan_sqlite(item, store)
    }
}

#[test]
fn failed_source_rollup_retries_even_after_cursor_and_events_commit() {
    let f = Fixture::new();
    let e = engine();
    f.msg("01", "a", 100, 20, 0);
    e.store.conn().execute_batch("CREATE TRIGGER block_rollup BEFORE INSERT ON daily_rollups WHEN NEW.app='codebuddy_ide' BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert!(e.scan_adapter(&OnlyConversation(&f)).is_err());
    assert_eq!(e.store.totals(None, None, None, None).unwrap().events, 1);
    assert_eq!(
        e.store
            .get_state("codebuddy_ide_rollups_pending")
            .unwrap()
            .as_deref(),
        Some("1")
    );
    e.store
        .conn()
        .execute_batch("DROP TRIGGER block_rollup")
        .unwrap();
    assert_eq!(
        e.scan_adapter(&OnlyConversation(&f))
            .unwrap()
            .events_ingested,
        0
    );
    assert_eq!(
        e.store
            .get_state("codebuddy_ide_rollups_pending")
            .unwrap()
            .as_deref(),
        Some("0")
    );
    assert_eq!(
        e.store
            .conn()
            .query_row(
                "SELECT input_tokens FROM daily_rollups WHERE app='codebuddy_ide'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        100
    );
}

#[test]
fn file_backed_migration_creates_verified_backup_before_replacement() {
    let f = Fixture::new();
    let p = f.msg("01", "a", 100, 20, 0);
    let e = Engine::new(Store::open(&f.0.join("ledger.db")).unwrap()).unwrap();
    e.store
        .upsert_event(&UsageEvent {
            dedup_key: "old".into(),
            app: apps::CODEBUDDY_IDE.into(),
            input_tokens: 7,
            raw_ref: Some(p.display().to_string()),
            ..Default::default()
        })
        .unwrap();
    e.scan_adapter(&OnlyConversation(&f)).unwrap();
    let backup = e.store.get_state("codebuddy_v2_backup").unwrap().unwrap();
    let c =
        rusqlite::Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT input_tokens FROM usage_events WHERE dedup_key='old'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    assert_eq!(
        c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    e.scan_adapter(&OnlyConversation(&f)).unwrap();
    assert_eq!(
        e.store.get_state("codebuddy_v2_backup").unwrap().as_deref(),
        Some(backup.as_str())
    );
}
