//! Vendor quota pollers (spec §6.9) — subscription windows + credits, kept
//! separate from price-book USD estimates. Blocking HTTP via `ureq`; callers
//! must run this off the UI thread. Every channel is independently fallible:
//! one vendor's breakage never masks the others (`PollOutcome.error`).

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

use crate::model::QuotaSnapshot;
use crate::store::now_ms;
use crate::sync::home;

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_REFRESH_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CURSOR_USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
/// tokcat-verified: refresh only when auth.json's last_refresh is >8d old.
const REFRESH_STALE_SECS: i64 = 8 * 24 * 3600;

pub struct PollOutcome {
    pub app: &'static str,
    pub quotas: Vec<QuotaSnapshot>,
    pub error: Option<String>,
}

/// Poll every channel with local credentials; absent creds → silent skip.
pub fn poll_all() -> Vec<PollOutcome> {
    let mut out = Vec::new();
    if home(".codex/auth.json").exists() {
        out.push(outcome("codex", poll_codex()));
    }
    if cursor_db().is_some() {
        out.push(outcome("cursor", poll_cursor()));
    }
    out
}

fn outcome(app: &'static str, r: Result<Vec<QuotaSnapshot>>) -> PollOutcome {
    match r {
        Ok(quotas) => PollOutcome {
            app,
            quotas,
            error: None,
        },
        Err(e) => PollOutcome {
            app,
            quotas: vec![],
            error: Some(format!("{e:#}")),
        },
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent()
}

fn snap(app: &str, kind: &str, used_pct: Option<f64>, resets_at: Option<i64>, raw: &str) -> QuotaSnapshot {
    QuotaSnapshot {
        app: app.into(),
        account: None,
        captured_at: now_ms(),
        window_kind: kind.into(),
        used: None,
        limit_value: None,
        used_percent: used_pct,
        resets_at,
        raw_json: Some(raw.into()),
    }
}

// ── Codex (ChatGPT subscription, wham) ────────────────────────────────────

fn codex_auth(path: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(path)?).context("auth.json parse")
}

/// Refresh access_token via refresh_token and write auth.json back
/// (tokcat-verified flow; preserves every other key in the file).
fn codex_refresh(auth: &mut Value, path: &Path) -> Result<()> {
    let refresh = auth["tokens"]["refresh_token"]
        .as_str()
        .context("auth.json has no refresh_token")?;
    let mut resp = agent()
        .post(CODEX_REFRESH_URL)
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({
            "client_id": CODEX_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "scope": "openid profile email",
        }))
        .map_err(|e| anyhow::anyhow!("codex refresh: {e}"))?;
    if resp.status() != 200 {
        bail!("codex refresh status {}", resp.status());
    }
    let body: Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
    for k in ["access_token", "refresh_token", "id_token"] {
        if let Some(v) = body[k].as_str() {
            auth["tokens"][k] = serde_json::json!(v);
        }
    }
    auth["last_refresh"] = serde_json::json!(jiff::Timestamp::now().to_string());
    std::fs::write(path, serde_json::to_string_pretty(auth)?)?;
    Ok(())
}

fn codex_stale(auth: &Value) -> bool {
    let stale = auth["last_refresh"].as_str().and_then(|s| {
        s.parse::<jiff::Timestamp>()
            .ok()
            .map(|t| (jiff::Timestamp::now() - t).get_seconds() > REFRESH_STALE_SECS)
    });
    stale.unwrap_or(true)
}

fn wham_usage(agent: &ureq::Agent, token: &str, account: Option<&str>) -> Result<String> {
    let mut req = agent
        .get(CODEX_USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "GlobalTokenTracker");
    if let Some(id) = account.filter(|s| !s.is_empty()) {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let mut resp = req.call().map_err(|e| anyhow::anyhow!("wham usage: {e}"))?;
    Ok(resp.body_mut().read_to_string()?)
}

fn poll_codex() -> Result<Vec<QuotaSnapshot>> {
    let path = home(".codex/auth.json");
    let mut auth = codex_auth(&path)?;
    if codex_stale(&auth) {
        codex_refresh(&mut auth, &path)?;
    }
    let token = auth["tokens"]["access_token"]
        .as_str()
        .context("auth.json has no access_token")?
        .to_string();
    let account = auth["tokens"]["account_id"].as_str().map(String::from);
    let agent = agent();
    let body = match wham_usage(&agent, &token, account.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            // One refresh+retry on auth failure, then give up.
            if format!("{e}").contains("401") || format!("{e}").contains("403") {
                codex_refresh(&mut auth, &path)?;
                let t2 = auth["tokens"]["access_token"].as_str().unwrap_or(&token);
                wham_usage(&agent, t2, account.as_deref())?
            } else {
                return Err(e);
            }
        }
    };
    let v: Value = serde_json::from_str(&body).context("wham response parse")?;
    let mut out = Vec::new();
    let mut window = |kind: &str, w: &Value| {
        if let Some(pct) = w["used_percent"].as_f64() {
            let resets = w["reset_at"].as_i64().map(|s| s * 1000);
            out.push(snap("codex", kind, Some(pct), resets, &w.to_string()));
        }
    };
    // ~5h session window vs weekly window — told apart by limit_window_seconds.
    for (key, fallback) in [("primary_window", "5h_block"), ("secondary_window", "weekly")] {
        let w = &v["rate_limit"][key];
        let secs = w["limit_window_seconds"].as_i64().unwrap_or(0);
        let kind = if secs >= 6 * 24 * 3600 { "weekly" } else { fallback };
        window(kind, w);
    }
    for extra in v["additional_rate_limits"].as_array().into_iter().flatten() {
        let label = extra["metered_feature"]
            .as_str()
            .or_else(|| extra["limit_name"].as_str())
            .unwrap_or("extra");
        let rl = &extra["rate_limit"];
        let w = if rl["primary_window"].is_object() {
            &rl["primary_window"]
        } else {
            &rl["secondary_window"]
        };
        window(label, w);
    }
    if let Some(bal) = v["credits"]["balance"].as_f64() {
        // Semantics: `used` carries remaining BALANCE for credit windows.
        let mut q = snap("codex", "credits", None, None, &body);
        q.used = Some(bal);
        out.push(q);
    }
    if out.is_empty() {
        bail!("wham response carried no rate-limit windows");
    }
    Ok(out)
}

// ── Cursor (Connect RPC over JSON) ────────────────────────────────────────

/// state.vscdb path — %APPDATA% on Windows, XDG config on Linux,
/// Application Support on macOS (the mac port gets it for free).
fn cursor_db() -> Option<std::path::PathBuf> {
    let rel = "Cursor/User/globalStorage/state.vscdb";
    [dirs::config_dir(), dirs::data_dir()]
        .into_iter()
        .flatten()
        .map(|b| b.join(rel))
        .find(|p| p.exists())
}

fn cursor_token() -> Result<String> {
    let db_path = cursor_db().context("cursor state.vscdb not found")?;
    let conn = rusqlite::Connection::open_with_flags(
        format!("file:{}?mode=ro", db_path.to_string_lossy().replace('\\', "/")),
        rusqlite::OpenFlags::SQLITE_OPEN_URI | rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let tok: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken'",
            [],
            |r| r.get(0),
        )
        .context("cursorAuth/accessToken absent — not signed in")?;
    Ok(tok)
}

/// Connect encodes int64 as JSON strings — accept both shapes.
fn lenient_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn poll_cursor() -> Result<Vec<QuotaSnapshot>> {
    let token = cursor_token()?;
    let mut resp = agent()
        .post(CURSOR_USAGE_URL)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Connect-Protocol-Version", "1")
        .header("Accept", "application/json")
        .header("User-Agent", "GlobalTokenTracker")
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({}))
        .map_err(|e| anyhow::anyhow!("cursor usage: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        bail!("cursor session expired — sign in again in Cursor");
    }
    if !(200..300).contains(&status) {
        bail!("cursor usage status {status}");
    }
    let body = resp.body_mut().read_to_string()?;
    let v: Value = serde_json::from_str(&body).context("cursor response parse")?;
    let resets = v["billingCycleEnd"]
        .as_str()
        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        .map(|t| t.as_millisecond());
    let plan = &v["planUsage"];
    let mut out = Vec::new();
    if let Some(pct) = lenient_f64(&plan["totalPercentUsed"]) {
        let mut q = snap("cursor", "billing_period", Some(pct), resets, &body);
        // Money fields are cents — normalize to USD for display.
        q.used = lenient_f64(&plan["used"]).map(|c| c / 100.0);
        q.limit_value = lenient_f64(&plan["limit"]).map(|c| c / 100.0);
        out.push(q);
    }
    for (key, kind) in [("autoPercentUsed", "auto_pool"), ("apiPercentUsed", "api_pool")] {
        if let Some(pct) = lenient_f64(&plan[key]) {
            out.push(snap("cursor", kind, Some(pct), resets, &body));
        }
    }
    if out.is_empty() {
        bail!("cursor response carried no plan usage");
    }
    Ok(out)
}
