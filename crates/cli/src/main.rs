//! globaltokentracker-cli — dev harness & power-user entry.
//! `scan` | `report [today|week|month|all]` | `reconcile` | `sources`

use anyhow::Result;
use clap::{Parser, Subcommand};
use globaltokentracker_core::{Engine, Store, store::default_db_path};
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(name = "globaltokentracker", version, about = "Unified AI-coding usage ledger")]
struct Cli {
    /// Ledger DB path (default ~/.globaltokentracker/ledger.db).
    #[arg(long, global = true)]
    db: Option<std::path::PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Incremental scan of all sources (or one: claude|codex|opencode|zcode).
    Scan { source: Option<String> },
    /// Aggregate report.
    Report {
        #[arg(default_value = "today")]
        span: String,
    },
    /// Cross-check ledger numbers against cc-switch's DB.
    Reconcile {
        /// cc-switch db path override.
        #[arg(long)]
        ccswitch: Option<std::path::PathBuf>,
    },
    /// List discovered source files per adapter.
    Sources,
    /// Rebuild daily_rollups aggregates from raw events (idempotent).
    Rollup,
    /// Export detail rows to CSV (`all|today|week|month`).
    Export {
        #[arg(default_value = "all")]
        span: String,
        /// Output file; stdout when omitted.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Run the OTLP/HTTP+JSON receiver standalone (127.0.0.1:4318).
    Otel,
    /// Merge OTEL_* env into ~/.claude/settings.json (enables Claude telemetry).
    OtelSetup,
    /// Poll vendor quota channels (codex wham / cursor RPC) once.
    Quota,
    /// Price book: show freshness; --update pulls models.dev + LiteLLM and
    /// reprices events still marked unpriced.
    Prices {
        #[arg(long)]
        update: bool,
    },
    /// Rebuild rollups, then drop raw events older than --keep-days.
    Prune {
        /// Detail retention in days.
        #[arg(long, default_value_t = 90)]
        keep_days: i64,
        /// Reclaim file space afterwards.
        #[arg(long)]
        vacuum: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let db = cli.db.unwrap_or_else(default_db_path);
    let store = Store::open(&db)?;
    let engine = Engine::new(store)?;

    match cli.cmd {
        Cmd::Scan { source } => {
            let t = std::time::Instant::now();
            let r = match source.as_deref() {
                Some(id) => engine.scan_source(id)?,
                None => engine.scan_once()?,
            };
            println!(
                "files: {} seen / {} scanned / {} pinned | events: +{} (merged {}, skipped {}) | quotas: +{} | {:?}",
                r.files_seen,
                r.files_scanned,
                r.files_pinned,
                r.events_ingested,
                r.events_merged,
                r.events_skipped,
                r.quotas,
                t.elapsed()
            );
            for e in &r.errors {
                eprintln!("  ERR {e}");
            }
        }
        Cmd::Report { span } => report(&engine, &span)?,
        Cmd::Reconcile { ccswitch } => reconcile(&engine, ccswitch)?,
        Cmd::Sources => {
            println!("adapters: {}", engine.adapter_ids().join(", "));
            println!(
                "events in ledger: {}",
                engine.store.event_count(None, None)?
            );
        }
        Cmd::Rollup => {
            let off = local_offset();
            let n = engine.store.rebuild_rollups(&off)?;
            println!("daily_rollups: {n} rows rebuilt (local offset {off})");
        }
        Cmd::Export { span, out } => export(&engine, &span, out.as_deref())?,
        Cmd::Otel => globaltokentracker_core::otel::serve(&db)?,
        Cmd::OtelSetup => {
            let p = globaltokentracker_core::otel::install_claude_env()?;
            println!("OTEL env merged into {}", p.display());
        }
        Cmd::Prices { update } => {
            use globaltokentracker_core::pricing;
            let last = pricing::last_live_sync(&engine.store)?;
            match last {
                Some(t) => {
                    let age_h = (globaltokentracker_core::store::now_ms() - t) / 3_600_000;
                    println!("live price book: synced {age_h}h ago (stale>{})", pricing::PRICE_TTL_SECS / 3600);
                }
                None => println!("live price book: never synced (seed fallback active)"),
            }
            if update {
                let r = pricing::refresh(&engine.store)?;
                println!(
                    "synced: models.dev {} rows, litellm {} rows; repriced {} unpriced events",
                    r.models_dev, r.litellm, r.repriced
                );
            }
        }
        Cmd::Quota => {
            for o in globaltokentracker_core::quota::poll_all() {
                match o.error {
                    Some(e) => eprintln!("{}: ERR {e}", o.app),
                    None => {
                        for q in &o.quotas {
                            engine.store.insert_quota(q)?;
                            println!(
                                "{} {:<16} used={:?} limit={:?} pct={:?} reset={:?}",
                                o.app, q.window_kind, q.used, q.limit_value, q.used_percent, q.resets_at
                            );
                        }
                    }
                }
            }
        }
        Cmd::Prune { keep_days, vacuum } => {
            let n = engine.store.rebuild_rollups(&local_offset())?;
            println!("daily_rollups: {n} rows rebuilt before prune");
            let cutoff = jiff::Zoned::now()
                .checked_sub(jiff::SignedDuration::from_hours(keep_days * 24))
                .unwrap()
                .timestamp()
                .as_millisecond();
            let d = engine.store.prune_events(cutoff)?;
            println!("pruned {d} raw events older than {keep_days}d (rollups preserved)");
            if vacuum {
                engine.store.vacuum()?;
                println!("vacuumed");
            }
        }
    }
    Ok(())
}

fn csv_cell(out: &mut String, s: &str) {
    if s.contains([',', '"', '\n']) {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn export(engine: &Engine, span: &str, out: Option<&std::path::Path>) -> Result<()> {
    let (f, t) = span_ms(span);
    let rows = engine.store.export_rows(f, t)?;
    let mut buf = String::from(
        "ts,app,model,pricing_model,project,session,input,output,reasoning,cache_read,cache_write,credits,cost_usd,cost_source,duration_ms,raw_ref\n",
    );
    for r in &rows {
        let ts = jiff::Timestamp::from_millisecond(r.ts_start.unwrap_or(0))?
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        buf.push_str(&ts);
        for cell in [
            &r.app,
            r.model.as_deref().unwrap_or(""),
            r.pricing_model.as_deref().unwrap_or(""),
            r.project.as_deref().unwrap_or(""),
            r.session_id.as_deref().unwrap_or(""),
        ] {
            buf.push(',');
            csv_cell(&mut buf, cell);
        }
        buf.push_str(&format!(
            ",{},{},{},{},{},{},{},{},{},",
            r.input_tokens,
            r.output_tokens,
            r.reasoning_tokens,
            r.cache_read_tokens,
            r.cache_write_tokens,
            r.credits.map(|v| format!("{v:.4}")).unwrap_or_default(),
            r.cost_usd.map(|v| format!("{v:.6}")).unwrap_or_default(),
            r.cost_source.as_deref().unwrap_or(""),
            r.duration_ms.map(|v| v.to_string()).unwrap_or_default()
        ));
        csv_cell(&mut buf, r.raw_ref.as_deref().unwrap_or(""));
        buf.push('\n');
    }
    match out {
        Some(p) => {
            std::fs::write(p, &buf)?;
            println!("exported {} rows -> {}", rows.len(), p.display());
        }
        None => print!("{buf}"),
    }
    Ok(())
}

fn span_ms(span: &str) -> (Option<i64>, Option<i64>) {
    // Local-midnight boundaries via jiff; today = [00:00 local, now].
    let now = jiff::Zoned::now();
    let start_of_today = now.start_of_day().unwrap().timestamp().as_millisecond();
    match span {
        "today" => (Some(start_of_today), None),
        "week" => (
            Some(
                now.start_of_day()
                    .unwrap()
                    .checked_sub(jiff::SignedDuration::from_hours(6 * 24))
                    .unwrap()
                    .timestamp()
                    .as_millisecond(),
            ),
            None,
        ),
        "month" => (
            Some(
                now.start_of_day()
                    .unwrap()
                    .checked_sub(jiff::SignedDuration::from_hours(29 * 24))
                    .unwrap()
                    .timestamp()
                    .as_millisecond(),
            ),
            None,
        ),
        _ => (None, None),
    }
}

fn local_offset() -> String {
    globaltokentracker_core::viewmodel::local_utc_offset()
}

fn fmt_tok(n: u64) -> String {
    globaltokentracker_core::viewmodel::fmt::tokens_exact(n)
}

fn report(engine: &Engine, span: &str) -> Result<()> {
    let (f, t) = span_ms(span);
    let tot = engine.store.totals(f, t, None, None)?;
    println!(
        "\n== {span} ==  events {} | in {} out {} reasoning {} cacheR {} cacheW {} | credits {:.2} | est. ${:.2}\n",
        tot.events,
        fmt_tok(tot.input_tokens),
        fmt_tok(tot.output_tokens),
        fmt_tok(tot.reasoning_tokens),
        fmt_tok(tot.cache_read_tokens),
        fmt_tok(tot.cache_write_tokens),
        tot.credits,
        tot.cost_usd
    );

    #[derive(Tabled)]
    struct Row {
        #[tabled(rename = "app")]
        app: String,
        #[tabled(rename = "events")]
        events: String,
        #[tabled(rename = "input")]
        input: String,
        #[tabled(rename = "output")]
        output: String,
        #[tabled(rename = "cache_read")]
        cr: String,
        #[tabled(rename = "credits")]
        credits: String,
        #[tabled(rename = "est_$")]
        cost: String,
    }
    let rows: Vec<Row> = engine
        .store
        .by_app(f, t, None, None)?
        .into_iter()
        .map(|a| Row {
            app: a.app,
            events: a.events.to_string(),
            input: fmt_tok(a.input_tokens),
            output: fmt_tok(a.output_tokens),
            cr: fmt_tok(a.cache_read_tokens),
            credits: format!("{:.1}", a.credits),
            cost: format!("{:.2}", a.cost_usd),
        })
        .collect();
    if !rows.is_empty() {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

fn reconcile(engine: &Engine, ccs: Option<std::path::PathBuf>) -> Result<()> {
    let path = ccs.unwrap_or_else(|| globaltokentracker_core::sync::home(".cc-switch/cc-switch.db"));
    let conn = rusqlite::Connection::open_with_flags(
        format!("file:{}?mode=ro", path.to_string_lossy().replace('\\', "/")),
        rusqlite::OpenFlags::SQLITE_OPEN_URI | rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    // cc-switch claude numbers live in proxy_request_logs (spec §6.11);
    // costs are TEXT there, cast to REAL.
    let (cc_ev, cc_in, cc_out2, cc_cr, cc_cost): (i64, i64, i64, i64, f64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(CAST(total_cost_usd AS REAL)),0)
         FROM proxy_request_logs WHERE app_type='claude'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap_or((0, 0, 0, 0, 0.0));
    let ours = engine.store.reconcile_summary("claude")?;
    println!(
        "ours:     ev={} in={} out={} cacheR={} ${:.2}",
        ours.events, ours.input_tokens, ours.output_tokens, ours.cache_read_tokens, ours.cost_usd
    );
    println!("ccswitch: ev={cc_ev} in={cc_in} out={cc_out2} cacheR={cc_cr} ${cc_cost:.2}");
    let pct = |a: f64, b: f64| {
        if b == 0.0 {
            f64::NAN
        } else {
            (a - b).abs() / b * 100.0
        }
    };
    println!(
        "ΔcacheR = {:.2}%  Δout = {:.2}%  Δcost = {:.2}%  (gate <1%)",
        pct(ours.cache_read_tokens as f64, cc_cr as f64),
        pct(ours.output_tokens as f64, cc_out2 as f64),
        pct(ours.cost_usd, cc_cost)
    );

    // ── Codex gate (spec §M0): session watermarks vs state_5.threads.
    // Verified on-device: threads.tokens_used == the rollout file's LAST
    // total_token_usage.total_tokens (347/347 match, 0 miss); total_tokens
    // == input+output (cached ⊂ input, reasoning ⊂ output). Each file's last
    // cumulative line is already persisted in sync_cursors.adapter_state.cum.
    let s5 = globaltokentracker_core::sync::home(".codex/state_5.sqlite");
    if s5.exists() {
        let conn2 = rusqlite::Connection::open_with_flags(
            format!("file:{}?mode=ro", s5.to_string_lossy().replace('\\', "/")),
            rusqlite::OpenFlags::SQLITE_OPEN_URI | rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        // Per-file watermarks: last cum.input + cum.output per rollout path.
        let mut ours: std::collections::HashMap<String, i64> = Default::default();
        for (path, blob) in engine.store.adapter_states("codex")? {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&blob) {
                let cum = &v["cum"];
                let t = cum["input"].as_i64().unwrap_or(0) + cum["output"].as_i64().unwrap_or(0);
                ours.insert(norm_path(&path), t);
            }
        }
        // state_5 threads keyed by rollout_path — join to compare like-for-like.
        let mut st = conn2.prepare(
            "SELECT rollout_path, tokens_used FROM threads WHERE tokens_used > 0",
        )?;
        let theirs: Vec<(String, i64)> = st
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut j_ours = 0i64;
        let mut j_theirs = 0i64;
        let mut joined = 0u64;
        for (rp, tu) in &theirs {
            if let Some(o) = ours.get(&norm_path(rp)) {
                j_ours += o;
                j_theirs += tu;
                joined += 1;
            }
        }
        println!(
            "\ncodex watermark: joined {} threads — ours={} state_5={}",
            joined, j_ours, j_theirs
        );
        println!("Δ = {:.2}%  (gate <0.5%)", pct(j_ours as f64, j_theirs as f64));
        println!(
            "coverage: {} files with state vs {} threads>0 (stale files outside join: {})",
            ours.len(),
            theirs.len(),
            ours.len().saturating_sub(joined as usize)
        );
    }
    Ok(())
}

/// Case/slash-insensitive absolute path key for cross-db joins.
fn norm_path(p: &str) -> String {
    let path = std::path::Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        globaltokentracker_core::sync::home(p)
    };
    abs.to_string_lossy().replace('/', "\\").to_lowercase()
}
