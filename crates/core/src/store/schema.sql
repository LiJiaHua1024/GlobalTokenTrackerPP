-- CodeLedger unified schema (spec §5). All timestamps are epoch ms.
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- 明细表：所有工具所有通道的唯一落点
CREATE TABLE IF NOT EXISTS usage_events (
  id INTEGER PRIMARY KEY,
  dedup_key TEXT UNIQUE NOT NULL,
  app TEXT NOT NULL,
  session_id TEXT, project TEXT, account_id TEXT, provider_id TEXT,
  model TEXT,               -- 上报原始名
  request_model TEXT,       -- 客户端请求别名（审计用）
  pricing_model TEXT,       -- 归一后计价键；NULL=unpriced
  ts_start INTEGER, ts_end INTEGER,
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  cache_read_tokens INTEGER DEFAULT 0,
  cache_write_5m_tokens INTEGER DEFAULT 0,
  cache_write_1h_tokens INTEGER DEFAULT 0,
  credits REAL,                          -- Qoder/WorkBuddy credit 制
  input_semantics TEXT DEFAULT 'excludes_cache',
  cost_usd REAL, cost_source TEXT,       -- official|provider_reported|computed|estimated|unpriced
  provenance TEXT NOT NULL,              -- local_jsonl|local_sqlite|otel|vendor_api|dashboard|ccswitch_db
  duration_ms INTEGER, ttft_ms INTEGER, active_ms INTEGER,
  status TEXT, error TEXT,
  raw_ref TEXT,                          -- 溯源：源文件路径+行号/字节偏移
  completeness INTEGER NOT NULL DEFAULT 0  -- UPSERT 冲突裁决分（见 store::upsert_event）
);
CREATE INDEX IF NOT EXISTS idx_events_time ON usage_events(ts_start);
CREATE INDEX IF NOT EXISTS idx_events_app ON usage_events(app, ts_start);
CREATE INDEX IF NOT EXISTS idx_events_pricing_model ON usage_events(pricing_model, ts_start);
CREATE INDEX IF NOT EXISTS idx_events_project ON usage_events(project, ts_start);
-- Only the (few) unpriced rows: makes the overview's "unpriced models" list and the reprice
-- backfill an index scan instead of a full table pass (7ms → ~0.1ms at 63k events).
CREATE INDEX IF NOT EXISTS idx_events_unpriced ON usage_events(model, request_model)
  WHERE pricing_model IS NULL AND cost_source = 'unpriced';

-- 增量游标（cc-switch session_log_sync 设计，已验证可靠）
-- 截断(偏移越界)或指纹不符 → 游标钉到 EOF，绝不重放（重放已 rollup 区间=永久双算）
CREATE TABLE IF NOT EXISTS sync_cursors (
  source TEXT NOT NULL,
  file_path TEXT PRIMARY KEY,
  last_byte_offset INTEGER NOT NULL DEFAULT 0,
  last_tail_fingerprint TEXT,           -- sha2-256(游标前尾段)
  last_modified INTEGER,
  last_synced_at INTEGER,
  adapter_state TEXT                        -- 适配器私有续扫状态（如 codex 累计计数器）
);

-- 配额时序（窗口百分比/credits余额/reset时间）；与 API 美元分列
CREATE TABLE IF NOT EXISTS quota_snapshots (
  id INTEGER PRIMARY KEY,
  app TEXT NOT NULL, account TEXT,
  captured_at INTEGER NOT NULL,
  window_kind TEXT NOT NULL,            -- 5h_block|monthly|daily|credits
  used REAL, limit_value REAL, used_percent REAL, resets_at INTEGER,
  raw_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_quota_app_time ON quota_snapshots(app, captured_at);
-- "latest snapshot per (app, account, window)" — one index seek per key instead of a window
-- function over the whole history (22ms → 0.6ms at 11k rows); also serves insert_quota's dedup probe.
CREATE INDEX IF NOT EXISTS idx_quota_latest ON quota_snapshots(app, account, window_kind, captured_at DESC, id DESC);

-- models.dev 主库（$/1M）+ LiteLLM 分档列（$/token 换算后统一 $/1M）
CREATE TABLE IF NOT EXISTS prices (
  provider TEXT NOT NULL, model_id TEXT NOT NULL,
  input REAL, output REAL, cache_read REAL, cache_write REAL,
  tier_above_200k_input REAL, tier_1h_cache_write REAL, tier_batch REAL,
  tier_above_200k_output REAL, tier_above_200k_cache_read REAL,
  source TEXT NOT NULL, fetched_at INTEGER NOT NULL,
  PRIMARY KEY (provider, model_id)
);

-- 用户覆写层（最高优先级，含 unpriced 模型手填；deleted=墓碑）
CREATE TABLE IF NOT EXISTS price_overrides (
  model_key TEXT PRIMARY KEY,
  input REAL, output REAL, cache_read REAL, cache_write REAL,
  note TEXT, updated_at INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0
);

-- 归一缓存 + 用户手工映射
CREATE TABLE IF NOT EXISTS model_aliases (
  raw_name TEXT PRIMARY KEY,
  resolved_pricing_model TEXT,
  resolved_via TEXT,                    -- override|models_dev|litellm|openrouter|prefix|unpriced
  hit_at INTEGER NOT NULL
);

-- 本地午夜对齐日聚合（rollup 任务维护）
CREATE TABLE IF NOT EXISTS daily_rollups (
  date TEXT NOT NULL, app TEXT NOT NULL, provider TEXT NOT NULL DEFAULT '',
  request_model TEXT NOT NULL DEFAULT '', pricing_model TEXT NOT NULL DEFAULT '',
  events INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  reasoning_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_write_5m INTEGER NOT NULL DEFAULT 0,
  cache_write_1h INTEGER NOT NULL DEFAULT 0,
  credits REAL, cost_usd REAL, active_ms INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (date, app, provider, request_model, pricing_model)
);

-- 数据源启停/健康（数据源页）
CREATE TABLE IF NOT EXISTS sources (
  source TEXT PRIMARY KEY,              -- adapter id
  enabled INTEGER NOT NULL DEFAULT 1,
  last_synced_at INTEGER,
  last_error TEXT,
  files_seen INTEGER NOT NULL DEFAULT 0,
  rows_ingested INTEGER NOT NULL DEFAULT 0
);

-- schema 版本（PRAGMA user_version 不好读 diff，用表记录迁移历史）
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);

-- OTLP 推送的官方指标时序（spec §③）：每 (metric, session, 属性签名) 一行，
-- 存最新累积值 —— 官方 cost/active_time 与本地价目估算分列展示，不双计。
CREATE TABLE IF NOT EXISTS otel_metrics (
  metric TEXT NOT NULL,
  session_id TEXT NOT NULL DEFAULT '',
  attr_sig TEXT NOT NULL DEFAULT '',
  value REAL NOT NULL,
  ts_ms INTEGER NOT NULL,
  received_at INTEGER NOT NULL,
  attrs_json TEXT NOT NULL DEFAULT '{}',
  PRIMARY KEY (metric, session_id, attr_sig)
);

-- 轻量 KV 表：价格抓取节流（last_attempt）等跨会话小状态。
CREATE TABLE IF NOT EXISTS app_state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
