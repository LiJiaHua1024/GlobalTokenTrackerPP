//! Unified data model — the single landing point for every tool/channel (spec §5).
//! Timestamps are epoch milliseconds (i64); `jiff` is only used at format boundaries.

use serde::{Deserialize, Serialize};

/// Known tool identifiers. Stored as TEXT; keep the constants for adapters/UI.
pub mod apps {
    pub const CLAUDE: &str = "claude";
    pub const CODEX: &str = "codex";
    pub const OPENCODE: &str = "opencode";
    pub const ZCODE: &str = "zcode";
    pub const GROK: &str = "grok";
    pub const WORKBUDDY: &str = "workbuddy";
    pub const CODEBUDDY_CLI: &str = "codebuddy_cli";
    pub const CODEBUDDY_IDE: &str = "codebuddy_ide";
    pub const QODER: &str = "qoder";
    pub const CURSOR: &str = "cursor";
    pub const GEMINI: &str = "gemini";
    pub const GEMINI_ANTIGRAVITY: &str = "gemini_antigravity";
    pub const COPILOT: &str = "copilot";
    pub const DEVIN: &str = "devin";
    pub const WINDSURF: &str = "windsurf";
    pub const MINIMAX_CODE: &str = "minimax_code";
    pub const KIMI_CODE: &str = "kimi_code";
}

/// Where the raw record came from (spec §5 `provenance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Provenance {
    #[default]
    LocalJsonl,
    LocalSqlite,
    Otel,
    VendorApi,
    Dashboard,
    CcswitchDb,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalJsonl => "local_jsonl",
            Self::LocalSqlite => "local_sqlite",
            Self::Otel => "otel",
            Self::VendorApi => "vendor_api",
            Self::Dashboard => "dashboard",
            Self::CcswitchDb => "ccswitch_db",
        }
    }
}

/// How `cost_usd` was produced (spec §5 `cost_source`).
/// API-billed dollars and subscription credits/percentages are never mixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CostSource {
    /// Vendor-reported authoritative figure (e.g. OTel cost metric).
    Official,
    /// Cost reported by the tool's own backend/session record (e.g. OpenCode `session.cost`).
    ProviderReported,
    /// Computed from the price book against normalized tokens.
    Computed,
    /// Computed under uncertainty (e.g. model was "auto"/aliased).
    Estimated,
    /// No price found anywhere — counted as 0 but flagged; never guessed.
    Unpriced,
}

impl CostSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::ProviderReported => "provider_reported",
            Self::Computed => "computed",
            Self::Estimated => "estimated",
            Self::Unpriced => "unpriced",
        }
    }
}

/// One normalized usage record. All token fields are non-negative counts;
/// `input_tokens` always EXCLUDES cache read/write (spec §7.1 `excludes_cache`).
#[derive(Debug, Clone, Default)]
pub struct UsageEvent {
    /// Stable dedup key — format is adapter-defined but must be collision-free
    /// across rescan/replay (e.g. `claude:{file}:{msg_id}`).
    pub dedup_key: String,
    pub app: String,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub account_id: Option<String>,
    pub provider_id: Option<String>,
    /// Raw model name as reported by the tool.
    pub model: Option<String>,
    /// Client-side requested alias (e.g. `gpt-reserve`), kept separate for audit.
    pub request_model: Option<String>,
    /// Resolved pricing key after alias normalization; NULL = unpriced.
    pub pricing_model: Option<String>,
    pub ts_start: Option<i64>,
    pub ts_end: Option<i64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_5m_tokens: u64,
    pub cache_write_1h_tokens: u64,
    /// Credit-based tools (Qoder/WorkBuddy) — never converted to dollars.
    pub credits: Option<f64>,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<CostSource>,
    pub provenance: Provenance,
    pub duration_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub active_ms: Option<i64>,
    pub status: Option<String>,
    pub error: Option<String>,
    /// Source file path + byte offset / row pointer for drill-down audit.
    pub raw_ref: Option<String>,
}

impl UsageEvent {
    /// Billing-dimension completeness used by the UPSERT conflict rule:
    /// a terminal streaming record replaces an interim snapshot only when it
    /// carries at least as much information (spec §5, avoids cc-switch #6994).
    pub fn completeness(&self) -> i32 {
        let mut n = 0;
        n += (self.input_tokens > 0) as i32;
        n += (self.output_tokens > 0) as i32;
        n += (self.reasoning_tokens > 0) as i32;
        n += (self.cache_read_tokens > 0) as i32;
        n += (self.cache_write_5m_tokens + self.cache_write_1h_tokens > 0) as i32;
        n += (self.cost_usd.is_some()) as i32;
        n += (self.credits.is_some()) as i32;
        n += (self.duration_ms.is_some()) as i32;
        n
    }

    /// Metering gate: any billed dimension > 0 qualifies for storage
    /// (Anthropic bills input+cache from request start; requiring output>0
    /// would systematically undercount — spec §6.1).
    pub fn is_billable(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.reasoning_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_write_5m_tokens + self.cache_write_1h_tokens > 0
            || self.credits.is_some_and(|c| c > 0.0)
            || self.cost_usd.is_some_and(|c| c > 0.0)
    }

    pub fn cache_write_total(&self) -> u64 {
        self.cache_write_5m_tokens + self.cache_write_1h_tokens
    }
}

/// A subscription/quota signal — never mixed with API dollars (spec §7.4).
#[derive(Debug, Clone)]
pub struct QuotaSnapshot {
    pub app: String,
    pub account: Option<String>,
    pub captured_at: i64,
    /// `5h_block` | `monthly` | `daily` | `credits`
    pub window_kind: String,
    pub used: Option<f64>,
    pub limit_value: Option<f64>,
    pub used_percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub raw_json: Option<String>,
}
