# AI 编码工具统一用量统计器（GlobalTokenTracker）——最终实施方案

> 版本：v2.0（深度调研版）｜日期：2026-09-26
> 调研方式：7 路智能体（本机两轮逐目录实测、cc-switch 源码级分析、TokenTracker/cursor-usage/tokcat/agent-trail 源码克隆拆解、价格表实测下载比对、官方文档核验）+ 本机 31 万条真实记录交叉对账。
> 所有"本机实测"结论均来自这台机器的真实数据，非文档推测。

---

## 0. 结论摘要

**做得到，且数据基础比预想的好。** 本机 14 个 AI 编码工具中，7 个可以从本地文件精确统计 token（Claude Code、Codex、OpenCode、ZCode、Grok、WorkBuddy、CodeBuddy Code CLI），4 个可以拿到官方订阅配额（Claude、Codex、Qoder、Cursor），剩余（CodeBuddy IDE、Gemini、Devin、Copilot、Windsurf）本机无使用数据或无计费数据，做元数据兜底即可。

三个决定设计的实测发现：

1. **朴素累加会多算 2.1 倍**。Claude Code 的 JSONL 中同一条流式消息会写多行，必须按 `message.id` 去重、取最后一条。实测：朴素累加 9.88B cache_read token，去重后 4.85B，与 cc-switch 数据库 4.86B 误差 0.12%。
2. **费用字段不可靠，成本必须自己算**。Claude 的 `cost-state.totalCostUSD` 在本机全为 0，Codex 完全无 cost 字段；价格表（models.dev 为主）对本机 15 个模型名覆盖 13 个，2 个 unpriced 需用户覆写。
3. **Qoder 新版已放弃 token 计费**，改为 credit 直扣（转录里 `{credits: 3.2}`，token 字段恒 0）。对 Qoder 只能做"配额/credits"维度，全网开源工具（agent-trail、statusline 等）还都卡在已废弃的旧路径上——这恰好是差异化空间。

---

## 1. 目标与范围

一个 Windows 桌面程序（含托盘常驻），统一统计本机所有 AI 编码工具的：

| 维度 | 说明 |
|---|---|
| token 消耗 | 输入 / 输出 / 思考 / 缓存读 / 缓存写，统一口径归一化 |
| 费用 | API 制：按价目表估算美元；订阅制：credits / 配额窗口百分比。两类绝不混算 |
| 时间 | 会话时长、回合时长、请求延迟、TTFT、活跃编码时间（OTel） |
| 维度切分 | 按工具 / 按模型 / 按项目 / 按天（热力图）/ 按供应商账号 |
| 溯源 | 每条记录可回查到源文件+偏移；每笔金额标注 provenance（official/computed/estimated） |

明确不做：代理拦截（对走官方后端的 IDE 不可行，见 §6.10）、跨设备云同步（MVP 不做）。

---

## 2. 本机数据源全量清单（两轮实测合并）

✅=精确可用 🟡=部分/元数据 ❌=本机无数据

| 工具 | 数据路径 | 格式 | token | 费用 | 时间 | 精度 |
|---|---|---|---|---|---|---|
| **Claude Code** | `~/.claude/projects/<slug>/*.jsonl`（26 文件） | JSONL | ✅ 五类 token 逐条 | 🟡 cost 字段全 0，须自算 | ✅ timestamp+durationMs | ✅ |
| **Codex** | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`（515 个/1.78GB）+ `state_5.sqlite` + `thread_history_1.sqlite` | JSONL+SQLite | ✅ `token_count` 事件（累计+当轮增量+reasoning） | ❌ 无 cost 字段 | ✅ turn 级 duration_ms | ✅ |
| **OpenCode** | `~/.local/share/opencode/opencode.db`（130MB） | SQLite | ✅ session 表原生四列 | ✅ `session.cost` 原生 USD | ✅ time_created/updated | ✅ |
| **ZCode** | `~/.zcode/cli/db/db.sqlite`（137MB，`model_usage` 4759 行） | SQLite | ✅ 五类 token 最规范 | ❌ 须自算 | ✅ duration_ms+TTFT | ✅ |
| **Grok CLI** | `~/.grok/sessions/<cwd>/<uuid>/updates.jsonl` | JSONL | ✅ 33 条 usage（驼峰命名） | 🟡 `costUsdTicks` 整数刻度 | ✅ apiDurationMs | ✅ |
| **WorkBuddy** | `~/.workbuddy/projects/**/*.jsonl` + `~/.workbuddy/workbuddy.db` `session_usage`（39 行） | JSONL+SQLite | ✅ `providerData.rawUsage`（含子代理两层） | 🟡 `credit_json`（按 md5 模型键的积分） | ✅ | ✅ |
| **CodeBuddy Code CLI** | `~/.codebuddy/projects/**/*.jsonl`（Claude fork 结构，CLI= `@tencent-ai/codebuddy-code`） | JSONL | ✅ `providerData.rawUsage`（4 种缓存字段变体） | ❌ 须自算 | ✅ | ✅ |
| **CodeBuddy IDE (CN)** | `%APPDATA%/CodeBuddy CN/...state.vscdb` | SQLite | ❌（secret:// 键是 DPAPI 加密死路） | 🟡 `Tencent-Cloud.coding-copilot` 键 → base64(gzip) 332KB JSON 含 23 模型倍率（"x0.29"） | 🟡 codebuddy-sessions.vscdb 会话级 | 🟡 |
| **Qoder** | `~/.qoder/logs/sessions/**/segments/*.jsonl`（78 会话）+ `main.sqlite`（95 表全 0 行） | JSONL | ❌ 新版转录是 credit-only `{credits:3.2}` | 🟡 credits 需走官方 API（§6.9） | ✅ turn_id/phase/duration | 🟡 |
| **Cursor** | `%APPDATA%/Cursor/User/globalStorage/state.vscdb`（385MB） | SQLite | ❌ 本地无 token | 🟡 官方非公开 API（§6.9） | 🟡 | 🟡 |
| **Gemini CLI** | `~/.gemini/tmp/<project>/chats/session-*.jsonl`（源码证实，本机未用） | JSONL | ✅（若使用）`tokens{input,output,cached,thoughts,tool}` | ❌ 无 cost（免费额度） | ✅ | ✅ |
| **Devin** | `$APPDATA/Devin/cli/sessions.db` | SQLite | ❌ 只有上下文估算 | ❌ | 🟡 | ❌ |
| **Copilot CLI** | `~/.copilot/`（66KB） | — | ❌ | ❌ | ❌ | ❌ |
| **Windsurf** | `~/.codeium/windsurf/`（protobuf 全空） | — | ❌ | ❌ | ❌ | ❌ |
| **cc-switch**（参考+对账源） | `~/.cc-switch/cc-switch.db`：`proxy_request_logs` 34,051 行 / `model_pricing` 218 / `session_log_sync` 511 游标 | SQLite | ✅ 五类已统一（claude 侧经实测验证准确） | ✅ total_cost_usd | ✅ | ✅ |

---

## 3. 关键实测验证（设计依据）

### 3.1 去重规则验证（最重要）

用 Python 全量解析本机 26 个 Claude JSONL（23,675 条 assistant 行）：

| 口径 | 消息数 | input | output | cache_read |
|---|---|---|---|---|
| 朴素逐行累加 | 23,675 | 18,503,546 | 26,435,773 | **9,885,087,631** |
| **按 message.id 去重（保留最后一条）** | **11,099** | **2,295,068** | **10,151,045** | **4,850,457,501** |
| cc-switch 数据库 | 11,146 | 2,295,214 | 10,200,602 | 4,855,830,804 |

去重后与 cc-switch 误差 **0.1%**（其多出的 47 条来自已删除的历史文件）。结论：**Claude Code 流式响应会把同一 message.id 写多行（usage 逐步累积），必须"同 id 保留最后一条"**。这也解释了行业工具间的数字打架——谁不做去重谁的数字就虚高一倍。

### 3.2 成本字段验证

- Claude `cost-state` 行的 `totalCostUSD` 本机全部为 0（另一会话曾出现过 53.35，**字段不恒定**）→ 不可依赖，成本一律按价目表计算，官方字段只作辅助并标 provenance。
- cc-switch 对 Claude 全期估算 **$2,592.80**（其内置 218 模型价目表），可作为我们计价管线的对账基准。

### 3.3 价格表覆盖率验证（三库实测下载比对，2026-09-26）

本机 15 个模型名 × 三大价格库：

| 模型名 | models.dev | LiteLLM | cc-switch 内置 | 结论 |
|---|---|---|---|---|
| claude-opus-5 / -5-5 | ✅ 直击 | ✅ | ✅ | 全覆盖 |
| gpt-5.6-sol / gpt-6-astra | ✅ | ✅ | ✅ | 全覆盖 |
| grok-4.6 / glm-5.3 / glm-5.3-flash / kimi-k3 / qwen3-max / qwen3-coder | ✅ | 🟡 需 provider 前缀（`xai/`、`zai/`、`moonshot/`…） | ✅ | 归一层解决 |
| step-5-preview | ✅ | ❌ 无 | ✅ | models.dev 补位 |
| muse-spark-1.3-contributor | ✅ | 🟡 前缀 | ❌ | models.dev 补位 |
| stealth/custom-alpha | 🟡 有键价 0 | 🟡 有键价 0 | ❌ | **unpriced，用户覆写** |
| gpt-reserve / codex-auto-review | ❌ | ❌ | ❌ | **unpriced，见 §7.3** |

**结论：models.dev 是本机模型的完备超集（13/15 直击），选它做主库**；LiteLLM 独有价值是分档计价（4 个正交轴：`_above_200k_tokens`、`_above_1hr`、`_batches`、组合，字段拼写已逐一核实）；OpenRouter 的 `canonical_slug`（458/458 全有）做第三回退。

单位陷阱（实测核对）：models.dev 是 **$/1M token**（`cost{input,output,cache_read,cache_write}`），LiteLLM 是 **$/token**（`input_cost_per_token`…），换算错误会差 100 万倍。

### 3.4 本机已有可复用资产

- `~/.zcode/workspace/default/ccs/`：cc-switch 反编译源码 9 个文件（`usage_mod.rs`、`usage_stats.rs`、`model_mapper.rs`、`proxy_usage_calculator.rs`、`model_pricing.rs`、`modelsDevPricing.ts`、`types_usage.ts`、`database_schema.rs`、`logger.rs`）——归一化算法与 schema 的现成参考。
- `~/.zcode/workspace/default/pricedata/`：三库原始 JSON + 4 个匹配脚本（match/deep/ll/precise.mjs）+ report.json——价格管线可直接复用。
- 外部克隆：TokenTracker / cursor-usage / tokcat / agent-trail / gemini-cli 源码在 `%LOCALAPPDATA%/Temp/tt-research/`。

---

## 4. 总体架构

```
┌──────────────────────── 采集层（Rust 守护进程，托盘常驻，随 ZCode/IDE 自然运行） ───────────────────────┐
│                                                                                                     │
│ ① 文件监听: notify crate 监听各数据目录 + 60s 兜底轮询（防漏事件）                                     │
│ ② SourceAdapter × 12（§6）：字节游标+尾部指纹增量解析，产出统一 UsageEvent                             │
│ ③ OTLP 接收器: 127.0.0.1:4317(grpc)/4318(http) —— Claude Code 官方指标（含美元成本与活跃秒数）          │
│ ④ 配额轮询器(§6.9): Codex wham API / Claude oauth usage / Qoder credits / Cursor RPC（可关，默认低频）  │
│ ⑤ 价格同步器: models.dev(主) + LiteLLM(分档) + OpenRouter(canonical_slug) + 内置离线快照 + 用户覆写      │
└──────────────────────────────────┬──────────────────────────────────────────────────────────────────┘
                                   ▼  归一化管线（§7：字段变体→口径归一→模型别名→计价→provenance）
┌──────────────────────── 存储层（~/.globaltokentracker/ledger.db，SQLite WAL） ────────────────────────────────┐
│ usage_events(明细, dedup_key UPSERT) │ sync_cursors(游标) │ quota_snapshots(配额时序)                    │
│ prices / price_overrides / model_aliases │ daily_rollups(本地午夜对齐) │ sources(启停/健康)             │
└──────────────────────────────────┬──────────────────────────────────────────────────────────────────┘
                                   ▼
┌──────────────────────── 展示层（Tauri 2 + React，参考 cc-switch 技术栈） ──────────────────────────────┐
│ 总览 │ 按工具·模型·项目·日期 │ 订阅配额页(5h块/月窗/credits) │ 明细溯源 │ 价格覆写 │ 托盘(今日$/当前窗口) │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

技术栈：**Tauri 2 + Rust**（流式解析 2GB+ JSONL 需要性能；tokscale 已验证 SIMD-JSON 路线；与 cc-switch 同栈便于参考其反编译源码）+ React 前端（recharts）。存储 SQLite（rusqlite，WAL 模式）。

---

## 5. 统一数据模型

```sql
-- 明细表：所有工具所有通道的唯一落点
CREATE TABLE usage_events (
  id INTEGER PRIMARY KEY,
  dedup_key TEXT UNIQUE NOT NULL,     -- 规则见 §7.1
  app TEXT NOT NULL,                  -- claude|codex|opencode|zcode|grok|workbuddy|codebuddy_cli|codebuddy_ide
                                      -- |qoder|cursor|gemini|gemini_antigravity|copilot|devin|windsurf
  session_id TEXT, project TEXT, account_id TEXT, provider_id TEXT,
  model TEXT,                         -- 上报原始名
  request_model TEXT,                 -- 客户端请求别名（与真实模型分开存，审计用）
  pricing_model TEXT,                 -- 归一后实际计价键；NULL=unpriced
  ts_start INTEGER, ts_end INTEGER,   -- epoch ms
  input_tokens INTEGER DEFAULT 0,
  output_tokens INTEGER DEFAULT 0,
  reasoning_tokens INTEGER DEFAULT 0,
  cache_read_tokens INTEGER DEFAULT 0,
  cache_write_5m_tokens INTEGER DEFAULT 0,
  cache_write_1h_tokens INTEGER DEFAULT 0,
  credits REAL,                       -- Qoder/WorkBuddy 等 credit 制
  input_semantics TEXT DEFAULT 'excludes_cache',  -- 归一化后的统一口径
  cost_usd REAL, cost_source TEXT,    -- official|provider_reported|computed|estimated|unpriced
  provenance TEXT NOT NULL,           -- local_jsonl|local_sqlite|otel|vendor_api|dashboard|ccswitch_db
  duration_ms INTEGER, ttft_ms INTEGER, active_ms INTEGER,
  status TEXT, error TEXT,
  raw_ref TEXT                        -- 溯源：源文件路径+行号/字节偏移
);
CREATE INDEX idx_events_time ON usage_events(ts_start);
CREATE INDEX idx_events_app   ON usage_events(app, ts_start);

-- 增量游标（照抄 cc-switch session_log_sync 四字段设计，已验证可靠）
CREATE TABLE sync_cursors (
  source TEXT, file_path TEXT PRIMARY KEY,
  last_byte_offset INTEGER, last_tail_fingerprint TEXT,  -- sha2(游标前尾段)
  last_modified INTEGER, last_synced_at INTEGER
  -- 截断(偏移越界)或指纹不符 → 游标钉到 EOF，绝不重放（重放已 rollup 区间=永久双算）
);

CREATE TABLE quota_snapshots (        -- 配额时序（窗口百分比/credits余额/reset时间）
  id INTEGER PRIMARY KEY, app TEXT, account TEXT,
  captured_at INTEGER, window_kind TEXT,     -- 5h_block|monthly|daily|credits
  used REAL, limit_value REAL, used_percent REAL, resets_at INTEGER, raw_json TEXT
);

CREATE TABLE prices (                 -- models.dev 主库（$/1M）+ LiteLLM 分档列（$/token 换算后统一 $/1M）
  provider TEXT, model_id TEXT, input REAL, output REAL,
  cache_read REAL, cache_write REAL,
  tier_above_200k_input REAL, tier_1h_cache_write REAL, tier_batch REAL,  -- LiteLLM 分档
  source TEXT, fetched_at INTEGER,
  PRIMARY KEY (provider, model_id)
);
CREATE TABLE price_overrides (        -- 用户覆写层（最高优先级，含 unpriced 模型手填）
  model_key TEXT PRIMARY KEY, input REAL, output REAL, cache_read REAL, cache_write REAL,
  note TEXT, updated_at INTEGER, deleted INTEGER DEFAULT 0   -- deleted=墓碑
);
CREATE TABLE model_aliases (          -- 归一缓存 + 用户手工映射
  raw_name TEXT PRIMARY KEY, resolved_pricing_model TEXT, resolved_via TEXT, hit_at INTEGER
);
CREATE TABLE daily_rollups (
  date TEXT, app TEXT, provider TEXT, request_model TEXT, pricing_model TEXT,
  events INTEGER, input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER,
  cache_read_tokens INTEGER, cache_write_5m INTEGER, cache_write_1h INTEGER,
  credits REAL, cost_usd REAL, active_ms INTEGER,
  PRIMARY KEY (date, app, provider, request_model, pricing_model)
);
```

**UPSERT 规则（避开 cc-switch #6994 事故）**：`ON CONFLICT(dedup_key) DO UPDATE ... WHERE 新行计费维度更完整`——流式消息从快照行演进到终态行时，后者覆盖前者；绝不用 `INSERT OR IGNORE` 锁死主键。

---

## 6. 适配器规格（12 个，按优先级）

### 6.1 Claude Code（P0，本地 JSONL）
- 路径：`~/.claude/projects/<cwd-slug>/<session-uuid>.jsonl`（子代理同目录）。
- 字段：`type=="assistant"` → `message.usage.{input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens}`；缓存写拆分 `usage.cache_creation.{ephemeral_5m_input_tokens, ephemeral_1h_input_tokens}`（无拆分时全按 5m 价）；`output_tokens_details.thinking_tokens` → reasoning；`message.model`；行级 `timestamp`、`durationMs`。
- **去重铁律**：按 `message.id` 保留最后一条（§3.1 实测 2.1 倍差异）。
- 计量 gate：任一计费维度 >0 即入库（Anthropic 请求开始即对 input+cache 计费；要求 output>0 会系统性低估）。
- 成本：价目表计算（cache_read=0.1×input，5m 写=1.25×，1h 写=2×）；`cost-state.totalCostUSD` 仅在 >0 时作 official 参照。

### 6.2 Codex（P0）
- 主源：`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`（+`archived_sessions/`）。取 `type=="event_msg" && payload.type=="token_count"`：`info.last_token_usage` 是当轮增量；`info.total_token_usage` 是累计值（二选一，用增量优先，累计值做校验和）。`payload.rate_limits` 顺手写入 `quota_snapshots`（used_percent/window_minutes/resets_at/plan_type——免费的官方配额信号）。
- 快源：`state_5.sqlite.threads.tokens_used`（单调累计，495 线程预聚合）做"会话级"视图，与明细差值校验；`thread_history_1.sqlite.thread_turns.{started_at,completed_at,duration_ms}` 提供回合时长；`thread_turns.rollout_byte_offset` 可做跳读游标。
- 模型：`turn_context` 事件的 model；注意 `gpt-reserve`、`codex-auto-review` 是服务端路由别名 → 按 §7.3 处理。
- ⚠️ cc-switch 的 `common_config_codex` 会覆写 `~/.codex/config.toml`——我们的工具不写用户配置文件，只读，无冲突。

### 6.3 OpenCode（P0）
- 路径：`~/.local/share/opencode/opencode.db`（注意**不在** `~/.opencode`）。
- `session` 表原生：`cost`（USD）、`tokens_input/output/reasoning/cache_read`、`time_created/time_updated`、`title`。`message.data` JSON 有 `model{providerID,modelID}`。
- 成本来源标 `provider_reported`（其 cost 由上游返回时）；无 cost 的会话回落价目表。只读模式打开（WAL 兼容）。

### 6.4 ZCode（P0）
- 路径：`~/.zcode/cli/db/db.sqlite` 表 `model_usage`（4759 行，五类 token + duration_ms + TTFT + status + error_type）。
- 去重键：`logical_request_id + attempt_index`（id 含 retry 后缀）。
- 跨源去重注意（TokenTracker 实证）：providerID 为 `anthropic/openai/google` 时其子代理数据会写进 `~/.claude`/`~/.codex` 等由对应适配器统计，**这两类 provider 行默认跳过并计数**（可配置），防止双计。
- 备份源：`~/.zcode/cli/rollout/model-io-*.jsonl`（traceId 可交叉校验）。额度：`~/.zcode/v2/coding-plan-cache.json` 仅 entitlement（非余额），仅展示。

### 6.5 Grok CLI（P1）
- 路径：`~/.grok/sessions/<urlencoded-cwd>/<uuid>/updates.jsonl`，筛 `method ∈ {session/update, _x.ai/session/update}`，usage 在 `params.usage`。
- 字段映射（命名变体！）：`inputTokens/outputTokens/cachedReadTokens(拼写少个 c)/cacheCreationTokens/reasoningTokens/apiDurationMs`；`costUsdTicks` 整数刻度（187081600 ≈ $0.187，即 1e9 ticks = $1，上线前用多会话抽样校准）。`modelUsage{<model>:{...}}` 支持按模型拆分。

### 6.6 WorkBuddy（P1）
- 主源：`~/.workbuddy/projects/**/*.jsonl`，聚合**所有**带 `providerData.rawUsage` 的记录（不止 assistant 行），按 response id 去重；**子代理嵌套两层** `subagents/agent-*.jsonl` 需递归。
- 口径陷阱（源码实证）：`prompt_tokens` 是完整 prompt（含 cache read+write）→ `input = prompt - cacheRead - cacheCreate`；`model` 常为字面量 `"auto"`（自动路由不暴露真实模型）→ 计价标 estimated。
- 辅源：`~/.workbuddy/workbuddy.db.session_usage`（`used`=token 数、`credit_json` 按 md5 模型键的积分消耗——md5→模型名映射本地对不上，按 credits 展示）。

### 6.7 CodeBuddy（P1：CLI 精确 + IDE 元数据）
- CLI（`@tencent-ai/codebuddy-code`，是 Claude Code 完整 fork）：`~/.codebuddy/projects/<encoded-cwd>/<sessionId>.jsonl`，**直接复用 6.1 的解析器**，兼容 4 种缓存字段取 max：`cache_read_input_tokens`(Anthropic) / `prompt_tokens_details.cached_tokens`(OpenAI) / `prompt_cache_hit_tokens`(DeepSeek) / `prompt_cache_write_tokens`；`completion_tokens` 含 reasoning 需相减。支持 SessionEnd hook（写 `~/.codebuddy/settings.json`，hook 只作触发扫描的信号、不传数据）。
- IDE (CN)：token 无（secret:// 键为 DPAPI 加密，死路）；但 `state.vscdb` 键 `Tencent-Cloud.coding-copilot` → `.CodeBuddy-Product-Cache` = base64(gzip) 332KB JSON，含 **23 个模型的 credits 倍率**（`"x0.29"`）——作为倍率配置源；`codebuddy-sessions.vscdb` 提供会话级时长。IDE 侧额度走腾讯云控制台（无 API，实测 404），显示为手动登记。

### 6.8 Gemini CLI / Antigravity（P2，本机未使用，先备好）
- 路径（gemini-cli 源码证实）：`~/.gemini/tmp/<projectIdentifier>/chats/session-<ts>-<id8>.jsonl`（子代理在 `<父sessionId>/` 子目录）。`type=="gemini"` 行的 `tokens{input, output, cached, thoughts, tool, total}` + `model`。**tokens 可为 null（免费轮次），缺失即跳过**；`total` 不可信，按 `input+output+cached+tool` 重算；`input` 含 `cached`，计价前拆出。零配置默认落盘。
- OTel 备选：`settings.json.telemetry{enabled, target, otllpEndpoint, otlpProtocol(grpc|http), outfile}`；指标 `gemini_cli.token.usage`（type: input/output/thought/cache/tool）。⚠️ `GEMINI_TELEMETRY_LOG_PROMPTS` 默认 true（隐私提示）；outfile 是带缩进的 JSON 流（非 NDJSON），解析按缩进块。

### 6.9 官方配额/订阅通道（P2，凭据文件可用性已验证）

| 工具 | 端点 | 凭据 | 实测状态 |
|---|---|---|---|
| Codex（ChatGPT 订阅） | `GET chatgpt.com/backend-api/wham/usage/daily-token-usage-breakdown`（每日 token 明细）、`.../usage/credit-usage-events`、`.../wham/profiles/me` | `~/.codex/auth.json` OAuth（刷新：`auth.openai.com/oauth/token`，client_id `app_EMoamEEZ73f0CkXaXp7hrann`，tokcat 已验证） | ✅ auth.json 存在（4218B） |
| Claude（订阅） | `GET api.anthropic.com/api/oauth/usage`（tokcat 验证） | `~/.claude/.credentials.json` | ✅ 存在（<size>） |
| Qoder（credits） | `GET qoder.com/api/v2/me/usages/big_model_credits`（CN 版 qoder.com.cn；v1 端点 + organization-shared 变体） | 需 cookie：本机 `~/.qoder/.auth/` 只有 machine_id 无 cookie → 用户手贴 / 客户端运行时走本地 IPC（`SharedClientCache/.info.json` JSON-RPC，新版路径已迁移，可能无此文件）/ renderer.log 正则兜底 | 🟡 需用户配合 |
| Cursor | ①总量：Connect RPC `api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage`（Bearer 直连、无 CSRF，tokcat 验证）；②明细：`POST cursor.com/api/dashboard/get-filtered-usage-events`（per-chat `tokenUsage{input,output,cacheRead,cacheWrite,totalCents}`；**必须带 `Origin: https://cursor.com` 头**） | `%APPDATA%/Cursor/User/globalStorage/state.vscdb` 键 `cursorAuth/accessToken`（存在，JWT；⚠️ 两个参考项目都只实现了 macOS 路径，Windows 路径由我们补齐） | ✅ 凭据在 |
| 被动配额 | Codex rollout 的 `rate_limits`（每请求免费刷新）；Claude OTel cost/token | 无需凭据 | ✅ |

工程约束：轮询默认 30 分钟一次、可关、失败静默降级并标 `stale`；凭据只读、永不外传、日志脱敏；非公开端点失效时 UI 明确提示"通道失效"而非归零。

### 6.10 明确放弃：代理拦截
四重障碍（私有协议+证书固定+endpoint 协商/machine_id 绑定+抓到的也是厂商自报 credit 数），且三个 IDE 的账号 token 都在其自家后端。**结论：对 Qoder/CodeBuddy IDE/Cursor 放弃 MITM 路线**，用上面的磁盘+API+hook 三件套。

### 6.11 cc-switch 作为对账源（P1）
`~/.cc-switch/cc-switch.db` 只读接入，provenance=`ccswitch_db`。用途：①初始化历史数据（其 claude/codex/opencode 明细已验证准确）；②每日报表交叉校验（差异 >2% 告警）。注意其 `usage_daily_rollups` 只有 codex/opencode 两类，claude/grok 需从 `proxy_request_logs` 聚合；与原始解析重复的部分以原始源为准、cc-switch 仅对账，防止双计。

### 6.12 兜底工具（P3）
Devin（`sessions.db` 仅元数据+上下文估算）、Copilot CLI、Windsurf、Claude Desktop：标记 `unmonitored`，支持用户手填月费做订阅摊销，不冒充精确数据。

---

## 7. 归一化与计价管线

### 7.1 字段变体归一（本机实测至少 4 套命名）

| 语义 | 变体 | 出现处 |
|---|---|---|
| input | `input_tokens` / `inputTokens` / `prompt_tokens` | 全部 |
| cache read | `cache_read_input_tokens` / `cacheReadInputTokens` / `cachedReadTokens`(Grok 少 c) / `cached_tokens` / `cacheReadTokens` | Claude/Codex/Grok/Qoder/Gemini |
| cache write | `cache_creation_input_tokens` / `cacheCreationTokens` / `prompt_cache_write_tokens` / models.dev 叫 `cache_write` | 全部 |
| cost | `total_cost_usd`(字符串 Decimal) / `costUsdTicks`(整数) / `cost`(float) / `credit_json` | cc-switch/Grok/OpenCode/WorkBuddy |

适配器输出前先映射到统一 schema；`input_semantics` 统一标为 `excludes_cache`（Grok/WorkBuddy/Qoder 的 prompt 含缓存，入裸表前先减掉，减法规则在各自适配器内完成）。

### 7.2 模型别名归一（照抄 cc-switch 候选队列算法，实测有效）
1. 基础归一：`rsplit_once('/')` 取末段（`stealth/custom-alpha` → `custom-alpha`）→ 按 `:` 切掉后缀 → `@`→`-` → 小写 → 剥 `[1m]`。
2. BFS 候选队列逐级剥离：`openai./anthropic./moonshot./bedrock./global.` 前缀、`rfind("claude-")`、`-v<数字>`、`-YYYYMMDD` 日期后缀、`-minimal/-low/-medium/-high/-xhigh` 推理档后缀。
3. 精确匹配全部失败才前缀匹配（`LIKE '<cand>-%'` 取最短命中），且设 dash 数门槛防误吞（`claude-`≥3、`gpt-/gemini-/qwen-/glm-/kimi-`≥2 个横线——防止 `gpt-5.6` 吞掉 `gpt-5.6-sol`）。
4. 解析结果写 `model_aliases` 缓存 + `resolved_via` 审计。
5. 匹配顺序：`price_overrides`（用户）→ models.dev（provider 锚定）→ LiteLLM（带前缀键）→ OpenRouter canonical_slug → **unpriced**。

### 7.3 unpriced 模型处理（实测存在：`gpt-reserve`、`codex-auto-review`、`stealth/custom-alpha`）
- 优先用**响应体里的真实模型名**计价（`request_model` 与 `model` 分列的原因；cc-switch 的 `pricing_model_source='response'` 同思路）。
- 仍无价：记 0 成本 + UI 标 `unpriced` 徽标 + 引导用户在价格覆写弹窗手填（`price_overrides` 表）。**绝不猜价、绝不同族平摊**（gpt-5.6-sol $4/$20 与 gpt-6-astra $10/$50 差 2.5 倍，猜就是错账）。UI 上"真免费"与"没查到价"必须区分显示。

### 7.4 计价规则
- 统一单位 $/1M；四分量：input + output + cache_read(≈0.1×) + cache_write(5m=1.25× / 1h=2×)。
- LiteLLM 分档轴按会话上下文实际值触发：`_above_200k_tokens`（Claude 长上下文真实会发生）、`_above_1hr`、`_batches`。
- 订阅制（Claude Max/Codex Plan/Qoder credits/Cursor Pro）：**不折算美元**，走 quota_snapshots 显示"已用百分比 + 重置时间 + credits 余额"；面板上与 API 制美元分列。
- 金额全部标注 `cost_source`；面板角标"估算值，非账单"。价格快照内置离线种子（首启/断网可用），24h TTL 增量刷新，上游改价不回写历史（历史按当日快照价冻结）。

---

## 8. 时间与活跃度统计

| 层级 | 来源 |
|---|---|
| 请求级延迟 | cc-switch `latency_ms/first_token_ms`（历史）；ZCode `duration_ms/TTFT`（实时） |
| 回合级时长 | Codex `thread_turns.duration_ms`、ZCode `turn_usage`、Claude `durationMs` |
| 会话级时长 | 各源首末事件差（Claude timestamp、OpenCode time_created/updated、CodeBuddy createdAt/updatedAt） |
| 活跃编码时间 | Claude Code OTel `claude_code.active_time.total`（秒，官方指标）——**这是唯一能回答"我今天真正用了几小时"的数据** |
| 使用规律 | 按天热力图 + 按小时分布（ts_start 聚合） |

---

## 9. UI / 功能设计

1. **总览页**：今日/本周/本月卡片（总 token、估算 $、credits 消耗、活跃时长）；各工具占比环形图；7/30 天趋势；当前 5 小时计费块进度（Claude）与 Codex 月窗百分比。
2. **明细页**：按工具/模型/项目/日期四维透视表 + 日历热力图；点任意聚合格下钻到 `usage_events` 行；每行可展开 raw_ref 溯源（源文件+偏移+原始 JSON 片段）。
3. **配额页**：各订阅账号的窗口用量、reset 倒计时、credits 余额时序（quota_snapshots）、burn rate 与"预计何时用完"。
4. **数据源页**：14 个源的启停开关、健康状态（最后同步时间/游标位置/错误）、覆盖能力说明（精确/估算/元数据三档徽标）。
5. **价格页**：价目表浏览/搜索；unpriced 模型提示；用户覆写编辑（含墓碑删除）。
6. **托盘**：今日 $ 与当前窗口百分比，tooltip 显示 top3 工具；点击唤起面板。

---

## 10. 分期路线图与验收标准

| 阶段 | 内容 | 验收标准（用本机真实数据） |
|---|---|---|
| **M0 核心（2-3 天）** | schema + Claude/Codex/OpenCode/ZCode 四适配器 + 字节游标增量 + 价格管线（models.dev+LiteLLM+覆写）+ CLI 报表 | Claude 全量回溯与 cc-switch 对账误差 <1%（基线 0.1% 已验证可达）；Codex tokens_used 差值与 rollout 明细差值 <0.5%；2 个 unpriced 模型正确标徽标 |
| **M1 面板（1 周）** | Tauri UI 六页面 + Grok/WorkBuddy/CodeBuddy CLI 适配器 + cc-switch 对账源 + 托盘 | Grok 33 条 usage 全部入库且 costUsdTicks 校准误差 <5%；WorkBuddy 子代理两层递归不漏；报表与 ccusage `daily --json` 交叉对账 <2% |
| **M2 配额与 OTel（1 周）** | OTLP 接收器 + Claude `env` 块引导配置；Codex wham API；Qoder segments 活跃度 | OTel 收到 `claude_code.cost.usage`（美元）且与价目表估算偏差可解释（<15%，费率快照时差）；wham 每日用量与本地 rollout 聚合趋势一致 |
| **M3 IDE 攻坚（1-2 周）** | Cursor RPC+dashboard 双通道（Windows 凭据路径）；Qoder credits API（cookie 引导）；CodeBuddy IDE 倍率表热更新；Gemini 适配器 | Cursor 拿到 per-chat token 与 chargedCents（totalCents 降级生效）；Qoder credits 与 IDE 显示一致；Gemini 对 ccusage gemini 报表 <2% |
| **M4 打磨** | CSV 导出、burn-rate 告警、 rollup/prune（本地午夜对齐）、异常检测（Codex 计数器交错 6-8 倍这类病态数据自动熔断标记） | 30 天明细 prune 后报表数字不变 |

---

## 11. 风险与边界（诚实清单）

1. **非公开接口会变**：Cursor dashboard/RPC、Qoder credits API、OpenAI wham 均无稳定性承诺 → 适配器热更新 + 失效显式提示 + 本地估算兜底。
2. **DPAPI/加密缓存不可逆**：CodeBuddy secret:// 键已确认是死路，不做尝试。
3. **cc-switch 配置覆写冲突**：它管理 `~/.claude/settings.json` 与 `~/.codex/config.toml`；我们 OTel 引导写入 `env` 块时要合并写入（保留其键），并在文档提示。
4. **Claude 30 天本地清理**：Claude Code 会清旧 JSONL → M4 提供可选"档案库"（把明细归档到 ledger.db 即自然解决，我们的库就是 warehouse）。
5. **隐私**：凭据文件只读不外传；Gemini `LOG_PROMPTS` 默认 true 的提醒；本项目不采集任何遥测。
6. **价格波动**：历史按当日快照冻结，改价不回写；报表永远标"估算"。
7. **估计不可达的**：Qoder/CodeBuddy IDE 的服务端真实扣费、ChatGPT 订阅的美元账单——用 credits/百分比呈现，不伪造精度。

---

## 12. 附录：主要来源

**本机实测**：两轮智能体勘探原始数据（本文 §2/§3 全部路径与数字）；对账脚本输出（naive 23,675 行 / dedup 11,099 / cc-switch 11,146）。

**源码级**：
- cc-switch：https://github.com/farion1231/cc-switch （schema.rs / session_usage*.rs / usage_rollup.rs / usage_script.rs；issues [#6994](https://github.com/farion1231/cc-switch/issues/6994)、[#7384](https://github.com/farion1231/cc-switch/issues/7384)、[#1855](https://github.com/farion1231/cc-switch/issues/1855)）；本机反编译件 `~/.zcode/workspace/default/ccs/`
- TokenTracker：https://github.com/xiufengsun/TokenTracker （rollout.js 23,793 行 / qoder-limits.js 四级降级 / pricing 分层）
- cursor-usage：https://github.com/chocolatemale/cursor-usage （fetch_cursor_usage.py / pitfalls.md）；tokcat：https://github.com/handlecusion/tokcat （agent_usage.rs：wham/oauth-usage/Connect RPC 端点）
- agent-trail：https://github.com/camtrik/agent-trail （qoder.ts）；OpenAI Codex：https://github.com/openai/codex （backend-client/analytics.rs）；Gemini CLI：https://github.com/google-gemini/gemini-cli （chatRecordingService.ts / metrics.ts / telemetry docs）
- CodeBuddy Code：https://www.npmjs.com/package/@tencent-ai/codebuddy-code 、https://cnb.cool/codebuddy/codebuddy-code

**官方文档**：Claude Code 遥测 https://code.claude.com/docs/en/monitoring-usage ｜ Qoder Credits https://docs.qoder.com/Credits ｜ LiteLLM 价格表 https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json ｜ models.dev https://models.dev/api.json

**竞品格局**：ccusage https://github.com/ccusage/ccusage ｜ CodeBurn https://github.com/getagentseal/codeburn ｜ tokscale https://github.com/junhoyeo/tokscale ｜ Splitrail https://github.com/Piebald-AI/splitrail
