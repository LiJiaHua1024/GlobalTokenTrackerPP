# REVIEW — 逐步审查记录

## S0 地基 ✅
- Rust 1.98.1 stable-msvc 就位（rustup）；VS2026 Community MSVC 14.51 链接器在位。
- WinAppRuntime 1.5/1.6/1.7/1.8/2.4/2.5 全部已装 → 走 framework-dependent 部署。
- 依赖全部取最新版：rusqlite 0.40.2、serde_json 1.0.151、jiff 0.2.37、notify 8.2、rayon 1.12、sha2 0.11、windows-reactor =0.100.0（钉版防 0.x churn）、tray-icon 0.25.1。

## S1 存储层 ✅（8 单测绿）
- schema 按 spec §5 全 8 表 + WAL；UPSERT 用 `completeness` 裁决分实现"终态覆盖快照、快照不覆盖终态"（避开 cc-switch #6994）。
- 游标：sha2-256 尾指纹；截断/改写 → 钉 EOF 绝不重放（测试覆盖：正常 append、截断、同长度改写前缀均被检测）。
- 新增 `sync_cursors.adapter_state` 列（v2 迁移）存适配器私有续扫状态——为 codex 差分/去重需要跨扫描记忆。
- review 发现已修：`query.rs` 命名参数混位 bug（strftime 内联校验 offset 解决）。

## S2 Claude 适配器 ✅ 验收门过
- **实测口径修正**：spec 说"按 message.id 去重"，实测需**全局去重**（非 per-file）——resume/fork 会话把同一 message.id 写进 862 个不同文件；per-file 口径多算 6.75%。已改为 `claude:{msg_id}`。
- 验收：`ΔcacheR=0.09%`（gate <1%，spec 基线 0.1%）、`Δout=0.40%`、ev 11,113 vs cc 11,185（差值=cc 库中已删文件残留+新增）。
- `cache_creation.ephemeral_{5m,1h}` 拆分已按真实字段实现；无拆分时全计 5m。

## S3 Codex 适配器 ✅ 口径修正（重要）
- **spec 的 `last_token_usage` 增量法实测会多算**：token_count 行存在逐字节重复发射（同 cum+inc 重复行 838 处）。但 `total_token_usage` 差分法也不行——compaction 重置 + 并行序列交错会让"重置即全量"的规则反而更高估（6.19B vs 6.04B）。
- **最终口径**：`last_token_usage` 逐调用求和 + 剔除"cum 与 inc 均与上行相同"的逐字节重复行。结果 5.90B；与朴素 Σinc 6.04B 差 2.2% 恰为剔除的重复量。
- **`tokens_used` 语义锁定**（S8 复核，347/347 线程 0 miss）：= rollout 文件**最后一行** `total_token_usage.total_tokens`（会话级水位终值，compaction 后取新累计）；`total_tokens == input+output`（cached⊂input、reasoning⊂output，2930 行全对）。**不是**逐调用增量和。
- **验收门过（0.01%）**：`reconcile` 按 `rollout_path` join `sync_cursors.adapter_state.cum`（=每文件末值）vs `state_5.threads.tokens_used` → joined 333 线程，ours=1,556,625,563 / st5=1,556,810,227，Δ=0.01% << 0.5%。先前整表 4.83% 是覆盖率假象：515 个有状态文件中 182 个在 state_5 无对应线程（已删/archived）。
- 两个口径都有意义：usage_events 存逐调用增量（真实处理量，含上下文重读放大）；水位=会话级 billed 口径（订阅配额以 rate_limits.used_percent 为准，已入 quota_snapshots）。
- cc-switch 的 codex=9.22B 混合了代理通道日志，不可作基准。
- rate_limits → quota_snapshots（used_percent/window_minutes/resets_at/plan_type）已带变更检测；credits.balance 单列。
- `adapter_state` 持久化 PrevLine（inc+cum 签名）使跨扫描去重成立。

## S4 OpenCode + ZCode ✅
- opencode.db 只读打开（mode=ro URI），session 表 → 事件；cost>0 → provider_reported，cost=0 → 计价管线；watermark=time_updated-1s 重叠防竞态。
- zcode model_usage：cross-provider（anthropic/openai/google 前缀）行跳过并计数（本机 skipped=81）；dedup=logical_request_id+attempt_index；watermark=started_at。
- 全量首扫（543 文件，~1.8GB）debug 版 17.7s ≈ 100MB/s；增量扫描毫秒级。

## S5 计价管线 ✅（种子 20KB/2314 模型）
- 内置离线种子：models.dev 全量精简到 `{model:[in,out,cr,cw]}` gzip=20KB → 零网络可计价。
- 别名 BFS：rsplit('/') → ':'/('@'→'-')/小写/'[1m]' → openai./anthropic./moonshot./bedrock./global. 前缀剥离 → rfind('claude-') → -v数字/-YYYYMMDD/-effort 后缀 → 前缀匹配（带 dash 门槛）。
- unpriced 不猜价：cost=0+cost_source='unpriced'（本机 codex-auto-review、gpt-reserve、custom-alpha 等别名会命中）。
- **已知偏差**：claude 侧 Δcost=5.51% vs cc-switch——两家价目表不同源（我们 models.dev 种子 vs 其内置 218 表），spec 明确成本一律"估算"，在可解释范围；后续 M2 用 OTel official 成本校准。
- **订阅美元列示注意**：codex/plus 等订阅制的 computed $ 是"若按 API 计价的等值"，UI 需与 credits/百分比分列（spec §7.4）。

## S6 CLI ✅ `scan | report [today|week|month|all] | reconcile | sources`

## S7 WinUI3 壳 ✅（reactor 0.100.0 实测）

- **路线验证**：纯 Rust + WinUI3 窗口运行成功，解包模式自动 bootstrap（WinAppRuntime 2.5.1 命中），Mica 背板生效。
- **API 偏差记录**（docs.rs 示例 ≠ 本地 0.100.0）：
  - 无 `window_frame`/`menu_items`/`content` 便捷方法 → 用 `context.window_title` + `SlotsControl::slot/slots` + `SlotView::collection`。
  - 动态子集必须 `keyed_children(KeyedView)`（`children` 只收静态元组/数组）；`IntoViews` 不接受 `Vec<View>`。
  - `Button.content()` 消费自身返回 `View` → `on_click` 必须先调用。
  - `update(ctx)` 签名是 `&ComponentContext`（非 `&mut`）；`ViewContext::message` 要 `Msg: Clone` → 用 `callback(move |_| Msg)`。
  - 无 `font_family` 等字体 API（0.100.0）→ 换字体暂不支持，`ThemeConfig.font_family` 字段预留占位。
  - `Brush` 是 Copy；`Thickness`/`CornerRadius` 非 Copy → Theme 存 f64 现造。
- **NavigationView 在 0.100.0 解包模式下构造即 stowed crash（0xC000027B）**：二分确认纯 `slot(Content)` 也崩 → 改用 **SelectorBar** 顶部 pill 导航（WinUI Gallery 同款，更简约）。后续可换 Pivot/TabView 或等上游修复。
- **真机验证**：窗口标题/尺寸正常，5 页全部渲染，45s 长跑过 ≥1 个 30s 自动刷新周期无崩溃；截图人工核过视觉效果。
- **panic 诊断位**：`cl_panic.log`（stowed 异常无 stderr，panic hook 落盘）。

## S7b 可扩展 UI 架构 ✅

- **主题令牌**：`theme.rs` — `ThemeConfig`（JSON 覆写）→ `Theme`（解析后 Brush/度量）。颜色支持 8 种命名主题刷（随系统明暗）+ `#rrggbb/#aarrggbb` 纯值换肤；`warn`/`ok` 默认 Fluent 黄/绿（主题刷无对应）。
- **布局配置**：`config.rs` — `ui.json`（与 ledger.db 同目录）持久化每页 `order`/`hidden`；`order_for` 容忍注册表新增部件。
- **手动排序**：总览页"布局"开关进入编辑模式 → 每部件带上移/下移/隐藏按钮条，隐藏项以 chip 陈列可恢复。
- **部件注册表**：`widgets.rs` `OVERVIEW_WIDGETS`（id/title/icon）—新增部件 = 一条注册 + 一个 match 臂。
- **线条+图标划分**：`section_header`（SymbolIcon + 标题 + 拉伸发丝线）、`key_value_row` 行间发丝分隔（`line_separators` 可关）、卡片可选 `accent_edge` 左侧描边条、卡片间 `gap`/`section_gap` 令牌化。
- **克制强调**：`badge()` 描边药丸（Accent/Warn/Danger/Muted 四档）仅用于状态信号——成本卡"估算"徽章+accent 值色（唯一排版强调点）、配额 ≥50% 出徽章（≥80 红/≥50 黄）、数据源异常红徽章、明细 unpriced 黄徽章。正常态一律素文本。
- **残留限制**：reactor 无 font-family setter（换字体待上游）；SymbolIcon 无 foreground 着色；拖拽排序未做（先按钮排序，drag-drop 样例存在但复杂度高留 M2+）。

## S8 常驻能力 ✅（live watch + 托盘）

- **`SourceAdapter::watch_roots()`**：各适配器上报顶层监听根（claude `~/.claude/projects`、codex `sessions`+`archived_sessions`、opencode `~/.local/share/opencode`、zcode `~/.zcode/cli/db`），`Engine::watch_roots` 聚合去重 + `is_dir` 过滤。
- **`watch.rs`**：`notify` 递归监听 → 防抖（首个事件后 drain 1.2s，总防抖上限 8s 防持续写入饿死刷新）→ `Msg::WatchFired` → 增量扫描 → 重新武装。30s 定时器保留作兜底。
- **修了两个真 bug**：
  1. `pending_rescan` 分支里 `scanning` 未复位 → `start_scan` 守卫拒扫 → 刷新链整体停摆（30s 定时器也死）。
  2. **watch 自激循环**：只读打开 SQLite 也会更新 `*-shm`（WAL 共享内存锁文件）→ 每次扫描触发新事件 → 无限重扫。过滤 `*-shm`/`*-journal`/`*.tmp` 解决；`-wal` 保留（真实写入先落 wal，不漏信号）。端到端验证：新文件写入 → 1 次 WatchFired → 1 次增量扫 → 游标落库。
- **`tray.rs`**：系统托盘（程序化 32px 圆角图标+柱状字形，零资源文件），左键/双击/「显示」→ `FindWindowW+SetForegroundWindow` 聚焦（`AllowSetForegroundWindow` 解锁后台激活），「退出」→ `WindowRef::request_close`；tooltip 随每次 Loaded 刷新"今日 X tok"。
- **限制记录**：reactor 0.100.0 `WindowRef` 无 hide/minimize → 托盘是启动器+状态牌，不能"最小化到托盘"（待上游 API 或自管 HWND）。
- 诊断：`GTT_DEBUG=1` 才开 stderr 日志；`GTT_NOTRAY` 跳过托盘安装。
- ui.json 持久化已实测：覆写 accent=#a371f7 + 部件重排/隐藏，重启后精确生效（截图核对）。

## P1 适配器扩展 ✅（Grok + WorkBuddy；Qoder 有意延后）

- **Grok**（`adapters/grok.rs`）：`~/.grok/sessions/<urlenc-cwd>/<uuid>/updates.jsonl`，收 `method=session/update` + `sessionUpdate=turn_completed` 行；驼峰 usage 字段 + `costUsdTicks` → `CostSource::ProviderReported`（保留官方报账不双算）+ `apiDurationMs` → `duration_ms`；`timestamp` 为 epoch **秒**。项目名取目录段 `pct_decode`。
  - 实测：7 文件 / 33 events / $11.2250 provider-reported；python 独立重算逐字节一致。
  - 抓过一个 bug：needle `b"session/update"` 15B 却写死 `windows(16)` → 全跳过；改 `windows(needle.len())`。首次空扫已钉 EOF 游标 → 清游标重扫恢复。
- **WorkBuddy**（`adapters/workbuddy.rs`）：`~/.workbuddy/projects/**/*.jsonl`（含嵌套子代理目录）+ `workbuddy.db`。
  - JSONL 收 `providerData.rawUsage`：`prompt/completion/reasoning/cached_tokens` + `prompt_cache_hit_tokens`/`cache_read_input_tokens`/`cache_creation_input_tokens`/`prompt_cache_write_tokens` + `credit`（订阅额度，独立于 USD）。
  - 去重键 `workbuddy:{messageId}`——本机验证 4819 个 messageId 全局唯一、0 重复。
  - `session_usage` 表（session_id/used/size/credit_json）→ `QuotaSnapshot`（window_kind=`session_ctx`，account=None 取最新即"最近活跃会话水位"）。
  - 实测：53 文件 / 4,819 events / 39 quota rows / credits Σ15000.46；python 抽验行数一致。
  - 抓过一个 bug：`&[u8].contains(...)` 是元素查找不是子串匹配 → `windows(needle.len()).any(...)`。
- **Qoder 延后**：`~/.qoder/logs/sessions/**/segments/*.jsonl` 86 文件全是生命周期记录（session.config/route/phase/hook），**本机无 token/credit 用量字段**；其 credit API 需 cookie 鉴权（本机无凭据）。为保 provenance 真实性不造假数据 → 挂到 M3 API/cookie 集成。

## M4 聚合/导出/清理 ✅

- **`rebuild_rollups(offset)`**：`daily_rollups` 全量重建，事务内 `DELETE+INSERT`（派生数据，幂等且时区变更自愈）；本地日期边界用与 `daily()` 相同的 `strftime('%Y-%m-%d', ts/1000,'unixepoch','{offset}')` 口径——offset 校验后内联（防注入）。
- **自动联动**：`Engine::scan_once`/`scan_source` 末尾 `events_ingested>0` 才重建；rollup 失败仅 warn 不阻断扫描（派生数据可重建）。
- **CLI 三命令**：`rollup`（重建+报告行数）、`export [span] --out`（16 列 CSV：本地 ISO 时间戳/模型/项目/五类 token/credits/cost_usd/cost_source/duration/raw_ref，字段级引号转义）、`prune --keep-days N [--vacuum]`（**先重建 rollup 再删明细**，聚合长期趋势不受明细清理影响；`--vacuum` 跑 wal_checkpoint+VACUUM）。
- **测试**：+3（日期边界：23:30Z 事件在 +08:00 下滚入次日且旧日行被清、幂等两次重建行数一致、prune 保留 rollup）。
- **真机验证**：rollup 180 行，与 `usage_events` 原始总量逐字段相等（52,797 ev / 1.67B in / 35.36M out / $7,311.96）；export 160 行 16 列 CSV python 解析无误；`prune --keep-days 36500` 链路跑通删 0 行（破坏性路径未对真库执行）。

## M1b Direct2D 趋势图 ✅

- **`windows-canvas` reactor 集成**：`windows-canvas = { features = ["reactor"] }` —— `canvas()` 按需绘制 `View`（`GpuDevice::new_or_warp` 自动回退软件渲染），挂在 `SwapChainPanel` 上，解包模式正常运行。
- **图表实现**（`trend_strip` 重写）：圆角柱（当日全 alpha、历史 0.45 形成层级）、50% 虚感中网格线 + 发丝基线、DirectWrite 画最大值标签与首/末 `MM-DD` 日期刻度；0 值日画 1.5px 占位线不消失。
- **顺带解锁字体配置**：`theme.font_family` 喂给 `TextFormat::new(family, size)`——reactor 0.100.0 无 XAML font setter，D2D 文字是当前唯一可换字体的面（ThemeConfig 注释已更新）。
- **皮肤联动**：`Theme` 新增 `accent_cf/subtle_cf/divider_cf`（`ColorF`）——hex 配置精确映射；命名主题刷无 RGB 可读回，回退 Fluent 常量（accent 默认 #76B9ED Win11 暗色 accent）。
- **真机验证**：截图确认柱形/标签/刻度正常渲染，进程长跑含 watch 触发扫描后重绘无崩溃。
- **限制**：需求驱动绘制（数据快照随 view() 重建重绘）；无 tooltip/hover（D2D 画布不产 XAML 命中测试，悬停明细留待后续交互层）。

## M2 配额与 OTel ✅ + 收尾项

- **最小化到托盘** ✅：`WindowRef` 无 hide 走 Win32 `SW_HIDE`/`SW_RESTORE` 绕行；托盘菜单新增"隐藏到托盘"，左键/「显示」恢复。真实窗口隐藏-恢复链路成立。
- **趋势图悬停** ✅：`TrendHandle{shared: Rc<TrendShared>, inv: Invalidator}`——Border 包 canvas 收 `on_pointer_moved/exited` → Msg 回环写 `Rc<Cell>` → `invalidate()` 只重绘不重渲染；悬停柱全 alpha + 描边 + 右上 `MM-DD · tokens` 明细。
- **OTLP 接收器** ✅（`otel.rs`）：`127.0.0.1:4318` `POST /v1/metrics`，std::net 极简 HTTP/1.1（无 tokio/axum，头 8KB/体 8MB 上限，10s 读超时）；**专用 OS 线程**不占 reactor 池；`otel_metrics` 表 `(metric,session_id,attr_sig)` 原位 upsert——累积序列重复推送不双计；官方指标与价目估算分列（不进 usage_events）。curl 实测 200 + 落库正确。
- **Claude OTel 引导** ✅：`globaltokentracker otel-setup` 合并写 `~/.claude/settings.json` env 块（serde_json `preserve_order` 保住 cc-switch 键序）；实测既有键全保留。
- **配额轮询器** ✅（`quota.rs`，tokcat 源码级复核）：
  - Codex wham `GET /wham/usage`：Bearer + `ChatGPT-Account-Id`；`last_refresh>8d` 或 401/403 触发 OAuth 刷新回写 auth.json；primary/secondary 按 `limit_window_seconds` 分 `5h_block`/`weekly`；实测 **weekly 0% + reset 命中真数据**。
  - Cursor `POST api2.cursor.sh .../GetCurrentPeriodUsage`（Connect RPC JSON，`Connect-Protocol-Version:1`）：`planUsage` lenient 双形数字、cents→USD、auto/api 池分行；实测 3 行落库。
  - UI 30 分钟低频门（`GTT_NO_QUOTA` 关），CLI `globaltokentracker quota` 手动。UI 实测 4 行 0 错。
- **仍 blocked（如实）**：Qoder（`.auth` 仅 machine_id，无 cookie/token——留手动粘贴入口到 M3）、Claude oauth（`.credentials.json` 只有 mcpOAuth 无 claudeAiOauth）、CodeBuddy/Gemini（本机无数据文件）、wham 每日明细端点（`daily-token-usage-breakdown` 未接，窗口信号已够配额页用）。

## 待办（S9+）
- [ ] Qoder：cookie 手动粘贴入口 + credits API（M3）
- [ ] Claude oauth usage：等本机出现 `claudeAiOauth` 凭据（Claude Code 登录态）
- [ ] CodeBuddy/Gemini 适配器（本机无数据，待真实文件出现）
- [ ] wham `daily-token-usage-breakdown` 明细端点（可选增强）
- [x] ~~托盘最小化~~ → Win32 SW_HIDE 绕行成立
- [x] ~~趋势图悬停~~ → Invalidator+共享 Cell，只重绘不重渲染
- [x] ~~OTel + 配额通道~~ → OTLP 4318 + wham + Cursor RPC 全部实测落库

## 兼容性与性能审计（release，2026-09-27 实测）

| 指标 | 实测值 | 判定 |
|---|---|---|
| 二进制体积 | UI 8.4MB / CLI 6.8MB（release 默认 strip） | ✅ 达标（<10MB 目标；bundled sqlite+rustls+D2D 占了大部分） |
| 启动→窗口可见 | ~2.6s（首扫在后台线程，不阻塞首帧） | ✅ |
| 常驻内存 | 118.7MB working set / 107.3MB private（15s 后） | ✅ WinUI3 基线内（XAML 框架本身 ~60-80MB） |
| 增量扫描 | 176ms / 603 文件可见 / 3 实际重扫 | ✅ 秒级以内 |
| rollup 重建 | 180 行瞬时（52,797 明细全量重聚合） | ✅ |
| 账本体积 | ledger.db 39.2MB（52.8K 事件 + 19K 配额行） | ✅ prune 通道已备 |
| 线程数 | 123（reactor 池 + rayon 全核 + watch/otel/tray 各一） | ⚠️ 偏高但合理：reactor/rayon 按需休眠线程占大头 |

**兼容性结论**
- **Win 下限**：WinUI3 需 Win10 1809+（build 17763）；WinAppRuntime 需 1.5+/2.x 之一在位（framework-dependent 部署，本机 1.5–2.5 共存验证）。
- **GPU**：D2D `GpuDevice::new_or_warp`——无 GPU/老显卡自动落 WARP 软件渲染，图表照样画。
- **mac 迁移成本**：core 零 Windows 依赖；`tray.rs`/`quota.rs` 已按 cfg/dirs 做多平台分支（Cursor state.vscdb 走 `dirs::config_dir`/`data_dir` 跨平台查找）；ui 壳整体重写是唯一大头，ViewModel/Theme 令牌可移植。
- **网络面**：otel 仅绑 127.0.0.1（不暴露 LAN）；quota 出站仅 chatgpt.com/api2.cursor.sh 两个固定端点，rustls 校验。
- **降级路径**：端口被占→文件源照常；凭据缺失→该通道静默跳过；API 改版→单通道 error 不影响其余（`PollOutcome` 隔离）。
- [x] ~~Grok/WorkBuddy 适配器~~ → 过，真机数据逐字段复核
- [x] ~~daily_rollups/CSV 导出/prune~~ → M4 完成
- [x] ~~Codex 验收门~~ → 过，Δ=0.01%（tokens_used=会话水位终值语义锁定）

## S10 联网价目同步 ✅

- **需求**：价目表此前只有内置种子，不能联网更新——改为双源拉取 + 自愈回填。
- **实现**（`pricing/mod.rs`）：
  - `refresh()`：models.dev `api.json` + LiteLLM `model_prices_and_context_window.json` 两源**独立容错**——单源挂不影响另一源，双挂才报 Err 且库表不动；单事务落库。
  - `prices(provider,model_id)` PK 按 source 分槽（dev/litellm 行互不覆盖）；`load()` 读取侧 `ORDER BY CASE source seed<litellm<dev` 保证 dev 官方价压过 litellm 变体价与种子。
  - LiteLLM 的 `$ /token` 统一 ×1e6 转 $/1M；tier 三列（>200k 输入/1h 缓存写/批折扣）落库，与 `compute()` 的长上下文/缓存写路径衔接。
  - `fetched_at` 记同步时刻；`prices_stale()` 24h TTL。
  - `reprice_unpriced()`：刷新后回填 cost_source='unpriced' 的历史事件——新模型入库自动获得估算价；`auto` 模型与 prefix 命中保持 `estimated` 语义；无价模型（gpt-reserve/codex-auto-review/自定义名）正确保持 unpriced，绝不猜价。
- **接线**：UI `load_all` 扫描后检查 24h 陈旧自动刷新（不阻塞首帧，失败仅 diag）；CLI `prices [--update]` 查新鲜度/手动同步；价格页头部显示"联网同步于 X 小时前 / 仅本地种子"。
- **实测**：真联网 models.dev 7750 行（PK 归并后 2089）+ litellm 3623 行（1952），reprice 0（本机 5389 unpriced 全是内部/自定义模型，符合预期）；UI 截图确认新鲜度行与 source 列混合展示。
- **测试**：+3（reprice 回填、override>dev>litellm>seed 优先级、陈旧判断+幂等）；15/15 绿，clippy 0。
- **已知分歧**：gpt-5.5 dev $5/$30 vs litellm $2.5/$15（litellm 含 azure/codex 变体价）——dev 优先策略已锁定。

## S11 单文件安装器 ✅

- **需求**：安装文件必须是一个 exe。
- **实现**：新 crate `globaltokentracker-setup`——`include_bytes!` 内嵌 `payload.zip`（build.rs 兜底 22B 空 zip，dev 构建不受影响），运行时 zip-deflate 解压到 `%LOCALAPPDATA%\Programs\GlobalTokenTracker`。
- **安装动作**：WinAppRuntime 检测（`Get-AppxPackage Microsoft.WindowsAppRuntime*`）→ 缺失则提示下载微软官方 aka.ms 安装包（ureq+rustls）→ taskkill 旧实例 → 释放文件 → 复制自身为卸载器 → WScript.Shell 快捷方式（powershell COM，免引 windows crate COM 面）→ HKCU `Uninstall\GlobalTokenTracker` 注册（DisplayVersion/EstimatedSize/QuietUninstallString）→ 用户 PATH 追加（去重）。
- **卸载**：`--uninstall [--dir]`——杀进程、删快捷方式、删 HKCU 项、PATH 回滚、detach cmd `rmdir` 自删目录（程序运行中 exe 不可删 → 延迟 cmd）；用户数据 `~/.globaltokentracker` 明确保留。
- **踩过的坑（实测抓出）**：
  1. `Command::arg()` 对 cmd `/C` 字符串做 `\"` 转义 → cmd 读到字面 `\"` 解析失败，自删静默不执行——改 `raw_arg()` 原样透传。
  2. UninstallString 原先不带 `--dir`——自定义目录安装后走注册表卸载会清错路径——始终显式携带。
- **产出**：`installer/package.ps1` 一条命令：release 构建 → Compress-Archive 打 payload → 嵌包构建 → `dist\GlobalTokenTracker-Setup-<ver>-win-x64.exe`（**8.78MB**）+ SHA256。
- **实测**：默认路径 install/uninstall 全链路、带空格 `--dir` 路径 install/uninstall（注册表串直接复用验证）、快捷方式/PATH/注册表落点逐项核对、CLI 安装后报表出真数据。
- **边界**：WinAppRuntime 安装步骤本身可能弹 UAC（微软安装器行为，非我们可控）；`--quiet` 自动接受运行时安装；dev 桩 exe 拒绝安装并提示走 package.ps1。

## 审计增量（S10+S11 后，2026-09-27 复测）

| 指标 | 实测 | 判定 |
|---|---|---|
| 安装器体积 | `GlobalTokenTracker-Setup-0.1.0-win-x64.exe` 8.78MB（双 exe deflate 压缩） | ✅ |
| 联网价目刷新 | 2.4s（dev 7750 行 + litellm 3635 行 + 落库 + reprice 扫描），仅 UI 启动时 24h 陈旧时后台跑 | ✅ 不阻塞首帧 |
| UI 启动/内存 | release 正常起窗，121.7MB RSS 与上轮一致（刷新在 load_all 内非阻塞） | ✅ 无回归 |
| 新增出站端点 | models.dev + raw.githubusercontent.com（固定 HTTPS，ureq/rustls） | ✅ |
| 卸载完整性 | 自删目录含运行中 exe（raw_arg/cmd 延迟 rmdir）、注册表/PATH/快捷方式全清、带空格路径验证过 | ✅ |
| 供应链 | zip 8.6.0（2026-04-25）/ winreg 0.55 / ureq 3.4.2，均远超 7 天沉淀 | ✅ |
| 测试 | 15/15 绿，clippy 全工作区 0 警告 | ✅ |


## S12 统计范围选择 + 精确数字 ✅

- **需求**：统计时间尺度可选；token 显示精确数字不再用 M/B 缩写。
- **`Range` 枚举**（viewmodel）：`今日/近7天/近30天/全部`，`key/label` 双向映射，`ui.json` 持久化（`"range": "week"` 默认）。
- **`overview(range)`**：span totals + by_app + 趋势序列全部按范围出数；`今日` 粒度不足一天 → 新增 `hourly()` 按本地小时分桶（strftime '%H:00'）；`今日` 以外按天。`today`/`all` 保留——托盘 tooltip 与"全部 $X"对比副标仍要全日口径。
- **UI**：总览头部 SelectorBar（复用导航同款控件）→ `Msg::SetRange` → 写回 config + 走标准扫描链重载（索引字段，重载毫秒级）。卡片标题随范围改（"近 7 天 Tokens"等），趋势标题显示粒度（"今日 · 按小时"/"全部 · 按天（近 60 桶）"）。
- **`fmt::tokens_exact`**：千分位精确数字（`1,730,848,235`），全 UI + CLI `report`/`export` 表头统一替换；D2D 图顶标与悬停明细同步换精确值。
- **实测截图**：近7天（4,760 事件/1.73B 精确）与全部（52,874 事件/60 桶趋势/活跃 14m12s）两档渲染均正确；持久化重启后范围保持。
- **测试**：+3（hourly 分桶、tokens_exact 分组、Range key/label 往返）；18/18 绿，clippy 0。

## S13 产品改名 CodeLedger → GlobalTokenTracker ✅

- **全量改名**：crate 包名 `globaltokentracker-{core,ui,cli,setup}`、二进制 `globaltokentracker-*.exe`、窗口标题/托盘 tooltip/快捷方式/HKCU 卸载项（`Uninstall\GlobalTokenTracker`）/安装目录 `Programs\GlobalTokenTracker`/发布包 `GlobalTokenTracker-Setup-*`、诊断环境变量 `CL_*`→`GTT_*`、文档全扫。
- **数据目录迁移**：`default_db_path()` 检测到 `~/.codeledger` 存在且新目录不存在时原地 `fs::rename` 到 `~/.globaltokentracker`——实测 52,874 事件无损迁移，ui.json 同步搬家。
- **踩坑**：
  1. HKCU\Environment 的 `Path` 写入被拦（未签名二进制的 EDR/策略保护，powershell 签名进程可写）→ PATH 追加改**尽力而为**，失败只告警不阻断安装。
  2. `cargo clean -p` 匹配不到改名后的包指纹（"Removed 0 files"）且不清顶层 exe 硬链——package.ps1 改 touch build.rs 强制重嵌 payload。
- **实测**：新名安装（PATH 拒绝优雅降级）→ CLI 报表出真数据 → 卸载目录自删干净；窗口标题、注册表、快捷方式均为新名；包 8.79MB。
- **测试**：18/18 绿，clippy 0 警告。

## S14 明细页表格化 ✅

- **痛点**：旧明细行是单行 `format!` 定宽字符串——无表头、中英混排撑破列宽、每行独立边框视觉碎。
- **实现**：`DETAIL_COLS`（8 列固定 px + 模型 Star 吃余量）逐行 Grid，同定义保证跨行对齐；表头行（subtle+semi-bold+底分隔线）；斑马纹 `argb(10,128,128,128)` 淡灰（亮暗主题都成立）；行发丝底线走 `line_separators` 主题开关；整表收进一张 card。
- **可读性细节**：数字列右对齐（输入/输出/缓存/成本/时长）；模型列淡化处理；成本后缀 `≈`估算/`↺`厂商回报保留，`unpriced` 警示徽章仍在成本列；行 tooltip 仍是 `raw_ref` 溯源。
- **验证**：截图核对——200 行/页真实数据 8 列严格对齐；内存 149MB（明细页大数据集，基线 119MB）；18/18 测试、clippy 0。

## S15 按工具勾选过滤统计范围 ✅

- **需求**：统计不能只看总和，要能勾选只看某个/某些工具。
- **core**：`scope_where(from,to,apps)` 统一 `WHERE` 拼装——`Option<&[String]>`：`None`=不过滤、`Some(list)`=`app IN (?,…)` 绑定参数（不拼字符串）、`Some(&[])`（全不勾）=`WHERE 0` 诚实空集。`totals/by_app/daily/hourly/events_page/event_count` 全挂过滤参数，新增 `app_names()`（checkbox 列表源，`by_app` 被过滤会吃掉候选名故单列）；`overview(range,apps)`/`detail(page,size,apps)` 透传，OverviewVm 新增 `apps` 全量工具名。
- **UI**：总览页头部下 + 明细页各一行 `CheckBox`（reactor 0.100 原生控件），勾选状态存 `ui.json` 的 `apps` 字段（None=全选不落盘）；`Msg::ToggleApp` 重算过滤集合并走正常扫描链路刷新；卡片/趋势/按工具表/明细行全部收窄。
- **语义**：全勾或缺省=全部；只勾 claude 后实测卡片 11,323 事件/4,996,899,992 tok/$2764 与按工具行精确一致；全不勾=空视图（不偷换为"全部"）。
- **验证**：截图核对全选/单选两态；测试 +1（None/subset/empty 三态 + app_names + 分页过滤）19/19 绿；clippy 0。
- **顺修**：panic 日志文件名 `cl_panic.log`→`gtt_panic.log`（改名遗漏）。

## S16 趋势柱悬停浮窗（延迟弹出）✅

- **需求**：柱上悬停片刻弹小浮窗：当日 tokens/价格/事件数/Top3 模型。
- **数据**：新增 `bucket_models()`——桶（日/时）× 模型二维聚合一次取回；`viewmodel` 折叠成 `TrendBucket{date,events,tokens,cost_usd,top[3]}`（按 tokens 降序截 3），`OverviewVm.daily` 升级为该类型（口径沿用旧趋势 input+output+cache_read）。与 app 过滤联动。
- **延迟机制**：`TrendShared{hover,tip,pending}`——换柱才重置 pending 并 `spawn_background` 睡 450ms → `Msg::TrendTip(idx)`，仍悬停同柱才置 `tip` 并 invalidate；同柱内微移不重置计时（“停留片刻”语义）；`TrendLeave` 全清。
- **绘制**：D2D 画布末段画近不透明深色卡片（Fluent tooltip 惯例）：accent 日期头、`tokens·$cost`、`N 事件`、Top3 模型行（20 字截断），锚定柱顶居中并 clamp 进画布。
- **关键坑（记录）**：注入输入（SendInput/SetCursorPos）对 WinUI3 `DesktopChildSiteBridge` 不产生 PointerMoved——无法用脚本做悬停端到端。另发现 `Background=null` 的 Border 不做命中测试，补 `Transparent` 背景（这是真实 bug 修复）。渲染链路用 `GTT_TIPTEST=<idx>` 环境变量强制弹窗截图验证（09-08 桶：197.3M tok/$238/586 事件/Top3 正确）。
- **残留风险**：真实鼠标的 PointerMoved 走同一订阅/分发链（Button Click、SelectorBar 已实证该泵工作），置信度高但未能注入验证——发布前建议真机手动悬停一次确认。
- **验证**：19/19 测试、clippy 0。

## S15b 复查增补

- **复查发现**：持久化过滤集可能残留已从账本消失的工具名（数据被清理后死名阻止 `Some`→`None` 塌缩）——`Msg::Loaded` 里加活数据交集清理，覆盖全部活工具时自动归零。
- **逐项核过**：`scope_where` 占位符编号在 time/app/limit/offset 三段连续无错位；`app` 过滤命中既有 `idx_events_app(app, ts_start)` 索引；`load_all` 明细固定取第 0 页——过滤变化无越界页风险；`app_names` 保持无过滤（勾选框始终可见）；明细页也挂了同一行勾选器。
- **验证**：19/19 测试、clippy 0、工作区无临时文件混入。

## S17 Devin 适配器（SQLite 源）✅ + 兼容性盘点

- **源**：`<data_dir>/devin/cli/sessions.db`（`dirs::data_dir`——Win `%APPDATA%` / mac `~/Library/Application Support` / Linux `~/.local/share`，跨平台口径一致）。
- **数据面（实地验证）**：`message_nodes.chat_message` 每行一条聊天消息 JSON；assistant 节点带 `metadata.metrics{input/output/cache_read/cache_creation_tokens, ttft_ms, total_time_ms, tpot_ms, tokens_per_sec}`——**全适配器里最全遥测**（唯一原生 TTFT/TPOT）。`metadata.request_id` 每次推理共享（同一消息在 tool_call 落地后被重存为 ~2 个快照行，token 指标相同、时延字段后补全）→ `dedup_key=devin:{request_id}` 配完备度 UPSERT 收敛成一条。
- **字段映射**：`generation_model`→model、`working_directory`→project、`started_generation_at/created_at`（RFC3339）→ts_start/end、`finish_reason`→status、ttft/total→ttft_ms/duration_ms。`sessions.metadata.total_acu_cost` 是会话级累计计数器，**刻意不映射**（会重复计费）；本地 backend 恒为 0。
- **性能**：SQL 侧 `json_extract($.metadata)` 只回传小对象（`chat_message` 含 thinking/tool_calls 可达 MB 级）；`LIKE '%"role":"assistant"%'` 做字节级粗筛再进 json_extract。首扫 513MB 库含全量解析 **3.3s**，增量 135ms。水位 = `row_id` AUTOINCREMENT。
- **实测**：4,971 事件落库（10,535 快照行去重收敛），swe-2-max 正确归 unpriced；UI 勾选行/按工具表出现 devin，截图核对。
- **其余目标盘点（实地核查后如实标记）**：Copilot 日志只有进程生命周期无 token；Gemini/Antigravity 装了没用（conversations/ 空）；CodeBuddy 只有 memwatch+空 expert-history；Qoder 只有基础设施日志；Windsurf `.codeium` 全是 protobuf 上下文；Cursor 7,761 条 bubble `tokenCount` 全 0（服务端计费）配额 RPC 已是上限；cli-proxy-api 是代理层不采。
- **测试**：+1（request_id 快照去重/字段映射/水位推进/二次扫描幂等），20/20 绿、clippy 0。

## S18 页面纵向滚动 ✅

- **根因**：根容器是垂直 `StackPanel`——主轴方向给子元素无限高度，`page_frame` 里的 `ScrollViewer` 被量成内容全高，永远没有可滚空间（XAML 经典坑）。
- **修法**：根换 `Grid`（`Auto` 导航行 + `Star` 内容行），内容区被约束进可视高度 → ScrollViewer 生效；滚动条显式 `Auto`（Fluent 惯例：悬停才现细条）。
- **验证路径（记录）**：注入输入（WM_MOUSEWHEEL/SendInput）对 WinUI3 输入岛无效，改用 **UI Automation**——`ScrollPattern.VerticallyScrollable=True`、`VerticalViewSize=68.3%`，`SetScrollPercent(100)` 后截图确认滚到底部（配额卡/未计价警示条可见）。这条 UIA 通道以后还可用于端到端 UI 验证。
- **验证**：build/clippy 干净。

## S19 安装器 GUI（纯 Win32/GDI 深色 Fluent）✅

- **约束**：安装器职责是在没有 WinAppRuntime 的机器上装运行时——**不能依赖 WinUI3/WebView2**，选纯 Win32+GDI 自绘。`windows 0.62.2` + `windows_subsystem="windows"`。
- **界面**：`#202020` 底 / `#60CDFF` accent / Segoe UI / DWM 深色标题栏（`DWMWA_USE_IMMERSIVE_DARK_MODE`）/ Per-Monitor-V2 / `DarkMode_Explorer` 主题化 EDIT+CheckBox；accent 竖条+标题+副标题、发丝分隔线、owner-drawn 圆角主按钮（卸载态红色）与描边副按钮、自绘进度条控件（GWLP_USERDATA 存千分比）、`IFileOpenDialog` 现代选目录。660×430 固定窗。
- **架构**：安装/卸载逻辑拆成 `install_steps`/`uninstall_steps`（`step(pct,label)`+`log` 回调），控制台与 GUI 共享同一代码路径；worker 线程跑活，`SendMessage` 更新状态/进度，`WM_APP_DONE` 切完成态。安装成功主键变"启动并关闭"（`ShellExecuteW` 拉起 ui.exe）。
- **CLI 保持**：`--quiet`/`--uninstall`/`--dir`/`--cli`/`--help` 全保留；`AttachConsole(ATTACH_PARENT_PROCESS)`+`SetStdHandle` 重接 CONOUT$/CONIN$。`--uninstall` 不带 `--dir` 时默认 **current_exe 父目录**（卸载器副本就住在安装目录里）。
- **过程中抓到并修掉的真实 bug**：
  1. `SelectObject(mem, old)` 写在 `BitBlt` **之前**——先把位图换出再 blit 等于从 1×1 stock bitmap 拷贝，客户区全白（截图二分定位）。
  2. **worker 里 `println!` 会 panic**——windows 子系统无控制台时 stdout 无效，写失败 panic 杀线程留下半装状态→GUI log sink 改 no-op（步骤标签+最终错误已够叙事）。
  3. **安装中关窗杀进程=半装**——`WM_CLOSE` 在 `working` 时拦截并提示。
  4. **每帧 WM_CTLCOLOR* 新建画刷泄漏**——画刷预建存 Gui 复用；STATIC 标签回 BG 刷（否则浅色带）、CHECKBOX 走 `WM_CTLCOLORBTN`+NULL_BRUSH。
  5. **卸载自删竞态**——原实现在 steps 里立刻排延迟 rmdir，GUI 窗口还开着时 setup.exe 被自己锁住删不掉留半删→拆出 `schedule_self_delete`，GUI 改到 `WM_CLOSE`（done_ok 才排）、控制台路径退出前排。
  6. **taskkill 挂死**——GUI 子进程 spawn 控制台程序时新控制台分配在本机环境下挂住（杀 cli 的 taskkill 15s+ 不退）；所有子 spawn 加 `CREATE_NO_WINDOW` 后实测秒过。
  7. `--help` 原本什么都不印直接进安装流程；`--dir` 吃掉下一个 flag 的问题一并修。
- **实测（发布包，非桩）**：GUI 安装→3 文件+快捷方式+注册项→完成态→"启动并关闭"拉起真 UI；GUI 卸载→杀运行中 UI→完成态→关窗→**整目录含自身自删干净**；`--quiet` install/uninstall 闭环（自删目录消失）；空载荷桩正确报错并恢复按钮可重试。
- **已知限制**：MinTTY/Git-Bash 等无真控制台环境下 `--quiet`/`--help` 静默无输出（windows 子系统 exe 初始无 std 句柄，AttachConsole 无处可挂——功能正常仅无输出，cmd/PowerShell 控制台中正常）；建议真机手动过目一次窗口。

## S20 UI 去除控制台黑窗 ✅

- **现象**：`globaltokentracker-ui.exe` 是 console 子系统，启动后常驻一个黑色控制台窗口。
- **修法**：`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`——release 走 GUI 子系统无黑窗；debug 保留 console（`cargo run`/`diag!` 开发期输出不丢）。
- **诊断保留**：`diag_console()`——`GTT_DEBUG=1` 时先 `AttachConsole(ATTACH_PARENT_PROCESS)`（终端里跑输出回父控制台），失败则 `AllocConsole`（双击也能调出日志窗），再 `CreateFileW("CONOUT$")`+`SetStdHandle` 把 stdout/stderr 接进真控制台。无 `GTT_DEBUG` 时完全静默。`eprintln!` 对无效句柄只丢错不 panic，安全。
- **验证**：release 构建启动后 `Get-CimInstance` 确认**无 conhost 子进程**，窗口渲染完整（截图核对）；子进程零 spawn（core/ui 无 Command 调用），黑窗来源仅此一处。

## S21 Cursor + Qoder 活动级适配器 ✅

- **实地核查结论**：Cursor 服务端计量——`state.vscdb` 的 `cursorDiskKV` 里 7,761 条 `bubbleId:*` 记录的 `tokenCount` 全为 0/0，`requestId` 仅用户气泡携带；`aiCodeTracking.dailyStats.*` 是行数不是 token。Qoder 全程 `--no-session-persistence`，transcript 不落盘，日志只有运行时元数据。**两边都没有本地 token 数据，一律不虚构**——按 Metadata 级适配器接入。
- **Cursor（Sqlite）**：`cursorDiskKV` 中 `type==1` 的用户气泡=一次提问（64 条），assistant 气泡（type 2，7,697 条）是响应块、灌进来会把事件数放大 ~120× 且无数据——只收用户气泡。`requestId`（空则回退 key 尾段 bubbleId）去重，key 中段提 `composerId` 作 session_id，`modelInfo.modelName`、`createdAt`。水位=`rowid`（key 是 `UNIQUE ON CONFLICT REPLACE`，改写产生新 rowid，更新天然被重扫并被 dedup 收敛）。value 为 BLOB 列，`CAST(value AS BLOB)` 取回 Rust 解析（`json_extract` 对 blob 会报 malformed）。
- **Qoder（Jsonl）**：`~/.qoder/logs/sessions/<group>/<uuid>/segments/*.jsonl`（collect_files 深度 4——三个中间目录层）。每个 segment 文件恰好一行 `session.config.loaded`（86/86 验证）→一个会话事件：`project_root`/`model`/`interactive` 取出行内字段，`duration_ms`=文件首末行 ts 跨度（session 根 phase 不落盘），dedup=`qoder:{file_stem}`（stem 自带 ts+rand+pid 天然唯一），session_id=父目录 uuid。
- **引擎门槛**：`is_billable` 之外为 `Capability::Metadata` 适配器放行 `ts_start` 非空的零计量事件——活动观测本身就是信息；Precise 适配器的严格计费门槛不变。
- **验证**：真机扫描 cursor=64（与 type=1 气泡数一致）、qoder=86（86 文件一一对应）；重扫 +0 零重复；UI 勾选行出现两者，按工具表显示 `cursor · 64 事件 · 0 tok · $0.0000`；24/24 测试、clippy 0。
- **如实声明**：两工具事件为**会话/提问级活动记录**，token 恒 0、成本归 unpriced/estimated，趋势图按 token 绘柱所以不产生柱形——这是数据真相，不是缺陷。若未来 Cursor/Qoder 本地写出真实 token 字段，适配器可直接扩展映射。

## S22 安装器视觉打磨（GDI+ 抗锯齿）✅

- **问题**：标题/副标题行距贴死（矩形重叠 4px）；复选框走系统主题、风格与自绘不统一；按钮/框全是 GDI `RoundRect` 无抗锯齿的锯齿毛边；编辑框 `WS_EX_CLIENTEDGE` 3D 凹边与扁平深色冲突；进度条是 6px 光板直线。
- **修法**：接入 GDI+（系统 DLL 零依赖，`Win32_Graphics_GdiPlus`），`fill_rr`/`stroke_rr`/`draw_check` 三个抗锯齿圆角 helper 统一全部控件词汇——按钮填充+描边、进度条圆头轨道+填充、复选框圆角方框+对勾；编辑框去 3D 边、父窗口 WM_PAINT 里画 1px 圆角描边（聚焦时 accent 色，`EN_SETFOCUS/KILLFOCUS` 驱动重绘）；标题区行距拉开（title 22–54 / sub 58–80）。
- **踩到并修掉的真实问题**：
  1. `GdiplusStartup` 传 `SuppressBackgroundThread: TRUE` 被 GDI+ 1.0 判 `InvalidParameter`——所有绘制静默失败（`GdipCreateFromHDC` 报 `GdiplusNotInitialized`），改 FALSE 后正常（后台空闲线程代价可忽略）。
  2. `BS_AUTOCHECKBOX | BS_OWNERDRAW` 样式位冲突——`0x03|0x0B=0x0B`，AUTO 语义被吞，勾选态 Windows 不维护（BM_GETCHECK 恒 0）→ 复选框改纯 `BS_OWNERDRAW` + Gui 自持 bool，WM_COMMAND 点击翻转 + InvalidateRect 重绘。
  3. `GdiPlus::*` glob 导入的 `Status` 常量 `Ok` 遮蔽 `Result::Ok`——改显式导入列表。
- **验证**：截图逐项核对（标题间距/勾选态 accent+对勾/按钮圆角无毛刺/编辑框描边/进度条圆角药丸）；BM_CLICK 点击复选框实测翻转；真实发布包端到端安装→完成态（进度条补满 100%）→"启动并关闭"。clippy 0、24/24 测试。

## S23 总览工具筛选改下拉浮出菜单 ✅

- **问题**：`app_checks` 把 9 个工具的 CheckBox 平铺在筛选行，窗口内放不下，右侧超出边界。
- **修法**：行内平铺 → `Button` + `Flyout`（`FlyoutPlacement::BottomEdgeAlignedLeft`）。按钮常态只显示摘要：`全部`（filter=None）/ `未选`（Some([])）/ `已选 N/9`；浮层内为逐工具 CheckBox + 分隔线 + `全选`/`清空` 快捷键。新增 `Msg::SetApps(Option<Vec<String>>)` 批量设值（None=全选、Some(vec![])=诚实空选），与既有 `Msg::ToggleApp` 走同一条 `app_filter → config.apps 持久化 → start_scan` 路径。按钮加 `automation_name("工具筛选")`（无障碍 + UIA 可测）。
- **过程中抓到并修掉的真实 bug**：`trend_strip` 的 D2D 绘制闭包在 `days.is_empty()` 时**先于 `ctx.clear` 提前返回**——demand canvas 不重画时旧交换链帧残留，清空筛选后卡片归零但柱形图"幽灵"般还在。改为先 clear 再判空，实测空筛选下图表正确清空。
- **验证（UIA InvokePattern/TogglePattern 端到端，截图逐项核对）**：
  - 关闭态为紧凑药丸 `全部 ▾`（64×29），筛选行不再有右溢出；
  - 浮层弹出含 9 个工具 CheckBox + 分隔线 + 全选/清空，勾选态与 filter 一致；
  - 清空 → `未选` + 全部卡片 0 + 按工具"暂无数据" + **图表清空**（修复后验证）；
  - 单勾 claude → `已选 1/9` + 仅 claude 数据（85.9M tok / $31.11）；
  - 从全选取消 claude → `已选 8/9` + 非 claude 数据；
  - 勾回 → 塌缩回 `全部`（`app_filter` 回 None，配置不落冗余项）；
  - 空选跨重启持久化（Some([]) 复原为诚实空视图）；
  - Flyout 在勾选后保持打开，可连续多选。
- clippy 0 警告、24/24 测试。

## S24 刷新频率下拉选择器 ✅

- **需求**：数据定时刷新原写死 30s（`REFRESH_SECS`），用户要可配的下拉菜单。
- **修法**：
  - `UiConfig.refresh_secs` 持久化（serde 字段默认 + 手写 `Default` 都给 30——derive Default 会得到 0=手动，新装直接不刷新，必须手写）。
  - 选项 `仅文件变更(0) / 10 秒 / 30 秒 / 1 分钟 / 5 分钟`，表驱动 `REFRESH_OPTIONS` + `refresh_secs_of`/`refresh_label` 双向映射。
  - UI 用 `DropDownButton` + `Menu`（→原生 `MenuFlyout`，**点选自动收起**——rich Flyout 会停留，单选场景语义不对，这与工具筛选的多选 Flyout 刻意不同）。与工具筛选并排成 `filter_row`，总览/明细两页共用。
  - `arm_refresh(ctx, secs)`：`0` 时不排定时器（文件 watcher 仍活刷新）；从 0 切回 >0 且不在扫描时立即补臂一个定时器，让新节奏立刻生效而不是等下一次扫描结束。`Msg::SetRefreshSecs(label)` 经 `refresh_secs_of` 反解标签→秒。
- **验证**：UIA 打开菜单（5 项齐全）→ 点 `1 分钟` → 按钮标签即时变 `1 分钟`、菜单自动收起、`ui.json` 落 `refresh_secs:60`；重扫后周期定时器按新值臂。clippy 0、24/24 测试。
- **说明**：`仅文件变更` 诚实语义——关掉的是周期性轮询，watcher 仍会在源文件变化时刷新；要彻底手动则用"刷新"按钮单次触发。

## S25 品牌+导航进自定义标题栏 ✅

- **需求**：系统标题栏的默认图标+"GlobalTokenTracker"文字与内容区的品牌行重复，要求把自绘 logo 挪进标题栏。
- **修法**：用 WinUI `TitleBar` 控件（WinAppSDK 1.7+；安装器引导的是 1.8 runtime，兼容）。框架层把 `TitleBar` 节点自动映射为 `ExtendsContentIntoTitleBar(true)`+`SetTitleBar`，`preferred_height(Tall)`。`Content` 槽放 `[ViewAll 四格图标 + GlobalTokenTracker 字标 + SelectorBar 导航页签]`——品牌与一级导航整体进标题栏，省掉原 header 一整行；内容区顶边补 1px divider 衔接。
- **选型笔记**：曾尝试给 `SymbolIcon` 上 accent 色——本框架的 `SymbolIcon`/`FontIcon` 未暴露 `foreground`/`font_size` setter（生成的绑定只有 `symbol`/`glyph`），保持默认白 glyph，与 Fluent 简约一致。
- **验证**：截图核对标题栏 = 图标+字标+居中导航，caption 按钮（min/max/close）正常；UIA 点标题栏内"明细"→ 选中态与页面切换正常；`window_title("GlobalTokenTracker")` 保留（任务栏标题不受影响）。clippy 0、24/24 测试。

## S26 下拉浮层去系统描边——改自绘内容内叠层 ✅

- **问题**：统一样式后的两个下拉打开时，浮层四边有一条浅亮色描边（`FlyoutPresenter` 的 Fluent 默认 surface stroke + 光晕），在暗色主题下看起来像一圈"奇怪的光线"。框架（windows-reactor 0.100）的 `Flyout` 只暴露 content+placement，没有 FlyoutPresenter 样式/主题资源挂钩，无法覆盖系统描边。
- **修法**：放弃系统 Flyout，改**内容内叠层**——面板作为根 Grid 内容行的最后一个子元素渲染（z 序最高），`Margin` 定位在按钮正下方。卡片本体=不透明底（`page_bg`/`SolidBackground`）+ 半透明卡片色罩层（`card_bg`），描边/圆角复用卡片 token —— 视觉上就是一张抬升的深色卡片，无任何系统 chrome。
- **结构与状态**：
  - 根 Grid 改为 `[Auto titlebar][Auto 筛选 chrome 行][Star 内容]`——筛选行从页面滚动区上移为固定 chrome 行（顺带获得"滚动不丢"的收益），仅总览/明细两页显示。
  - `Shell.open_menu: Option<MenuKind>`（Tools/Refresh）；`Msg::ToggleMenu(kind)` 切换/替换；radio 选中自动收起（`SetRefreshSecs` 里清 open_menu）；`Nav`/`Rescan`/`SetRange`/`DetailPage`/`ToggleEdit` 等页面操作统一收起；`Tick`/后台消息不收起（多选列表需要跨刷新存活）。
  - 按钮 pill 固定宽（工具 104 / 刷新 120），标签定宽 28 → 面板左缘由常量计算（60 / 220），不依赖运行时测量。
- **过程中抓到并弃用的方案**：
  1. `card_bg` 单层做面板 → 它是 Fluent **半透明**层画刷，叠在内容上直接透穿（用户截图可见"透明了"）→ 补不透明底层复合。
  2. `Canvas` 绝对定位 → 弃用，普通 Grid + `Margin` 即可定位，少一层容器。
  3. **Border 指针事件在 reactor 里实际不发**——全透明/半透明 dismiss 层铺满内容区（红底可视化验证覆盖无误）但 `on_pointer_pressed`/`on_pointer_moved` 均不派发；同框架趋势图 hover 也是同一死路（SendInput 真实点击能驱动 CheckBox/Button，唯独 Border 的 Pointer* 路由事件不触发）。放弃点击空白收起，以"操作即收起"替代。
- **验证（UIA + SendInput 端到端）**：工具/刷新两面板均为不透明暗卡片、无亮边；勾选 zcode 实测翻转 + 面板停留多选；radio 选"10 秒"→ 自动收起 + 按钮标签更新 + `ui.json` 落 `refresh_secs:10`；明细页同样正常；放大截图确认四边仅自绘暗描边。clippy 0、24/24 测试。

## S27 模型筛选下拉 ✅

- **需求**：筛选行加第三个选择器——按模型过滤统计与明细。
- **修法**：
  - `scope_where` 加 `models` 维度——WHERE 表达式复用 `MODEL_EXPR`（`COALESCE(NULLIF(model,''),NULLIF(request_model,''),NULLIF(pricing_model,''),'?')`），与趋势桶/明细行的模型口径三处一致；`?` 桶是诚实的"无模型"条目，可勾选。
  - `model_names(apps)`：**模型列表按当前工具筛选级联收窄**，但不被模型筛选自身隐藏（与工具列表同一条 checkbox 不变式）。
  - 查询链路 `totals`/`by_app`/`daily`/`hourly`/`bucket_models`/`events_page`/`event_count` 全部加 `models` 参数；`overview`/`detail` 透传；CLI 传 `None`。
  - UI：`MenuKind::Models` + `Msg::ToggleModel`/`SetModels`（与工具同语义：`None`=全部、`Some([])`=诚实空选、全覆盖塌缩回 `None`）；`config.models` 持久化 `ui.json`；`Loaded` 时 reconcile（剔除消失模型 + 塌缩）。
  - 面板：CheckBox 列表包 `ScrollViewer` `max_height(300)`（真实库 ~40 模型），全选/清空页脚固定不滚；条目 tooltip 放全名、显示截断 36 字符。
  - `ChromeState` 打包 filter 状态传参（`filter_chrome`/`dropdown_overlay` 签名不随选择器数量膨胀）。
- **验证（UIA 端到端）**：面板不透明无亮边、滚动列表生效；取消 `claude-opus-5` → `已选 39/40`、总 tokens 12.4B→8.66B、claude 行 11404→2988 事件；取消 claude 工具 → 模型列表级联剔除全部 `claude-*`、reconcile 后模型筛选塌缩回"全部"；`ui.json` 正确落 `apps`/`models:null`。
- **测试**：新增 `model_filter_scopes_queries`——覆盖 `?` 桶、命名模型、`Some([])`、app×model 相交、`model_names` 级联、detail 行过滤。25/25 通过，clippy 0。

## S28 刷新频率真正接管采集节奏 ✅

- **问题**：用户选了刷新间隔后数据仍近乎实时更新——文件 watcher 绕过 `refresh_secs`，任何源文件写入都触发 `WatchFired → start_scan`，定时周期形同虚设。
- **修法**：watcher 与定时器职责彻底分离——
  - `refresh_secs > 0`（定时模式）：**watcher 完全不起臂**（`create()` 里条件起臂），周期 `Tick` 是唯一刷新源，下一次 tick 做全量 `scan_once`（游标增量解析，不漏数据）；切换瞬间留在飞的 watcher 最多再触发一次，被 `refresh_secs == 0` 门拦住、不续臂，自然消亡。
  - `refresh_secs == 0`（仅文件变更）：watcher 是唯一刷新源，文件事件照常驱动扫描；`SetRefreshSecs(0)` 切换时补臂。
- **收益**：定时模式下活跃的 AI 工具高频写日志不再导致 UI 线程持续被唤醒（每个 WatchFired 都过 update/view），节奏真正由配置值决定；语义自洽——"仅文件变更"字面生效。
- **验证（GTT_DEBUG stderr 实证）**：10s 模式下 touch 受监视文件 → 日志零 watcher 活动，仅周期 `start→loaded`；切"仅文件变更" → `[watch] armed on 10 roots` 补臂 → touch → `fired → scan start → loaded`。
- clippy 0、25/25 测试。

## S29 应用图标全链路落地 ✅

- **需求**：用给定蓝结 logo 做应用图标。
- **资产**：`assets/icon.png`（512px 圆角瓦片，Win11 Fluent 风格 18% 圆角）、`icon-64.png`（标题栏内嵌）、`icon.ico`（16/24/32/48/64/128/256 PNG 帧）——PIL 从源图生成。
- **挂载点 ×4**：
  1. **exe 资源**：`winresource` build-dep，`build.rs` 嵌 `1 ICON`——Explorer/快捷方式/任务栏回退都靠它；
  2. **窗口/任务栏**：`WindowVisuals::icon(path)`→`AppWindow.SetIcon`——**资源 ID 字符串形式对 unpackaged exe 不解析**（实测窗口 ICON_BIG 画出通用占位符），改为 `window_icon_path()` 把内嵌 ico 一次性物化到数据目录再传真实路径——dev/安装形态通用；
  3. **托盘**：`Icon::from_resource(1)`（标准 `LoadIconW` 对 exe 内嵌资源正常解析，实测验证）→ 物化文件兜底 → 程序化 glyph 最后兜底；
  4. **标题栏品牌位**：`SymbolIcon::ViewAll` → `ImageIcon::source_data(EncodedImage::from_static(include_bytes!(icon-64.png)))`——与窗口/托盘同一张图，零运行时文件。
  5. **安装器**：setup `build.rs` 同款嵌资源 + `WNDCLASSW.hIcon = LoadIconW(MAKEINTRESOURCE(1))`。
- **踩坑**：`PCWSTR(1 as *const u16)` 触发 clippy `manual_dangling_ptr`——windows 0.62 未导出 `MAKEINTRESOURCEW`，改 `std::ptr::without_provenance::<u16>(1)`。
- **验证**：exe 提取图标=蓝结瓦片；`WM_GETICON ICON_BIG` 画出蓝结（对比修复前的通用窗格图标）；`LoadIcon(module,1)` 返回蓝结（托盘路径等价验证）；标题栏截图确认品牌位换图。clippy 0、25/25 测试。

## S30 配额面板分组折叠 + CodeBuddy 接入 ✅

- **需求**：配额按同一软件来源折叠、更细区分窗口类型；WorkBuddy 一长串刷屏；CodeBuddy（与 WorkBuddy 同厂商不同产品）未参与计数。
- **配额刷屏根因**：`latest_quotas` 用 `captured_at = MAX(...)` 联结——WorkBuddy `session_usage` 批量更新产生同毫秒时间戳，MAX 命中全部 3995 行快照。修为 `ROW_NUMBER() OVER (PARTITION BY app, account, window_kind ORDER BY captured_at DESC, id DESC)` top-1——每身份键**恰好一行**，不同 account（codex 的 free/plus/prolite）仍是独立配额主体。
- **写入端去重**：`insert_quota` 先查同键最新行，所有用户可见字段（used/limit/pct/resets_at）全等则跳过——增量轮询不再为不变窗口堆历史（存量 11409 行保留，查询路径已正确）。
- **配额页**：`OverviewVm.quota_groups`（`group_quotas` 按 app 折叠 + `quota_kind_label` 中文标签：5h_block→5 小时窗口、session_ctx→会话上下文、credits→剩余点数（`used`=余额语义单独标注）等）；每 app 一张卡，透明底 `Button` 头（名称·N 项配额·worst 徽章·▾/▴），点击经 `ToggleQuotaGroup` 折叠/展开（`Shell.quota_collapsed` 会话态）。行内 account 以徽章区分多账号。
- **CodeBuddy 适配器**（`codebuddy_ide`，独立于 workbuddy）：解析 `%LOCALAPPDATA%/CodeBuddyExtension/Data/<user>/<host>/<acct>/history/<ws>/<conv>/messages/*.json`——assistant 消息 `extra.statsSnapshot` 是**会话级累计计数器**（input/output/cached/cacheWrite/thinking/elapsed/credit），逐消息差分（max 水位）出事件，合计=最终快照=厂商精确值；无快照消息跳过（其 token 已在累计内）。会话目录作 `Sqlite` kind SourceItem，`adapter_state` 存已处理文件名+累计水位，增量 O(新文件)。conv→cwd 经 `*/codebuddy-sessions.vscdb` 映射出 project。实测：31 事件、78.6M tokens 入账。
- **取舍**：`lastStep*` 字段不用——多步轮次只报最后一次调用会漏计；差分把一轮多调用合并为一条事件（粒度换精确总额）。`Capability::Estimate`。
- **验证**：UIA Invoke 折叠/展开往返截图确认；`latest_quotas` 15 行（原 3795+）；测试 29/29（新增 dedup、top-1、分组、CodeBuddy 差分 4 例）；clippy 0。

## S31 llmpricing.dev 第三价目源 + 12h/启动双刷新 ✅

- **需求**：在线计价改用 llmpricing.dev 资源；每 12h 自动刷新一次 + 每次打开软件自动获取一次。
- **源选型**：`https://llmpricing.dev/api/models.json`——静态 JSON、无密钥、CDN、CORS 开放、CC BY 4.0；1970 模型归并自 models.dev + Artificial Analysis + OpenRouter。实抓验证：glm-5.3、deepseek-v4-pro/flash 等此前缺口全部命中。
- **取价口径**：只入账 `reference`（官方牌价，`official` 标记）；reference 缺 numeric input 时 `cheapest` 兜底。不用 cheapest 替代 reference——最低托管价会低估用户真实供应商的成本（诚实估算原则）。`cacheRead` 直通；源无 cacheWrite 字段 → 列存 NULL（诚实缺失，不猜倍率），`compute()` 缺省回退照常生效。
- **加载优先级**：seed(0) < litellm(1) < models.dev(2) < llmpricing(3)——ORDER BY CASE 显式分层，同键 llmpricing 胜出（它是 models.dev 的归并超集，且覆盖用户实际在用的国内模型更全）。用户 `price_overrides` 仍居顶（测试锁定）。provider 列存 'llmpricing'，价格页 source 列如实显示。
- **刷新语义**：
  - `PRICE_TTL_SECS` 24h → **12h**，`prices_stale` 判据从"上次成功 `fetched_at`"改为"上次**尝试** `prices_last_attempt`"（新 `app_state` KV 表，schema 幂等兼容旧库）——否则失败一次后每次扫描 tick 都重拉 ~1.5MB。
  - **每次启动必刷一次**：首次 `load_all(range, apps, models, force_prices=true)`（后台线程，不阻塞首帧），与 12h 周期检查解耦；运行期内每次扫描复核 12h TTL。
- **验证**：CLI `prices --update` 实抓：models.dev 7750 + litellm 3637 + llmpricing 1774 upsert（落库 1741 distinct 归一键）；`app_state.prices_last_attempt` 落戳；glm-5.3 解析得 1.4/4.4（官方价）。新增测试：reference/cheapest 取舍、空报价跳过、key 碰撞优先级、override 仍最高、12h 节流 3 例——**32/32 通过，clippy 0，release 干净**。
- **归属**：CC BY 4.0 数据，源标注 `llmpricing`；`meta.syncedAt` 为上游构建时间，本库 `fetched_at` 记本地抓取时刻，两者语义分开。

## S32 标题栏品牌位固定左上角 ✅

- **需求**：icon + 标题应固定在左上角，不与导航分类项放在一起。
- **现状**：`TitleBar.Content` 槽居中且收缩到内容宽（实测：品牌+导航整条被居中），`LeftHeader` 槽在本版 windows-reactor 未绑定——无法走原生三段布局。
- **修法**：导航独占 `Content` 槽（保持居中）；品牌 `StackPanel`（icon+字标）独立放根 Grid 第 0 行，`horizontal_alignment(Left) + margin(12)` 叠在 TitleBar 上方——后挂载即高层级。
- **取舍**：品牌区覆盖的标题栏像素不响应拖拽（同系统标题栏图标惯例），空白区拖拽不受影响；`collection_slot` 返回裸 `View` 无 `grid_column`——改用 `Border` 包列（此版只居中了 SelectorBar 本体，最终方案不需要）。
- **验证**：截图确认品牌固定左上、导航居中、窗口按钮区正常；clippy 0、32/32 测试。

## S33 加载性能重构 + 价格页表格化 ✅

- **启动**：`pricing::refresh` 从 `load_all` 拆出为独立后台任务——联网价目（~2.4s）不再串行卡在首帧数据前。`load_all` 只回传 `price_due` 标记（首启强制 + 12h TTL 检查），`Msg::Loaded` 后置地 spawn 抓取任务，`prices_refreshing` 门防重入；`repriced>0` 时补一次扫描让新价落进 USD 列。
- **刷新/切页**：`Snapshot.sources`/`prices` 改 `Option` 页面域加载——`price_rows(5000)`（最重查询）此前每个 tick 都跑，现在只在价格页打开时取；数据源页同理。`Nav` 到缺数据的页自动补拉一次。明细页保持常载（分页 LIMIT 本就便宜）。
- **价格页**：定宽文本拼接 → 真表格。列：模型(STAR) | 输入 | 输出 | 缓存读 | 缓存写(各 96px 右对齐) | 来源(110px 徽章)；表头行 + 行底细分割线；数字去尾零（0.1/3/0.0038）；source 徽章 seed=Muted、live=Accent。
- **顺手修的 bug**：`price_rows` 用 `f64` 读可空列——llmpricing 行 `cache_write=NULL` 会让整个查询报错。改 `Option<f64>`。另把价格页改成显示**生效价**：`ROW_NUMBER` 按 `PriceBook` 同款优先级每 model_id 取唯一赢行（seed<litellm<models.dev<llmpricing），5000 行原始库存 → 3445 个有效模型，不再三行同名。
- **验证**：截图确认六列表格+来源徽章渲染（UIA 实测 llmpricing 徽章在位）；32/32 测试、clippy 0。

## S34 配额行紧凑计数 ✅

- **问题**：配额页 `已用 166,090 / 上限 300,000` ——上限全是 `,000` 结尾的大数，千分位尾巴视觉上是无意义噪声。
- **修法**：新增 `fmt::tokens_compact`——<1万原样；≥1万 `X.Y万`（去 `.0`）；≥1亿 `X.Y亿`。配额行 meta 用紧凑值（`6.4万 / 30万`），**精确值放 tooltip**——对账语义不丢（`tokens_exact` 注释本就为对账保留精确值）。`credits` 行余额同样紧凑化。
- **范围**：仅配额行；明细页/总览大数仍精确千分位（用户需对账）。
- **验证**：`tokens_compact` 边界单测（9999/1万/6.4万/30万/17.3亿）；32/32 测试、clippy 0；配额页实测渲染正常。

## S35 价格页表格与明细页同构 ✅

- **需求**：价格页行排布/线条分割对齐明细页。
- **对齐项**：`price_row` 改用明细同款 chrome——padding `xy(10,5)`、`theme.line_separators` 控制底部 1px 分割线、隔行 `argb(10,128,128,128)` 斑马纹；整表（表头+行）包进 `w::card` 卡片；行 tooltip 显示完整 `model_id`（截断 56 字符时的全名回退）。
- 表头本来就是明细同款（`padding(10,6)` + 底部分割线 + label_size 半粗 subtle 标签）。
- **验证**：截图确认卡片包裹+斑马纹+分割线渲染；32/32 测试、clippy 0。

## S36 MiniMax Code / Kimi Code 适配 ✅

- **需求**：MiniMax Code、Kimi Code 均已开源，适配其本地用量统计。
- **源码取证**（官方仓库，非猜测）：
  - `MiniMax-AI/minimax-code`：数据根 `~/.minimax`（旧 `~/.mavis`，迁移后 `.mavis` 变成指向 `.minimax` 的 junction —— `packages/config/src/data-dir.ts` `createCompatLink`）。用量在 SQLite `v2/sqlite/runtime-state.sqlite` 的 **`local_runtime_token_usage`**（`infra/db/schema/usage.ts`）：每行一次 LLM 调用，`id` 自增 PK、`session_id`、`agent_name`、`turn_id`、`model`、`ts`(epoch ms)、`input/output/reasoning/cache_read/cache_write_tokens`、`cost_usd`、`raw`；`local_runtime_sessions` 提供 `project_workspace_dir`/`workspace_dir`。
  - `MoonshotAI/kimi-code`：数据根 `$KIMI_CODE_HOME || ~/.kimi-code`（官方文档 `docs/en/configuration/data-locations.md`）。会话日志 `sessions/<workDirKey>/<sessionId>/agents/<agent>/wire.jsonl`；`usage.record` 记录 = **每次 LLM 调用的增量**（`agent/usage/usageOps.ts` + `human/usage/machine.ts` 直接累加），`usageScope` 只标记调用出处（turn/session），**不按 scope 过滤**，否则非 turn 调用会漏计。`TokenUsage` 仅 `inputOther/output/inputCacheRead/inputCacheCreation` 四字段 —— **无 reasoning 拆分**。`config.update` 记录带 `cwd`；`session_index.jsonl` 提供 sessionId→workDir 兜底。
- **实现**：
  - `minimax_code.rs`：Sqlite 源，`id > 水位` 增量（自增 PK 天然单调）；`cost_usd` 厂商自记 → `ProviderReported`；project 取 `project_workspace_dir` 原值（与 claude/codex 存全路径一致）；dedup `minimax_code:<数据根basename>:<id>`（`.minimax` 与 `.minimax-<profile>` 多 profile 隔离）；`.mavis*` 只在没有任何 `.minimax*` 目录且自身非 junction 时才扫（防迁移后同一文件双读双计）；`local_runtime_token_usage` 表不存在视为"无数据"不报错。
  - `kimi_code.rs`：Jsonl 源，`wire.jsonl` 行解析；`usage.record` → 事件，dedup `kimi_code:<文件>:<字节偏移>`（记录无自身 id，偏移天然稳定）；`inputOther` 本就是非缓存输入，直填 `input_tokens`；project 三级解析：`config.update.cwd`（权威、随段可见）→ `session_index.jsonl` → 缓存进 `adapter_state`（追加段从文件中部开始，看不到头部的 config.update），后解析到的 project 回填同段已入账事件。
  - 两者均 `Capability::Precise`（厂商逐调用精确计量）；`apps::MINIMAX_CODE`/`KIMI_CODE` 独立身份，配额/工具过滤/UI 显示名全链接通。
- **验证**：新增 6 测试（minimax：增量水位/全零行跳过/project join/无表容错；kimi：turn+session 增量混计/project 三段解析/半行 defer）。**38/38 测试、clippy `-D warnings` 0、release 干净**。本机两个工具均未安装 → 无本地实测数据，扫描将自然显示"未安装"。

## S37 Cline / Command Code 适配 ✅

- **需求**：Cline、Command Code 均已开源，适配本地用量统计。
- **源码取证**（官方源，非猜测）：
  - `cline/cline`（VS Code 扩展 + CLI）：用量在 `ui_messages.json`（整文件 JSON 数组、全量重写而非追加——`core/storage/disk.ts`），`<dataDir>/tasks/<taskId>/`。`say:"api_req_started"/"api_req_finished"/"subagent_usage"/"deleted_api_reqs"` 行的 `text` JSON 携带 `tokensIn/tokensOut/cacheReads/cacheWrites/cost`（`shared/getApiMetrics.ts`）。语义实锤：`sdk/…/agent-events.ts` 注释明确 `tokensIn` 是**非缓存输入**（disjoint buckets，`tokensIn+cacheReads+cacheWrites`=总输入），`cost` 是 Cline 自算 USD。模型归属：`task_metadata.json` `model_usage[]` 按 `ts` 就近匹配；`state/taskHistory.json` 提供 `cwdOnTaskInitialization`/`apiProvider`（`legacy-state-reader.ts` 确认 CLI 布局）。
  - `CommandCodeAI/command-code`（npm `command-code`，仓库仅 readme——dist bundle 为事实源）：会话在 `~/.commandcode/projects/<slugify(cwd)>/<sessionId>.jsonl`，首行 `{type:"session",…,"cwd"}`。用量挂在 assistant `message` entry 的 `usage` 字段（`createSessionRecorder` 把每个 `model_request_end` 折进恰好一条 assistant 行）：`{inputTokens,outputTokens,cacheReadTokens,cacheWriteTokens,cacheWriteTokens1h?,costUsd?}`。`estimateSessionCostUsd` 实锤：`inputTokens` **含** cache（`fresh = input − read − write`），`cacheWriteTokens1h` 是总量的 1h 子集（`min(1h,total)`、其余为 5m）。
- **实现**：
  - `cline.rs`：Sqlite-kind（适配器自管水位）——`ui_messages.json` 全量重写，按 `{len,mtime}` 快路径跳过未变文件；冷扫描才计 `deleted_api_reqs`（导入时被删历史的聚合），暖扫描跳过（每请求行已入账、删除消息不退 token）；`api_req_started`/`finished` 按 `combineApiRequests` 同款贪心配对——finished 行胜出，带指标的孤立 started 行 EOF 入账；编辑器宿主枚举 `<config>/*/User/globalStorage/saoudrizwan.claude-dev`（Code/Cursor/Windsurf/Insiders 等全兼容）+ CLI `~/.cline/data`（env `CLINE_DATA_DIR`/`CLINE_DIR` 覆盖）。
  - `commandcode.rs`：标准 Jsonl 字节游标；`session` header 拿 cwd（权威 project），追加段走 `adapter_state` 缓存 → 目录 slug 兜底；侧车文件 `.checkpoints./.prompts./.v2.bak` 按官方 `isSessionTranscriptFileName` 排除。
  - 两者 `cost→ProviderReported`、`Capability::Precise`；`apps::CLINE`/`COMMANDCODE` 独立身份。
- **验证**：新增 9 测试（cline：配对胜出/孤立 started 入账/unchanged 快路径/deleted 仅冷扫/字段缺失容错；commandcode：cache 分层拆分/cwd 缓存接续/slug 兜底/全零跳过）。**44/44 测试、clippy `-D warnings` 0、release 干净**。本机未装两工具 → 数据源页显示"未安装"。
- **已知边界**：cline `subagent_usage`/`deleted_api_reqs` 是聚合行与每请求行并存——按 `getApiMetrics` 同款全部计入（与 cline 自身显示总额一致）；`deleted_api_reqs` 冷扫一次后不再重放，运行中删除消息时旧请求行保留（token 已消耗不退账，语义更准）。

## S38 浮点显示去尾零 ✅

- **需求**：明细页大量条目成本显示 `$0.0000`（千分位整数+4位定宽小数），尾零无意义。
- **根因**：`fmt::usd` 对 <$1 固定 `{:.4}`——`$0.5000`/`$0.0100`/`$0.0000` 全带尾零；同类还有 `{c:.1}cr`→`"5.0cr"`、`{pct:.1}%`→`"45.0%"`、`duration`→`"2.0s"`。
- **修复**：新增 `fmt::trim_f(v,prec)`——按 prec 定宽后去尾零/尾点，**prec=0 跳过 trimming**（否则 `"180"`→`"18"`）；`usd` 按量级选 0/2/4 位再过 `trim_f`：≥$100 整元不变，≥$1 `"$45.20"→"$45.2"`，<$1 `"$0.0000"→"$0"`、`"$0.0038"` 有效精度保留；`credits`/`used_percent`/`worst_pct`/`duration` 同步走 `trim_f`；`tokens_compact` <1万分支改走 `tokens_exact` 保千分位（`9999`→`"9,999"`）。
- **验证**：UIA 实测明细页成本列 `$0.0000`→`$0`；新增 2 测试（`trim_f` 边界含 prec=0 保整数零、四舍五入越界 `0.00004`→`"0"`；`usd` 各量级）；**46/46 测试、clippy `-D warnings` 0**。

## S39 导航栏全窗居中 ✅

- **需求**：标题栏导航（总览/明细/配额/数据源/价格）视觉不居中。
- **根因**：`TitleBar.Content` 槽的居中区域**不含右侧系统按钮（min/max/close）占位宽**——导航在"去掉按钮区"的空间内居中，相对全窗偏左。
- **修复**：导航与品牌位同一模式——挪出 Content 槽，作为 row 0 浮层 `HorizontalAlignment::Center + VerticalAlignment::Center` 跨全窗宽居中；TitleBar 退化为纯拖拽区/系统按钮宿主。UIA 实测窗口中心 918 = 导航中心 918（像素级正中）；点击价格项切换+懒加载正常。
- **验证**：46/46 测试、clippy `-D warnings` 0、release 干净。

## S40 账本快照备份 + 损坏自愈 ✅

- **需求**：统计层建一个"大小合适"的缓存——工具本地计数数据被清空后不丢历史统计；要求高效 + 安全。
- **前提确认**：事件一旦入账即独立存活，工具删源数据本就不影响已存统计——真正的单点故障是 `ledger.db` 自身丢失/损坏（46MB、60K 事件）。备份即补这块。
- **实现**（`store/mod.rs`）：
  - `backup_now`：`VACUUM INTO backups/ledger.tmp` → 轮换 `ledger.db`→`ledger.prev.db` → tmp 原子落位。VACUUM 产出**压缩 + 完全 checkpoint 的独立 db**（journal_mode=delete），实测 46MB→44MB，单趟 C 级拷贝 ~几十 ms；tmp+rename 保证崩溃不留半文件，旧代始终可用。
  - `maybe_backup`：`app_state.backup_last_at` 节流 24h，engine 每轮扫描尾部调用——平时成本=一次 KV 读；首次扫描立即建首份快照。备份失败只 warn 不挂扫描。
  - `Store::open` 自愈：db 缺失或打不开/迁移失败 → `restore_backup` 按 `ledger.db`→`ledger.prev.db` 顺序尝试；替换前**先删 `-wal`/`-shm`**（残留 WAL 会重放到恢复副本上再次报错）；损坏原件改名 `.db.corrupt` 保留取证，不静默丢。
  - `Store` 新增 `path: Option<PathBuf>`——`:memory:` 测试库跳过全部文件逻辑。
  - 体积上界 = 2 代 × 压缩后大小（当前 ~44MB × 2 ≈ 88MB）。
- **验证**：新增 3 测试（删库自愈+数据完整、损坏库自愈+`.corrupt` 留存、两代轮换+节流+memory 跳过）。CLI `scan` 实测生成 `backups/ledger.db`：`PRAGMA quick_check` ok、60,179 事件、独立 delete-journal 文件。**49/49 测试、clippy `-D warnings` 0**。
- **取舍**：restore 只覆盖"文件缺失/打开失败"路径；打开正常但页级深损的场景不做启动时 `quick_check`（46MB 全扫每次打开太贵），VACUUM 失败会以错误暴露并在备份目录留下完好旧代——恢复窗口仍由 prev 代兜底。

## S41 全面性能审计 + 空闲刷新零成本化 ✅

- **需求**：全面 review 性能、启动速度及其他问题。
- **实测基线**（本机 754 源文件 / 60K 事件 / 46MB 账本，debug CLI）：
  - 单 tick 全扫 1.4s → **`codebuddy_ide` 单适配器独占 893ms**（57 个会话目录）；codex 36ms（515 文件 stat）、qoder 10ms，其余 ≤6ms。
  - 冷启动（release）：窗口出现 1365ms，首个数据帧 1583ms —— `load_all` 路径 ≈220ms（Store::open ~10 + PriceBook::load 9 + scan ~170 + 聚合查询 ~50）。
  - 聚合查询（60K 事件）：totals_all 27ms、totals_range 28ms、bucket_models 55ms、by_app 25ms、latest_quotas/price_rows <1ms——单条都不慢，**问题是每个空闲 tick 全跑一遍还重建整棵视图树**。
- **修复一（codebuddy 热点）**：`session_dirs()` 原在 `scan_sqlite` 内**每个会话重跑**（57×全目录遍历+开 sqlite+ItemTable 全扫）→ 改为适配器实例级 `Mutex<Option<…>>` 缓存，每 tick 只算一次；锁在取 project 后即释放，不跨消息扫描；`files` 空/无新增时跳过 `save_cursor`。结果 **893ms → 49ms**，总扫描 **1.4s → 274ms**。
- **修复二（空闲 tick 零视图成本）**：`load_all` 返回 `LoadOutcome::{Fresh, Unchanged}`——扫描 `events_ingested==0 && quotas==0` 且未强制时**跳过全部聚合查询与整树重建**，只回传 `scan_ms`/`price_due`。强制重建由 `views_stale` 标记驱动：初启、手动刷新、范围/工具/模型筛选变更、页面懒加载数据缺失、配额轮询入账（`QuotaDone n>0`）、定价重刷（`PricesDone`）、加载失败重试。扫描**失败视为已变**（不能证明"没变化"时保守重建）。空闲 tick 成本从 ~430ms（扫描+查询+重建）降到纯扫描 ~274ms，且 UI 线程零抖动。
- **修复三（配额表有界化）**：`quota_snapshots` 每轮轮询全键追加历史行（本机 8 键已攒 11,431 行）——`prune_quotas` 挂进每日 `maybe_backup` 节流块：删除超 30 天历史行但**每键保留最新一行**（陈旧配额照常显示），prune 先于 `VACUUM INTO` 使备份始终收剪后体积。
- **引擎诊断**：`scan_once` 加适配器级 `debug!` 计时（id/ms/files/scanned），后续热点排障不用再插桩。
- **验证**：新增 `prune_quotas` 测试（超龄删除+每键保最新）；codebuddy 测试改 `::default()` 构造。**50/50 测试、clippy `-D warnings` 0、release 干净**；UI 冷启动实测窗口 1365ms/首数据帧 1583ms（debug 对照 2090/2507）。
- **未做（如实记录）**：每 tick 仍 reopen Store + PriceBook::load（~20ms，可换共享连接但改动大、收益小）；首帧仍等扫描完成（可先查旧账本先渲染再补扫，~200ms 收益、复杂度不值）；`usage_events` 自动 prune 未启用（CLI 手动 `prune` 保留——详情页历史是产品需求，不擅自开自动删）。

## S42 安装器升级路径 ✅

- **需求**：检查安装程序功能性，要求支持用更新版本的安装程序升级既有安装。
- **审计结论（升级机制原本已具备的）**：`extract_payload` 原地覆盖 ui/cli exe；`register_uninstall` 重写 `DisplayVersion`；`self_exe→globaltokentracker-setup.exe` 拷贝使卸载器跟随新版；`stop_running` 先杀进程防文件锁；用户数据在 `%USERPROFILE%\.globaltokentracker`（安装目录之外）全程不动；`Store::open` 迁移保证旧库新读。
- **修复的两个缺口**：
  - **自定义 `--dir` 安装无记忆**——新版安装器原本会装回默认目录，把旧目录变成孤儿。新增 `installed_info()` 读 `Uninstall` 键的 `InstallLocation`+`DisplayVersion`（校验 `globaltokentracker-ui.exe` 真实存在，过滤陈旧注册项），无 `--dir` 时优先装到已记录位置；GUI 检测到既有安装进入"更新"态：标题/按钮改"更新"、副标题显示 `已安装 v{old} — 更新至 v{VER}`（同版本显示"重装修复"）、路径框与浏览按钮禁用（升级不搬家，避免孤儿目录）。
  - **运行中的旧卸载器锁文件**——`stop_running` 补 `globaltokentracker-setup.exe`（一个开着的旧卸载器窗口会让 `fs::copy` 覆盖失败）；按自身映像名跳过自杀（重命名后的下载副本不会被自己 taskkill）。
- **实测（0.1.0→0.2.0 全流程）**：`--quiet` 升级 → `DisplayVersion` 0.2.0、`setup.exe` 换成新版哈希；`--dir` 自定义安装后不带参再跑 → 写回记录的自定义目录、默认目录 mtime 不动；自定义目录卸载 → 目录+注册项清干净；再装回默认 → 注册表复原；全程 `ledger.db`（46MB）mtime 不变。GUI 截图确认"更新"态渲染。
- **版本**：workspace bump 0.1.0 → 0.2.0（新增 4 适配器+性能改动累计够 minor）。
- **已知边界（如实记录）**：GUI 子系统 exe 在 PowerShell 下 `&` 调用不等待——脚本里要 `Start-Process -Wait`；`--quiet` 的输出经 `attach_console` 写到真实控制台，重定向文件收不到（设计如此）；payload 只增不改——若未来重命名 exe，旧文件残留需主动清。

## S43 代码签名管线（EV 就绪）✅

- **需求**：为后续 EV 签名做准备。
- **管线**（`installer/sign.ps1` + `package.ps1` 两处挂钩）：
  - payload 入 zip **前**签 ui/cli（内嵌二进制本体带签名），dist 落盘后签安装器外壳——SmartScreen 检查的两层都覆盖；
  - 证书源三选一按优先级：环境变量 `GTT_SIGN_SHA1`（token/HSM 证书入 `CurrentUser\My`——EV token 客户端装好即出现，私钥不出 HSM）、`GTT_SIGN_PFX[+_PASS]`、`GTT_SIGN_DLIB[+_METADATA]`（Azure Trusted Signing 等云签）；`GTT_SIGN_TSA` 默认 DigiCert 时间戳；无配置时跳过并提示，本地构建不受影响；
  - 签后逐文件 `signtool verify /pa` 强制校验。
- **采购侧**（`docs/SIGNING.md`）：EV 只发组织——需营业执照主体；列了 SSL.com/Sectigo/DigiCert/GlobalSign 价位与 token/云 HSM 交付方式；特别评估了 **Azure Trusted Signing**（~$10/月、免硬件、SmartScreen 信誉基线即时生效）作为高性价比替代，管线已兼容其 dlib 模式。
- **演练验证**：本地自签名测试证书（`CN=GlobalTokenTracker Dev (TEST)`，用户域、可删）跑通全流程——ui/cli/setup 三 exe `Get-AuthenticodeSignature Status=Valid` + DigiCert TSA 时间戳。真 EV 到手后仅改 `GTT_SIGN_SHA1` 指纹即可，零代码改动。dist 已重建为无签名净版。
- **顺手修**：core/cli/ui 版本号原为硬编码 0.1.0 而 setup 用 `version.workspace=true`——`VER`/包名与各 crate 版本漂移。统一为 workspace 继承（`globaltokentracker-core` path 依赖改 `workspace=true` + 根 `[workspace.dependencies]`），0.2.0 单一事实源。
- **验证**：50/50 测试、clippy `-D warnings` 0、release 干净、dist 9.45MB。

## S44 安装器文件夹选择器异步化 ✅

- **症状**：更新模式窗口里点"浏览…"无反应、无系统选择器弹出。
- **根因（环境，非代码）**：测试机挂有失联的 SMB 映射盘（`Z: → \\<nas>\<share>`）——Windows 公共文件对话框枚举网络位置时长时间卡死。最小复现确认 `IFileOpenDialog::Show`、`SHBrowseForFolder`、.NET `FolderBrowserDialog` 全部同样卡死。
- **修复（`crates/setup/src/gui.rs`）**：`pick_folder` 改为**全异步**——独立 STA 线程跑 `IFileOpenDialog`（自 `CoInitializeEx`），结果 `Box<Option<PathBuf>>` 经 `WM_APP_PICKED`（`WM_APP+2`）回主窗口；`pick_pending` 防重复点击，期间浏览按钮置灰 + 状态栏提示"若无响应请直接输入路径"；选/取消/失败都恢复按钮。窗口中途关闭时 `PostMessageW` 静默失败，无悬挂风险。卡死的对话框只泄漏 helper 线程直到进程退出，用户仍可手输路径。
- **设计连带变更**：更新模式**重新允许换目录**（此前禁用浏览过于反直觉）——换目录安装后 `cleanup_prior_install`（main.rs:311）清旧 PATH 项 + `cmd` 延迟 `rmdir` 清旧目录（`new.starts_with(old)`/同路径/旧目录无 ui.exe 时跳过，防误删），GUI（gui.rs:757）与 console（main.rs:488）路径均接入。
- **实测（UIA/Win32 驱动真实窗口）**：点击即返回不阻塞；对话框随后正常弹出；对话框开着时取消 → `pick_pending` 复位、按钮恢复；点"选择文件夹"确认 → 选中路径正确写入安装路径框、按钮恢复。自动化踩坑：文件名框是 `ComboBoxEx32` 内嵌 Edit，UIA 的 Edit 枚举首项是列表项重命名框（误触"重命名"错误框，未造成实际改名）。
- **验证**：50/50 测试、clippy `-D warnings` 0、release 干净；安装包重打（sha `6e28f3a2…`）。
- **未做**：不会主动修复用户的失联网络映射——那是系统环境问题，安装器只需不卡死（已满足）；无超时强制关对话框（用户的选择权优先，挂起只损失一个 helper 线程）。

## S45 内存/资源泄漏审计 ✅

- **范围**：`Box::into_raw`/`forget`/`leak`、GDI/GDI+ 句柄配对、后台任务槽堆积、无界缓存、Rc 环、线程/连接生命周期——全仓扫 + 逐处读码。
- **修复的唯一真实问题（engine.rs）**：JSONL 扫描把**全部**变更文件的未读字节一次性装进 `segments` Vec 再做并行解析——无总量上界。新装机面对数 GB 累积日志会先 `Vec::with_capacity` 预占同等内存再读，可能 OOM/卡死。重构：phase-2/3 抽为 `flush_segments`，新增 `SEGMENT_BUDGET = 256MB` 在飞字节预算——批次超限先冲刷再续读；单文件 >256MB 仍整读（游标要求连续字节，已在注释标明）。预算管的是批次不是单源。
- **验证为干净的**：
  - GDI/GDI+：setup 绘制路径所有 `CreatePen/SolidBrush/Font/Bitmap/Path/Graphics` 均就近 Delete（画刷 356-394、字体 502-534、GDI+ 156-160/171-179 逐一配对）
  - 任务槽：reactor `BACKGROUND_TASK_CAPACITY=64`，常驻阻塞任务（watcher/tray）都是"上一个返回才再武装"各稳占 1 槽；定时器经 `Loaded/Failed` 单一武装点自我收敛回 1
  - `Rc<TrendShared>` 单向（Shell→视图闭包），无回指不成环
  - `otel::spawn` 返回 `JoinHandle`——`let _otel` drop 仅 detach 不杀接收线程（正确用法）
  - watcher `watcher` 对象函数内建函数内毁，退出即释放 ReadDirectoryChanges 句柄
  - codebuddy `session_dirs` 缓存存活期=Engine 实例=一次扫描调用，有界
  - SQLite `Store` 每 `load_all` 开一次 drop 关一次；`PriceBook::load` 全量 8K 行生命周期单次调用
- **如实记录的既有取舍（不改）**：`Box::into_raw(Gui)`/安装器 WM_APP_PICKED lparam 盒——进程级生存期/窗口已关时最多漏 24B；setup 控件 `WM_SETFONT` 的 ~8 个 HFONT 进程级不删（短生命周期安装器）；悬停柱的 450ms 驻留计时每次换柱 spawn 一次（上限 64 槽，自限）；`read_segment` 对超大单文件仍整读——改流式解析才解，值不值留待真有 GB 级源再说。
- **验证**：50/50 测试、clippy `-D warnings` 0。

## S46 BIL 发布工作流移植 ✅

- **范围**：参照 Bilibili_Innocent_Lab 的 alpha/stable 双通道发布体系移植到 GTT（`.github/workflows/` + `.github/release-templates/`）。
- **保留的 BIL 架构**：verify/publish 双 job、main 精确 SHA 校验 + 陈旧构建守卫（checkout SHA == GITHUB_SHA == remote main）、门禁步骤带名互不短路、环境保护、模板化发布说明、LLM 双槽位 + 规则回退、BUILD_INFO/SHA256SUMS 溯源、`gh release create`。
- **GTT 适配**：Gradle/aapt/apksigner/R8 链 → `cargo test`/`clippy -D warnings`/`installer\package.ps1`；版本一致性校验重写为 `validate_build.py`（root Cargo.toml `[workspace.package] version` 为单一事实源：stable tag 必须全等，alpha 为 `vX.Y.Z-alpha.N` 同基版）；APK_* 模板 token → ASSET_*；`release_notes_config.json` 产品语境/领域词/禁用术语全量替换；`user_text_paths` 置空（解析器只认 Android `<string>` XML，指向 .rs 会静默空集——不假装生效）。
- **取舍**：空 alpha tag 允许（push 触发只做构建验证不发版）；首个 stable 用合成基线条目（BIL 原设计，说明靠 release_summary 承载）；`sync-lsposed-release.yml` 无对应物不移植；签名 Secret 透传留位，未配置则产出未签名包。
- **真实 runner 验证（push 触发）**：run 36334277350 verify job 9m24s 全绿——12/12 Python 测试、tooling 干跑（changelog 生成+模板渲染+token 零残留）、debug/release 构建、50/50 单测、clippy 0 警告、package.ps1 产出安装包；publish 正确跳过。
- **首跑暴露并修复**：Python 3.14 + cp1252 控制台下 `SystemExit` 消息含 em-dash → 子进程 `UnicodeDecodeError`。子进程解码统一 `errors="replace"`，错误消息去非 ASCII。
- **待办（使用时）**：发布前在仓库 Settings 建 `alpha-release`/`stable-release` 环境（可加审批）；首个 stable 建议手动填 `release_summary`。

## S47 设置页 + 国际化 + 开机自启动 ✅

- **范围**：新增设置页（主题/主题色/字体/字号/语言/开机自启动），全 UI 字符串 i18n 化（中/英双语运行时切换），托盘菜单本地化，`--minimized` 静默启动到托盘。
- **架构**：
  - `i18n.rs`：`Lang`（AtomicU8 全局态——D2D 绘制回调可能不在 UI 线程，thread_local 不安全）+ `tr` 全字匹配表（缺键透传中文，永不空白）+ `t!`/`tf!` 宏（`tf!` 位置占位替换，`as_display` helper 做 `&T→&dyn Display` unsize）+ `compact()` en 模式 K/M/B 压缩。词条表按页面分区注释。
  - 关键纪律：**语义值不过 UI 文本**——SelectorBar 只吐 item text，`pick_option`/`Range::LIST` 用 `tr(label)` 双语匹配回语义值（en 模式下 Range 解析曾会失配回退 Week，已修为回调时双语比对）。
  - `autostart.rs`：HKCU `...\Run` 写/删/读，`current_exe` 路径 + `--minimized` 参数；`set()` 返回**操作后的真实注册表状态**（`enabled()` 复读），不是操作成功与否——第一版曾把 `set(false)` 的"删除成功 true"误存为 `autostart=true` 导致开关视觉回弹，实测抓出已修。
  - `config.rs`：`lang`/`window_theme`/`autostart` 三字段 `#[serde(default)]` 向后兼容；autostart 启动时以注册表为权威对账（重装/手删自愈）。
  - 主题：`WindowVisuals::theme` 每次 view 重发布（SetThemeMode 零额外工作）；reactor `apply_window_theme` 映射 `ElementTheme::{Default,Light,Dark}` 真切换 + TitleBarTheme 跟随。
- **如实标注的限制**：字体族仅作用于 D2D 图表文字——reactor 0.100 无 XAML 控件 font_family setter，设置页该行附注说明，不冒充全局生效。
- **实测（GUI 端到端）**：设置页渲染正常；语言切换实时全页生效（截图验证 zh→en）；ToggleSwitch 空格触发 → `want=true got=true` 注册表写入 `"exe" --minimized` → 再切 `want=false got=false` 值删除不回弹；`--minimized` 启动实例 4 个顶层窗口全 `visible=False` 托盘常驻。
- **环境插曲**：测试期间 Run 键写入遭 AV 行为监控拦截（~31s 阻塞后失败）——代码无 bug，`set` 返回真实状态的设计恰好兜底；单测改到非监控暂存键验证注册表 plumbing。
- **验证**：52/52 测试（core 50 + i18n/autostart 各 1）、clippy `-D warnings` 0、触碰文件 fmt 净。

## S48 OpenCode 统计修正：会话级 → 消息级粒度 ✅

- **问题**：用户报告 OpenCode 统计不准。实测对照源库发现三条同源根因——
  1. **日分布压平**：`ses_fd13…` 会话跨 6 天（138.9h），38.6M input + $4.74 成本被整体记在创建日 8/23 一条事件上；真实消息分布在 8/23→8/29 七天。
  2. **时长虚增 27×**：`duration = session.time_updated - time_created` 算的是会话存活期（598,530s），真实 API 计算耗时仅 22,271s。
  3. **模型归属全错**：512 条 assistant 消息实际横跨 2 provider / 9 模型（含 106 条免费模型），session 行只记最后一个模型 `glm-5.3-flash`——deepseek/longcat/muse-spark/ox-alpha-free 的用量全被归到 glm 头上。
- **核查链路**：`session.tokens_*`/`cost` 与 `message` 逐条求和 **Δ=0**（session 是忠实汇总，不是错数据，是错粒度）；ledger 与源库总量逐分不差；`storage/` JSON 旧格式为空目录无遗留数据。
- **修法**：`scan_sqlite` 改读 `message JOIN session`，每 assistant 消息一个事件——
  - `dedup_key`: `opencode:session:{id}` → `opencode:msg:{id}`
  - `ts_start/ts_end/duration`：`data.time.created/completed`（真实调用耗时），回落 `m.time_created`/`m.time_updated` 列
  - `model/provider_id`：`data.modelID`/`providerID`（每条消息真实模型）
  - `cost>0` → `provider_reported`；`=0` → 价格簿兜底（免费模型如 `*-free` 保持 NULL 不编造）
  - `tokens.total` 与分量做自校验，不一致记 `notes`
  - 高水位沿用 `message.time_updated`（同毫秒刻度），1s 重叠幂等合并
- **迁移**：`sync_cursors.adapter_state` 标记 `"msg-v2"`——首扫时 `DELETE FROM usage_events WHERE app='opencode'`（新增 `Store::delete_app_events`）+ 水位归零全量重灌；老库升级不重计、不残留，崩溃重跑幂等。
- **schema 守卫**：`session`/`message` 缺一即报 `unsupported opencode schema`（计入数据源健康），不静默吞零。
- **实机验证**：`scan opencode` → 488 事件入账（24 条零量中止调用被计量门槛正确跳过），总量守恒（in 38,925,612 / out 136,254 / cacheR 213,744,974 / $4.76 与源库逐分不差）；按日分布修正为 8/23–29 七天、按模型拆出 8 个 provider/model 组；旧 `opencode:session:*` 行清零，游标 `msg-v2`。
- **验证**：54 core + 2 ui 测试全过（新增 4 个：消息级事件/迁移清除/更新重扫/schema 守卫），clippy `-D warnings` 0，触碰文件 fmt 净。

## S49 跨适配器计数审计 + ZCode 水位线修正 ✅

- **范围**：对照 OpenCode 三类失真（事件粒度≠计费粒度 / ts_start≠真实调用时间 / 累计值当增量）审计全部 14 个适配器，并实证核查水位线语义。
- **审计矩阵结论**：
  | 适配器 | 粒度 | token 语义 | 水位 | 结论 |
  |---|---|---|---|---|
  | Claude | 消息级（message.id 末行收编流式） | 每请求 | jsonl 字节 | ✅ |
  | Codex | 每调用 `last_token_usage` | 增量/累计显式区分 + PrevLine 去重 | 字节+state | ✅ |
  | Devin | 每请求 request_id（快照合并） | 每请求；会话累计字段显式不映射 | row_id 自增 | ✅ |
  | Cursor | 每用户气泡 | 元数据级（无 token，诚实不编造） | rowid | ✅ |
  | Qoder | 每段文件 | 元数据级 | 字节 | ✅ |
  | OpenCode | 消息级（S48 已修） | 每消息 | time_updated+state | ✅ |
  | Grok | 每 (prompt_id, model) | per-turn 含 nano-USD | 字节 | ✅ |
  | WorkBuddy | 每 rawUsage/messageId | 每请求多 provider 缓存字段变体取 max | 字节 | ✅ |
  | CodeBuddy | 快照 delta | 累计→分量差分（正确姿势） | state cum | ✅ |
  | MiniMax | 每 LLM 调用行 | 每调用 vendor 成本 | id 自增 | ✅ |
  | Kimi | 每 usage.record | 每调用 delta | 字节 | ✅ |
  | Cline | 每 api_req（started/finished 配对） | 每请求；deleted_api_reqs 仅首扫 | {len,mtime} | ✅ |
  | CommandCode | 每 assistant message | 每请求；1h/5m 缓存拆分正确 | 字节 | ✅ |
  | **ZCode** | 每 (lrid, attempt) | 每请求 | ~~started_at~~→**rowid** | 🔧 已修 |
- **ZCode 实锤缺陷（与 OpenCode 同族，方向相反——不是虚增是漏计）**：
  - `model_usage` 行在**请求完成时**才写入，`started_at` 回填原始开始时刻；水位线却用 `started_at`。
  - 真实库实测：4829 行中 **780 行迟到插入**（≤10s:267 / ≤60s:382 / >60s:131），最差 **772 秒**——长请求（实测有 189K token 的 70s+ 子代理调用）在水位推过后落库即被 `WHERE started_at > mark` 永久漏掉；1s 重叠完全不够。
  - 修复：水位改 `rowid`（单调插入序），`adapter_state="rowid-v2"` 一次性迁移——旧时间戳游标归零全量重扫，dedup key 不变幂等合并；`status='running'` 行不封顶水位（完成后重扫补入）。
  - 实机验证：`scan zcode` → +4773 事件、merged 0（幂等重灌无重复）、账本总量不变；源库 4829 行 = 4773 计费 + 29 cancelled + 25 error + 2 零量 completed。
- **其余适配器的深检记录**（非 bug，存档备查）：
  - Devin `row_id` 自增水位正确——`message_nodes` 单行单调；`sessions.metadata.total_acu_cost` 会话累计刻意不映射事件（映射会重复计）。
  - CodeBuddy 的 `statsSnapshot` 是**会话累计计数器**——但实现恰是分量差分（`snap - persisted_cum`），delta 挂到当前消息时间戳；总量守恒、粒度打包为 Estimate 如实标注。
  - Store upsert 语义确认：`completeness >=` 等值即覆盖 → 同维度的流式更新后写赢，最终快照必然落账（Claude/Devin 依赖此性质）。
  - ZCode provider 排除名单与真实数据核对：库里 provider 全为 `new-provider*`/`builtin:bigmodel-start-plan`/`account:*`/UUID，无 anthropic/openai/google 行被误排。
  - Claude `output_tokens_details.thinking_tokens`→reasoning 是 output 的子集展示列，汇总口径为分列展示（趋势约定 input+output+cache_read 不叠加 reasoning）——无重复计。
- **理论边角（观察未修）**：Cline dedup 用行 `ts`（ms）——同毫秒多请求完成会撞 key，subagent 扇出理论上可能；Claude message.id 全局去重假定跨文件同 id 即同消息（fork 重写场景设计如此）。
- **验证**：新增 3 个 zcode 测试（迟到行不漏/时间戳游标迁移/running 行不封顶），57 core + 2 ui 全过，clippy `-D warnings` 0，zcode.rs fmt 净；临时诊断 example 已删。

## S50 自定义日期范围（日历选择器）✅

- **需求**：统计区间原来只有 今日/近7天/近30天/全部 四个预设，参照 cc-switch 增加日历式自选日期。
- **实现**：
  - `Range` 新增 `Custom{start_ms,end_ms}`（Copy 保持，`[start,end)` 本地日界对齐）；预设 `end_ms=None`，Custom 带独占右端。
  - UI：范围选择器加第 5 项"自定义"；选中后标题下出现两个 `CalendarDatePicker`（WinUI 原生月历弹层，ccswitch 同款体验）+ 当前区间文本回显。`reactor 0.100` 的 CalendarDatePicker **无 `date` setter**——不回显控件内选中值，靠旁边 `YYYY-MM-DD → YYYY-MM-DD` 文本展示当前窗口（代码注明）。
  - 日期换算：picker 回报 UTC 午夜的 `windows_time::DateTime`（1601 纪元 100ns），`utc_day_to_local_start` 转成该 civil 日的**本地** start-of-day，窗口语义=本地日历日。
  - 语义：截止日为**含当日**（存 end=sod(次日)）；起止倒置时 clamp 到单日；点"自定义"无历史 bounds 时默认最近 7 天窗口。
  - 持久化：`ui.json` `range="custom"` + `range_start_ms/range_end_ms`（Option<i64> epoch ms，serde 默认兼容旧配置）；`Range::from_config` 重建。
  - 查询链：`bucket_models` 补 `to_ms` 参数，`totals`/`by_app` 本来就有——Custom 窗口对趋势图、卡片合计、按工具分布同步生效；明细页不受 range 约束（原设计）。
- **实测（GUI）**：选"自定义"→ 默认近7天窗口（09-22→09-28，2.76B tok/11,490 事件）；点开"起始日期"弹原生月历（2026年9月网格，今天高亮），选 25 日 → 控件回显 2026/9/25、区间文本 09-25→09-28、合计降至 1.51B/6,909、趋势/按工具同步收窄；`ui.json` 正确写入 custom 双 bound；切回"近 7 天"恢复。
- **i18n**：新增 自定义/自定义·按天/从/至/起始日期/截止日期 六词条。
- **验证**：59 测试全过、clippy `-D warnings` 0、触碰文件 fmt 净（query.rs 漂移已分离未混入）。

## S51 提交署名强制拦截 ✅

- **起因**：历史提交混入 `Generated with Devin`/`Co-Authored-By` 尾注（提交模板默认值，未得用户许可）——用户明确要求杜绝复发并拦截其他 agent。
- **清理**：重写 `6e539a1`/`e1e38b2` 两个提交去掉尾注 + force-push（cherry-pick 保序重放，重写前后树 diff 为空验证零代码变化）；全库 10 个最近提交 trailer 已全空。
- **强制层（不依赖自觉）**：`.githooks/commit-msg` 正则拒绝 `Generated with`/`Co-Authored-By: <AI|bot>`/`Signed-off-by: <AI|bot>`/`🤖` 等署名；`core.hooksPath` 已指向 `.githooks`——实测带尾注提交被拒（exit 1）、干净提交放行。
- **声明层（多入口覆盖）**：AGENTS.md / CLAUDE.md / `.github/copilot-instructions.md` / 全局 `~/.claude/CLAUDE.md` 均写入同一硬性规则。
- **如实限制**：钩子是本地机制——新克隆需 `git config core.hooksPath .githooks` 激活一次（已写进 AGENTS.md）；对**其他仓库**不生效，全局 hooksPath 会劫持已有钩子的项目，故未启用，靠全局 CLAUDE.md 规则兜底。

## S51 字体枚举 + 字号滑块 ✅

- **字体下拉**：`fonts.rs` 用 DirectWrite `GetSystemFontCollection` 枚举本机全部字体家族（与图表渲染同引擎，名字保证可解析）；优先用户 UI 语言、回落 en-US、再回落首条；进程内 OnceLock 缓存。实测下拉列出真实安装字体（含用户装的 "Aa可爱の日系中文"），UIA 选 "Bahnschrift" → `ui.json` 写入成功。
- **字号滑块**：替代原 3 档预设，Slider 9–18pt、步进 0.5（19 档连续），拖动实时预览+持久化；title/h2/label 保持 +10/+2/−1 偏移。`Msg::SetFontScale` → `SetFontSize(f64)`。
- **旧数据兼容**：`ui.json` 里已存的 4 个绝对字号字段原样保留，滑块按 `body_size` 读当前值。
- **验证**：UIA 实测 slider SetValue(12→15) 回读 `body_size:15/title:25`；枚举测试断言 >10 家族且含 Segoe UI；60 测试全绿、clippy 0、触及文件 fmt 净。
- **如实限制**：字体仍只作用于 D2D 图表文字（reactor 0.100 无 XAML font setter），行内注释保留。

## S52 费用占比环形图 ✅

- **数据源**：`Store::by_model` 新增（`MODEL_EXPR` 分组，与模型筛选器同一身份）；`OverviewVm.by_model` + 复用 `by_app`——均走 `scope_where(from,to,apps,models)`，自动跟随范围选择器（今日/7天/30天/自定义）与工具/模型筛选。
- **UI**：总览新 widget `share`（默认排在趋势后）：D2D 环形图（环形扇区多边形近似，~3° 步进，0.100 无 arc 原语）+ 图例（色块+名称+`xx.x% · $n`）；`按模型|按工具` SelectorBar 切换，`ui.json:share_dim` 持久化；top-6 + "其他" 折叠。
- **诚实口径**：仅统计可计价费用——`cost_usd<=0` 的行不画扇区，全零时显示"所选范围暂无可计价费用"而非假图。
- **修的真 bug**：`tf!` 模板不支持 `{:.1}` 格式说明符（按 Display 直出 15 位小数）——改为先 `format!("{:.1}")` 再进模板。
- **实测**：UIA 切维度截图验证——按工具 Codex 50.0%/$3710 vs 按模型 claude-opus-5-5 31.6%/$2342，合计 $7416 与统计卡一致。

## S53 占比卡片横向多环布局 ✅

- **需求**：单环+切换器横向浪费空间 → 改为 4 环并排：`按工具·费用` `按模型·费用` `按工具·Tokens` `按模型·Tokens`（top-4+其他，图例 `name xx.x% · 值`）。
- **结构**：`donut_cell(title, slices, center, fmt_v)` 组件化；`Grid STAR×4` 列均分。维度切换器退役（四口径同屏展示），`share_dim`/`SetShareDim` 移除。
- **修的 bug**：4 个 canvas 共享一个 `Invalidator` 时只有第一个绘制——改为每环自建 Invalidator（donut 无悬停重绘需求，重建即刷新），注释说明原因。
- **实测**：4 环全部渲染，中心合计 `$7416`/`164.5亿` 与统计卡一致；图例百分比单位正确（修过 tf! 不支持 `{:.1}` 的问题）。

## S54 总览自适应折行布局 ✅

- **需求**：总览元素随窗口宽度自适应，不固定死 4 列。
- **方案**：统计卡与占比环从 `Grid STAR×4` 换成 `VariableSizedWrapGrid`（`Orientation::Horizontal`，`item_width=250`），单元间距改由 `wrap_cell` 的 right+bottom margin（wrap grid 无 spacing 属性）。
- **实测**（PrintWindow 抓图）：1196px → 4 列；800px → 统计卡 2×2、环图 2 列起排；560px → 统计卡单列堆叠。趋势图本就 STAR 拉伸不受影响。
- **取舍**：单元宽度固定 250（卡片不随窗拉伸铺满——比固定列数挤压更可读）；窗口 < ~300px 仍会裁切（win 最小尺寸之下，可接受）。顶栏导航在极窄窗与品牌文字重叠是既有问题，不在本次范围。
- **验证**：60 测试全过、clippy `-D warnings` 0；pages.rs 存在仓库级 rustfmt 漂移（非本次引入），本次改动区域 fmt 干净。

## S55 总览列宽真自适应（宽度尺子 + STAR 网格） ✅

- **需求**：S54 的 VariableSizedWrapGrid 只能固定 250 单元宽折行，卡片不随窗拉伸、右侧留白——要"横宽自适应"。
- **方案**：reactor 无 SizeChanged 高层 API，但 `ElementRef<SwapChainPanel>::observe_surface` 会推 `Metrics{width}`。总览页挂 1-DIP 全宽尺子面板；观察回调把宽度量化成列数（`fit_cols(width, 250, 12, 4)`），**仅跨列数边界才发 `Msg::SetOverviewCols`**（resize 拖拽不刷屏）；Shell 存 `overview_cols`，`reflow_grid` 用 `STAR×N + Auto行` 重建——列宽随窗拉伸、列数随宽自适应。
- **实测**（PrintWindow）：1600px → 4 卡铺满全宽；800px → 2×2 撑满；560px → 单列满宽。S54 的固定单元留白问题消除。
- **取舍**：尺子只在总览页挂载（其他页未自适应）；列数变化是离散跳变而非连续插值（网格重排本就离散）。`SwapChainPanel` 空面板不渲染内容，开销≈一个空元素。
- **验证**：60 测试全过、clippy `-D warnings` 0；改动区 fmt 干净（仓库遗留漂移照旧不动）。

## S56 占比环扇区悬停详情 ✅

- **需求**：鼠标悬停饼图扇区显示对应分类详情。
- **方案**（沿用趋势图 hover 管线）：扇区 canvas 包 `Border`（透明背景保命中）→ `on_pointer_moved/exited` → `Msg::DonutHover(col, Option<idx>)` → Shell 写 `DonutShared.hover` + `Invalidator` 重绘。命中测试 `donut_hit`：指针坐标→环形距离判定 + 顺时针角累积区间（12 点起算，与绘制几何同参）。
- **悬停表现**：当前扇区外扩 2.5px、其余压暗至 32% alpha；中心文字由合计切换为 `名称 / xx.x% · 值`（主题色两行）。移出环孔/边界 → None 还原。
- **基础设施**：`DonutHandle`(shared+inv) ×4 存 Shell；`DonutSpec` 打包 slices/center/fmt/key/handle 避参数上限；`GTT_DONUTTEST=<col>,<idx>` 强制悬停（注入指针到不了 content island，与 GTT_TIPTEST 同理）。
- **实测**（PrintWindow）：`GTT_DONUTTEST=0,1` → 第 1 环显示 `Claude 39.1%·$2896`+他区压暗；`=2,0` → 第 3 环 `Codex 35.8%·59亿`。
- **验证**：`donut_hit` 单测 2 个（角度区间归属/环孔外空拒绝）共 62 全过；clippy `-D warnings` 0；改动区 fmt 干净。

## S57 环图对齐 + 页面切换动画 ✅

- **对齐**：donut `horizontal_alignment` Center→Left——之前环居中悬浮，下方图例靠左，视觉错位；现在环左缘与图例文字左缘齐平。
- **页面切换动画**：`LayoutControl::exit_transition` + Border 原生 `opacity_transition`/`scale_transition`（reactor 0.100 的合成器动画，无自写插值）。
  - 页面容器按 `page:<name>` 做 key：换页 → 旧容器卸载触发 `ExitTransition::fade(160ms)` 淡出
  - 入场两阶段：`Msg::Nav` 置 `page_entering` → 首帧挂载 opacity 0 / scale .99 → 30ms 后 `Msg::PageSettled` → 0→1 / .99→1 过渡 200ms，与淡出交叠成交叉淡入
  - 同页导航不触发；30ms 延迟是单帧级，额外 tick 无副作用（幂等置 false）
- **实测**：总览↔明细↔配额 UIA 切换正常渲染（opacity 正确回 1，无卡壳）；环图对齐截图确认。
- **取舍**：入场用的"挂载 0 → 下一帧置 1"两帧法——reactor 属性过渡只在已挂载元素间生效，挂载即终值不会自动播入场动画；PrintWindow 抓合成器最终态，动画帧需肉眼确认。
- **验证**：62 测试全过、clippy `-D warnings` 0；改动区 fmt 干净。

## S58 性能审查 + 增量 rollup 重建 ✅

- **审查结论**（release 实测，62k+ 事件账本）：
  - 闲置 UI：~2.7%/核（稳定后）、工作集 ~172MB — WinUI3 基线水位，无持续渲染循环泄漏
  - 扫描路径：`scan_once` → 每摄入事件即触发 `rebuild_rollups` = **全表 DELETE + 全表 GROUP BY 重算**（实测单次 ~1309ms）——这是唯一真热点；其余（适配器文件枚举、按需查页、WAL/NORMAL、release LTO/strip）均合理
- **修复**：摄取路径顺手记账——`ScanReport.rollup_days` 收集每个**实际写入行**（含 ON CONFLICT 升级，`upsert_event` 的 DO UPDATE 会改聚合字段）的本地日索引 `(ts + off_ms).div_euclid(86_400_000)`，与 rollup SQL 的 `strftime(...,utc_offset)` 同帧；`rollup_full` 兜底无 ts 行。
  - `Store::rebuild_rollup_days`：DELETE 只删触及日的 date 串（UTC 民用日换算用 jiff，与 strftime 输出逐字节同义）；INSERT 用 `ts_start >= ? AND < ?` 区间 OR 走 `idx_events_time` 索引，避免逐行 strftime
  - `refresh_rollups`：days 空不进；>400 天或 rollup_full → 回退全量重建（首装回填/时区变更场景）
  - `utc_offset_ms` 校验/解析收敛到 viewmodel，两处复用
- **实测**（release）：摄入事件的扫描内部耗时 **~117ms**（原：0 摄入 ~650ms、有摄入再 +1309ms rollup）；全量 `rollup` 后 `report all` 聚合不变（$7416.44 一致）
- **取舍/边界**：ON CONFLICT 升级若把 ts_start 改到更早的日（理论边角，快照间 start 不变）旧日聚合会滞后，修复路径 = `rollup` CLI 全量重建；400+ 天首装回填仍走全量——一次性成本可接受
- **验证**：新增 2 测试（局部重建=全量逐行一致 / +08:00 帧日界正确 / 空集 no-op）共 61 全过；clippy `-D warnings` 0；改动区 fmt 干净

## S59 canvas 底色融入卡片（消灭 SwapChain 黑框） ✅

- **现象**：每个环形图背后有一块 ~150×132 的暗矩形（#1F2021），明显深于卡片（#2B2B2C）。逐像素测量发现**趋势图同样存在**，只是全宽不易察觉。
- **机制**（红填充实验证实绘制路径正常）：`canvas_invalidated` 的交换链虽以 PREMULTIPLIED 创建，但透明像素的合成目标是**窗口/页面底色**而非下层卡片——Fluent `CardBackground` 主题画刷本身是 ~5% 半透明白叠页面底，交换链透明区跳过了这层，透出页面 #202020。`ctx.clear(TRANSPARENT)` 的"透出卡片"假设在此栈不成立。
- **修复**：不再依赖透明穿透，canvas 直接把卡片等效填充色画进 buffer——新增 `Theme.card_cf`：`card_bg` 为 hex 配置时 `colorf_of` 精确映射；默认走 Fluent 暗色 `CardBackgroundFillColorDefault` 等效值 `rgba(255,255,255,0x0D)`，canvas 与卡片画刷在同一页面底上合成出逐字节一致的结果。donut 与 trend 两处 clear 均替换。
- **实测**：修复后 canvas 区域像素 `#2B2B2B` vs 卡片 `#2B2B2C`（Δ=1 LSB 舍入差，视觉不可分辨）；黑框消失。
- **取舍**：`card_bg` 配非 hex 的命名画刷（如 `accent`/`solid`）时 `card_cf` 回退默认透明白——会轻微失配，已注释；亮色主题同样按暗色常量回退（与既有 accent_cf/subtle_cf 策略一致）。
- **验证**：64 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。

## S60 占比环悬停气泡（外缘锚定，替代中心置换） ✅

- **需求**：悬停详情不再塞进环孔（空间太窄），改为环外缘的小气泡。
- **方案**：canvas 150→200 宽，环固定在左侧 h×h 方区（`donut_geom(w,h)` 统一出 cx/cy/r_out/r_in，绘制与命中共用一处几何源，再不会漂移）；新增 `hover_am` 在绘制循环里记下悬停扇区中角，气泡锚定 `r_out+16` 的外缘点并 clamp 进 canvas；中心**恒显合计**不再置换。
- **气泡内容**（Fluent 暗色 tooltip 风格，同趋势图）：名称（主题色粗体，18 字截断）+ `xx.x% · 值` + `第 i / N 项` 名次——名次是中心空间放不下的增量信息。
- **命中变化**：`donut_hit` 签名去 total 参数（内部求和）、改传 w/h 经 `donut_geom`；右侧气泡带几何上在环外自然返回 None——气泡非交互元素，离开环即消失，无闪烁（命中是纯几何不依赖元素）。
- **实测**（GTT_DONUTTEST）：col0·Claude → 左下外缘气泡 `Claude / 39.1%·$2896 / 第 2 / 5 项`；col2·Codex → 右外缘气泡 `Codex / 35.8%·59亿 / 第 1 / 5 项`；中心恒显合计。
- **验证**：`donut_hit` 两测试更新到新几何（右侧带拒绝命中）共 64 全过；clippy `-D warnings` 0；i18n 增词条 `第 {} / {} 项`→`#{} of {}`；改动区 fmt 干净。

## S61 页面切换：横向滑动 + 逐控件弹簧 ✅

- **需求**：页面切换从淡入淡出改为左右滑动；每个控件要有单独的阻尼和弹簧速度。
- **reactor 现状**：0.100 高层 API 只暴露 opacity/scale 过渡 + fade 出场，无平移过渡、无合成器弹簧入口（`GetElementVisual` 为 pub(crate)）。因此采用**闭式弹簧 + margin 位移**方案：无状态、按 `Instant` 求值，每 ~16ms 一个 `NavAnimTick` 触发 view 重建重算。
- **结构**：切页时新页、旧页在 Grid 双层叠放——`leave` 层渲染旧页（`page_view(from)`）整层滑出，`enter` 层整层 `dir·460px` 弹簧滑入（ζ=0.95, ω=8.5，约 400ms 可读行程），页面内部每个顶层块再叠加 `dir·(46+15i)` 的独立弹簧——**逐控件参数**：延迟 24ms·i、刚度 k=150−10i、阻尼 ζ=0.80+0.012i（越靠后越软越晚起步，形成级联）。`pagehost` 键恒定，`enter` 键跨切页不变——定局视图绝不重挂载，canvas swapchain 不闪。
- **方向**：`Page` 声明序即导航序，向高索引页 → dir=+1（新页右来、旧页左去），反向自动镜像。
- **覆盖式滑动**：进场层不透明（opacity 恒 1）——新页滑动时遮挡旧页，旧页 −120px 逆移 + ~500ms 淡出。避免了双半透明层"鬼影"（实测首版交叉透明时中帧文字糊成一团）；淡出尾巴压到 500ms 是因为新页未覆盖区域需要旧页垫底——否则 Mica 透出桌面（连拍实测验证）。
- **anim→rest 边界**：`enter` 层 Border 与每块的 Border 包装**无条件**存在（rest 时 margin=0）——否则落定瞬间元素类型跳变触发整子树重挂载，overview 的 5 个 swapchain canvas 会闪。
- **取舍**：margin 位移走布局路径而非合成器位移动画——reactor 0.100 无公开杠杆（`Translation`/`TranslationTransition` 仅在内部绑定层）；布局量每帧仅 ~15 个顶层块的 margin 变更，实测 debug 下 60fps 无压力。超过 10 块的页（长表）超出块直接静态，防级联失控。
- **测试钩子**：`GTT_NAVTEST=<label>@<ms>[@<alt>]` 首个快照后每 ms 交替切页 label↔alt——截图实测捕获飞行帧（注入输入到不了 WinUI3 content island，同 GTT_TIPTEST 先例）。
- **实测**（60 帧连拍）：总览→明细中帧显示表格从右滑入盖在旧页上、旧页左移淡出；明细→配额同效反向。飞行全程 ~15 帧（~320ms 可见运动 + ~600ms 弹簧收尾），DONE_MS=950 收敛。
- **验证**：64 测试全过、clippy `-D warnings` 0、改动区 fmt 干净（`useless_conversion` 三处 `.into()` 已清）。

## S62 关闭拦截：退出/挂托盘询问 + 可记忆默认 ✅

- **需求**：标题栏 X 不再直接退出——弹出询问「彻底退出 / 隐藏到托盘」，附「记住我的选择」与设置页默认项。
- **拦截机制**：reactor 0.100 的 `IAppWindow` 绑定裁掉了 `Closing` 事件（vtable 无可用包装），走 Win32 正路——`close_hook.rs` 用 `SetWindowSubclass` 挂 `WM_CLOSE`：吞掉消息并发 `Msg::CloseRequested`；UI 线程 TLS 存 `LocalSender`（子类 proc 与 UI 同线程，TLS 天然安全）；`WM_NCDESTROY` 卸子类清 TLS。`allow_next_close()` 一次性放行——选「彻底退出」或托盘菜单 Quit 置位后再 `request_close()`，防止自己拦自己死循环；`INSTALLED`/`ensure_installed` 幂等，view() 每帧兜底挂载。
- **对话框**：reactor `ContentDialog`（title + 说明 + `CheckBox` 记住选择；primary=彻底退出 / secondary=隐藏到托盘 / close=取消）。托盘缺失（GTT_NOTRAY 或安装失败）时 secondary `is_enabled=false` + 提示「托盘图标不可用」；已存 `close_action=tray` 但托盘缺席 → `CloseRequested` 落 `_` 分支弹框兜底，绝不静默丢进程。
- **持久化**：`UiConfig.close_action`（`""`问/`"quit"`/`"tray"`）；勾选记忆后按所选按钮写入，未勾选不改动既有默认。设置页「通用」新增「点击关闭按钮时」下拉（每次询问/直接退出/隐藏到托盘），可随时改回。
- **顺带修复**：`UiConfig::load` 容忍 UTF-8 BOM——实测 PowerShell/记事本手改 ui.json 会写入 BOM，serde_json 解析失败导致**全配置静默重置**；现 `trim_start_matches('\u{feff}')` 后再解析。
- **实测**（SendMessage(WM_CLOSE) 触发，UIA 驱动按钮）：
  - 默认 → 弹框（截图：标题/说明/CheckBox/三按钮，页面遮罩正常）
  - 隐藏到托盘 → 进程存活窗口消失；FindWindowW+ShowWindow 恢复 ✓
  - 取消 → 进程存活 ✓
  - 记忆+彻底退出 → 进程退出、`close_action="quit"` 落盘 ✓
  - 存 quit → WM_CLOSE 直接退出无弹框 ✓
  - 存 tray → WM_CLOSE 直接隐藏无弹框（含 BOM 文件，验证 BOM 修复）✓
  - GTT_NOTRAY → 弹框+「托盘图标不可用」+隐藏按钮置灰 ✓
  - 存 tray + GTT_NOTRAY → 回落弹框 ✓
- **取舍**：托盘菜单 Quit 走同一 `quit_now()`（已由对话框路径实测验证 allow-once 放行），未单独 E2E 托盘菜单点击（原生弹出菜单 UIA 不可达）。`DefSubclassProc` 调用包在 unsafe 块内（Rust 2024 unsafe-op-in-unsafe-fn）。
- **验证**：64 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。

## S63 切页动画性能：页面树缓存 + tick 提速（~46fps → ~70fps） ✅

- **症状**：左右滑动的切页动画明显不流畅。
- **定位**：每个 `NavAnimTick`（~16ms）都走 `update → view()`，view() **重建两整页**——enter 页全块 + leave 页全块，含所有 `TextBlock`/`format!`/`tr!`/canvas 闭包与概览页 5 个 swapchain 挂载描述；实测帧周期 ~21.6ms（16ms sleep + ~5.6ms 构建+diff），有效帧率仅 ~46fps，debug 下更低——这就是卡顿本源。reactor 0.100 无平移过渡/合成器杠杆（`ElementCompositionPreview::GetElementVisual` 为 pub(crate)，Translation/CanvasLeft 仅在 native 绑定层），margin 位移方案保留，问题收敛为"每帧重建太贵"。
- **方案**：`View` 是不可变声明树（`#[derive(Clone)]`，子树 Rc 共享 clone 廉价）——把"每帧重算的几何"与"不变的页面内容"分离：
  - 各 `*_page` 函数改为返回**原始顶层块 `Vec<View>`**（去掉 `anim` 参数与 `page_frame/vstack/slide_children` 收尾）；新增 `frame_page(theme, gap, blocks, anim)` 统一组装；`page_gap`：Overview/Settings=`section_gap`，列表页=10.0。
  - `NavAnim` 新增 `cache: RefCell<Option<NavCache>>`——首个动画帧 view() 里懒构建一次：`enter_blocks: Rc<Vec<View>>`（新页原始块）+ `leave: View`（旧页整装）。
  - 此后每帧 tick 只做：`Rc::clone` + `(*b).clone()` + ~15 个 margin Border + 两个层 Border——两页内容零重建；diff 走 PartialEq 深比较无分配。`enter`/`leave` 键恒定，swapchain 不重挂载。
  - tick sleep 16→10ms：周期 ~14ms（10ms + ~4ms 薄重建），每 60Hz vsync 前都有新帧。
- **实测**（`GTT_NAVTEST` 交替切页，`GTT_DEBUG` 日志帧数统计）：每趟飞行 950ms 内 **65–73 帧 ≈ 68–77fps**（优化前 ~44–48 帧 ≈ 46–50fps），含概览页在内的双页飞行均达标；落定后页面内容正常（缓存仅在飞行期间冻结，anim→None 后首帧即全新构建）。
- **取舍**：飞行中（≤950ms）enter/leave 页内容冻结——期间到达的 `Loaded`/`Tick` 在落定后的首个 rest 视图统一呈现，无可感知影响；再次点 nav 会丢弃旧 cache 重建一次。`RefCell`/`Rc` 限 UI 线程，Shell 本就 !Send。
- **验证**：64 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。

## S64 功能+动画改动复审：四处缺陷修复（PID 窗口定位/记忆框复位/Nav 兜底/缓存失效） ✅

- **审查范围**：S62 关闭询问 + S61/S63 滑动动画的全部落地代码。
- **发现并已修**：
  1. **多实例窗口误认**（真 bug）：`FindWindowW` 按标题全系统匹配——跑第二个 GTT 实例（如已装版+开发版）时，`close_hook` 的子类化会落到对方进程 HWND（跨进程子类化必败）→ 拦截永远装不上、X 直接退出；`tray.rs` 三处 hide/focus 同样可能操控错窗口。修复：`tray::main_hwnd()` 用 `EnumWindows`+`GetWindowThreadProcessId` PID 过滤 + 标题比对，四个调用点统一改用。
  2. **「记住我的选择」取消后不复位**（UX bug）：勾选后点取消，`close_remember` 残留 true → 下次弹框已预勾选。修复：`CloseDialogResult` 先取值再清零，复选框按打开生命周期消费。实测勾选→取消→重开 = Off ✓。
  3. **`Msg::Nav(None)`/未识别标签 → 跳总览**（潜在 bug）：`_ => Page::Overview` 兜底让 SelectorBar 清空选择时把用户传送回首页。修复：总览改显式 arm，`_ => prev` 原地不动。
  4. **飞行中数据冻结至落定**（正确性瑕疵）：`Loaded` 落地时 `enter` 缓存仍显示加载态。修复：`Msg::Loaded` 处 `*a.cache.borrow_mut() = None`，下一帧即新数据。
- **复审确认无问题项**：`close_proc` 同线程 TLS sender 合法；`WM_NCDESTROY` 内 RemoveWindowSubclass 文档允许；`allow_next_close` consume-once 语义正确（`replace(false)`）；托盘 Quit 与对话框共用 `quit_now` 已 E2E；spring 闭式解 v(0)=0 数学正确；缓存 RefCell 借用无嵌套冲突；`enter`/`leave` 键恒定保 swapchain；`Block i≥10` 防级联失控。
- **已知限制**（不本次修）：离场页 ScrollViewer 位置不可保留（reactor 无 scroll offset API）——旧页若已滚动，滑出时显示为顶部状态；S61 起即存在。`GTT_NAVTEST`/`GTT_DONUTTEST` 测试钩子保留在 release（env 触发、默认惰性）。`ensure_installed` 之前点击 X 会直关（启动头几帧窗口期，行为可接受）。
- **实测**：WM_CLOSE→对话框→勾选记忆→取消→重开未勾选 ✓；close_action=tray 直隐 + `main_hwnd` 恢复 ✓（EnumWindows 路径）。
- **验证**：64 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。

## S65 多核调度：EcoQoS 大小核分工 + 后台线程打标 ✅

- **范围**：`crates/core/src/power.rs`（新模块）+ engine 解析池 + ui 全部后台任务 + 托盘显隐切换进程级 QoS。
- **设计**（Win11 EcoQoS = 任务管理器"效率模式"同款机制，对异构 CPU 是调度**提示**而非硬亲和——不保证独占 E 核，但实测 API 接受且符合系统设计；硬亲和需要 CPU 拓扑枚举，收益不确定且损害可移植性，故不采用）：
  - `power::worker(name)`：`SetThreadDescription` 命名 + `SetThreadInformation(ThreadPowerThrottling, ctrl=EXECUTION_SPEED, state=EXECUTION_SPEED)` 线程级能效标——每个后台任务闭包首行调用：`gtt-scan`(启动/手动扫描)、`gtt-watch`(文件监视)、`gtt-tray`(托盘事件泵)、`gtt-timer`(周期刷新)、`gtt-quota`/`gtt-prices`(网络拉取)、`gtt-detail`(明细分页)、`gtt-minhide`/`gtt-navtest`。
  - `power::efficiency_process(on)`：进程级 `ProcessPowerThrottling`——`hide_main_window`/`try_hide_main_window` 置 on（整进程含 UI 线程入效率态，托盘驻留场景诉求正确），`focus_main_window` 置 off（前台恢复默认 QoS 让 UI 线程回性能核）。Quit 路径不必复位（进程即死）。
  - `eco_pool()`（engine.rs）：rayon 全局池线程惰性创建拿不到句柄无法打标——改用 `ThreadPoolBuilder::spawn_handler` 自建池，`OnceLock` 单例、并发数 `max(2, 核数/2)` 限宽（EcoQoS 是偏好非独占，限宽防超发溢回 P 核），worker 首行 `power::worker("gtt-parse")`。JSONL 解析阶段是唯一重 CPU 后台负载，落在专用能效池；DB 摄入维持单写者不变。
  - `gtt-anim`（NavAnimTick 链）：**只命名不打标**——10ms 帧节奏是延迟敏感负载，压 E 核会掉帧。
- **API 语义实测纠错**（直接探针程序在 Win11 26200 上验证）：文档暗示的 `ControlMask=0`（系统自动管理）形态在本机返回 `ERROR_INVALID_PARAMETER(87)`；正确形态是显式 `ControlMask=EXECUTION_SPEED` + `StateMask=1`启用/`0`禁用——Set 返回 TRUE。`GetThreadInformation`/`GetProcessInformation` 读回这两类信息同样返回 87（写-only），故不做读回断言。`GetLastError` 在 Set 成功后会返回陈旧错误码（6），仅失败时读取。首次失败打 `[power] api rejected err=N` 一次性日志（进程级 AtomicBool）。
- **跨进程实测**（Toolhelp32 线程枚举 + `GetThreadDescription`）：前台运行实例命名线程 `gtt-timer/gtt-tray/gtt-quota/gtt-prices`（watch 因无监视根目录未启动，非缺陷）；NAVTEST 动画期间抓到 `gtt-anim`×1 + `gtt-navtest`×7；`--minimized` 启动即隐藏路径：进程存活、`MainWindowHandle=0`、零 `[power]` 失败日志 → `efficiency_process(true)` 被 OS 接受。
- **发现的设计约束**（如实记录）：`spawn_background` 投递到 **Windows 线程池**（`windows_threading::submit`），线程复用——命名是"任务最后执行者"语义，短暂任务结束后池线程保留旧名直到复用（诊断上可接受）；EcoQoS 线程标同理持久于池线程，但因生态化任务几乎占满池使用，方向一致无害。
- **非 Windows**：全部 cfg 掉为 no-op；macOS 移植对应物是 GCD QoS class（`DISPATCH_QOS_CLASS_UTILITY`/`BACKGROUND`），注释已注明。
- **逃生门**：`GTT_NO_ECO=1` 禁掉一切打标便于对照诊断。
- **验证**：`cargo test --workspace` 66 过（含 2 个 Windows-only power 测试：Set 调用成功断言 + `GetThreadDescription` 读回命名）；clippy `-D warnings` 0；改动区 fmt 干净。EcoQoS 对实际核心落位的调度影响无法程序化读回（本机 Get*Information 不支持）——以 Set 成功 + 无失败日志为准，不声称已验证"真的跑在 E 核"。

## S66 切页动画提速：缩短飞行、收敛淡出尾巴 ✅

- **诉求**：左右切换整体偏慢、淡出残影感明显 → 提速 + 让"滑动"盖过"淡变"。
- **参数调整**（widgets.rs `NavAnim`）：
  - 进层 `enter_layer`：ω 8.5→13.5、ζ 0.95→0.97（近临界，460px 行程 ~280ms 落定、几乎无回弹）
  - 块级 `block(i)`：起步延迟 24i→14i ms、行程 46+15i→38+11i、刚度 k 150−10i→170−12i（ω≈13.0→9.5 渐软）、ζ 0.80+0.012i→0.82+0.010i
  - 退场 `exit`：位移 −120→−90px、弹簧 ω 15→20、淡出 `1−2t`(500ms)→`1−3.2t`(312ms)
  - 总时长 `DONE_MS` 950→700
- **淡出尾巴缩短的风险权衡**：原 500ms 尾巴是给新页未覆盖区域"垫底"防 Mica 透桌面；312ms 时新页已覆盖 ~95%+，左侧残余窄条内旧页残影很淡（实测中段帧无可见桌面透出、无重影文字）。
- **实测**：`GTT_NAVTEST` 交替切页帧数统计 44–49 帧/700ms ≈ 66–70fps（debug 构建）——帧率不变、飞行时长 −26%；中段截图确认进层盖入、内容不透明。
- **验证**：66 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。主观节奏仍以实机点击为准——再嫌慢调 `enter_layer` 的 ω（13.5↑）或 `DONE_MS`。

## S67 完整效率模式适配：EcoQoS + IDLE 优先级 + 定时器降权 + 低内存优先级 ✅

- **诉求**：对齐 Windows 任务管理器「效率模式」的完整语义（此前 S65 只做了 EcoQoS 一项）。
- **`efficiency_process` 扩展为四件套**（托盘隐藏全量开启、前台恢复全量关闭）：
  1. `ProcessPowerThrottling` EXECUTION_SPEED——调度偏好能效核（原有）
  2. **`IGNORE_TIMER_RESOLUTION`**（ControlMask|=4）——隐藏态进程不得把系统定时器钉在高分辨率（电池消耗源）；前台恢复默认值保证动画 tick 可请求紧节奏
  3. **`SetPriorityClass(IDLE_PRIORITY_CLASS)`**——所有线程基优先级降 idle，隐藏态任何后台扫描/监视都让位给系统前台工作；恢复 NORMAL
  4. **`MEMORY_PRIORITY_LOW`**（ProcessMemoryPriority=2）——内存压力下我们的页先被回收；刻意不用 `EmptyWorkingSet`（强制换页会让恢复瞬间付硬缺页代价）
- **实测证据**：
  - 单测 `efficiency_mode_toggles_priority_class`：`GetPriorityClass` 可读回——`efficiency_process(true)` 后断言 = `IDLE_PRIORITY_CLASS`、`(false)` 后 = `NORMAL_PRIORITY_CLASS` ✓（效率模式四件套中唯一程序化可读回的一条腿，验证了整个调用链真实执行）
  - `--nocapture` 全量跑：零 `[power]` 失败日志 → 组合掩码 EXEC|IGNORE_TIMER_RES(=5)、`SetPriorityClass`、`ProcessMemoryPriority` 三个新调用全部被本机 Win11 26200 接受
  - **外部可见**：`Get-Process` 读运行实例——`--minimized` 隐藏态 `PriorityClass=Idle`（与任务管理器显示一致）
- **设计说明**：IDLE 基优先级 + EcoQoS 下托盘驻留的扫描/拉取变慢是特性不是缺陷——正是效率模式语义；`gtt-tray` 空闲优先级线程在用户点击时照常唤醒（idle≠挂起）。线程级 `efficiency_thread` 保持仅 EXECUTION_SPEED（线程类无 IGNORE_TIMER_RES 常量）。
- **验证**：67 测试全过（+1）、clippy 0、fmt 干净。

## S68 切页动画再提速：退场纯淡出早摘层 + tick 加密 ✅

- **复盘定位**：再次确认合成器平移无路——bindings vtable 中 `SetTranslation`/`SetTranslationTransition` 槽位被裁为 `usize` 占位，`request_*` 命令式通道只有 composition-host/swapchain/webview/native-source，**拿不到已挂载元素的属性直写**。margin 是唯一杠杆，优化转向「减少每帧布局工作量」。
- **结构性优化**：
  - **退场层 margin→纯 opacity**：opacity 是合成级属性不走布局，原来每帧 margin 位移让整个旧页树 arrange 纯浪费——现在淡出期间旧页树零布局开销
  - **淡出归零即摘层**：`exit()` 返回 `Option<f64>`，opacity≤0 时不再 push "leave" 层——原来 opacity=0 的子树还在 tree 里白占 ~400ms 的每帧 diff+布局
  - 两笔叠加：旧页树的每帧成本从「margin+opacity 双写+arrange」降到「一个 opacity 属性写」，且 250ms 后彻底消失
- **提速参数**：`DONE_MS` 700→600；进层 ω 13.5→15、行程 460→430px；块级延迟 14i→12i、行程 38+11i→34+10i、k 170−12i→180−13i；退场淡出 312→250ms（`1−4t`）；tick sleep 10→8ms（两处 spawn 同步）。
- **实测**（GTT_NAVTEST 交替切页 + 帧数统计）：38–48 帧/600ms ≈ **63–80fps**（debug 构建），飞行时长再 −14%；中段截图确认进层不透明盖入、无重影、无桌面透出（250ms 时新页已覆盖 ~97%）。
- **已知上限**（如实）：margin 布局路径的每帧 arrange 无法完全消除——XAML `Translation`（GPU 路径）的 vtable 槽位存在但绑定被裁；除非上游放开 `ElementCompositionPreview::GetElementVisual` 或补 `SetTranslation` 绑定，否则已到本框架内极限。剩余可调项只有弹簧参数与 DONE_MS。
- **验证**：67 测试全过、clippy `-D warnings` 0、改动区 fmt 干净。

## S69 切页卡顿根治：大表虚拟化 + 页面数据与扫描解耦 + 停顿钳制时钟 + 总览图表延后挂载 ✅

- **诉求**：切页动画在条目多的页面卡顿，怀疑加载策略——要求让加载更高效或异步化，使动画不受加载拖累。
- **定位（release，真实 63k 事件账本，`GTT_NAVTEST` 交替切页 8 趟、去掉首趟）**：`view()` 最慢仅 0–1ms（S63 的树缓存已榨干 Rust 侧），耗时全在 view 之后的原生 XAML 挂载/布局。三个根因：
  1. 明细 200 行 / 价格 400 行为非虚拟化 `StackPanel`，每行 ~20 个元素，一次性同步挂载 4000–8000 元素，且每帧 margin 位移都要重新 arrange；
  2. 切 数据源/价格 时 `start_scan` 先跑完整 `scan_once`（~300ms）+ 总览/明细聚合，数据恰好在动画中段落地，`Msg::Loaded` 还会**清空动画缓存**→ 飞行中途整页重建重挂载；
  3. 动画时钟是墙钟 `t0.elapsed()`——UI 线程任何一次停顿之后弹簧被"跳过"一截，观感即跳帧。
  另有一项在修完前三项后由对照实验暴露：**总览的 5 个 D2D canvas**——`windows_canvas::canvas_invalidated` 每个 canvas 各自 `GpuDevice::new_or_warp`（各建一套 D3D11+D2D 设备，~15ms/个，且无共享设备的按需重绘变体），挂载后首个布局时落在 UI 线程 ≈ **85ms 冻结**，出现在所有到/离开总览的飞行中。
- **改动**：
  1. **表格虚拟化**（`pages.rs` `virtual_rows`）：明细/价格改 `ItemsRepeater` + `VirtualSource`，行闭包只为可视索引执行（~25 行）；数据改 `Arc<Vec<_>>`（`DetailBundle.rows`、`PriceTable.rows`）避免每次 `view()` 复制；键=行索引，`key_revision` 恒 0——同长度换内容只 reconcile 已实化行，长度变化才重置集合（reactor `reconcile_virtual_collection` 源码核实）。
     - **行宽问题（首版回归，用户截图发现）**：reactor 用 `ContentControl` 承载每个虚拟行，其 `HorizontalContentAlignment` 默认 Left，行按**内容宽度**布局，`*` 列塌缩、数字列与全宽表头错位。首个修法 `min_width(16384)`（假设 XAML 会把期望宽度钳到可用宽度）**被实测证伪**：行真的变成 16384 宽，数字列被推出卡片裁掉。最终方案：卡片内放 1-DIP 宽度尺子（沿用总览已验证的 `SwapChainPanel` Metrics 机制），`Msg::SetTableWidth` 取整 DIP 上报，行以测得宽度 `width()` 铺满；初值按默认窗口估算（1180−48−2·pad−2）避免首帧跳变。
  2. **页面数据与扫描解耦**（`main.rs`）：`sources`/`prices`/`prices_synced_at` 移出 `Snapshot`，成为 Shell 字段，由轻量 `load_page(page)`（只 `Store::open` + 单表查询，不扫描）经 `Msg::PageData` 回填；首个快照落地后**预取**两页，`Nav` 不再 `start_scan`（stale-while-revalidate：先渲染已有数据，后台补刷）；每页单飞 + `page_again` 标记合并重复请求；`PricesDone` 成功后刷新价目表。`Msg::Loaded` 不再无条件清动画缓存——仅"首个快照"（缓存里只有扫描占位）或 `PageData` 落到仍为空的入场页时才清，其余保持冻结到落定（S63 语义）。
  3. **停顿钳制动画时钟**（`widgets.rs` `NavAnim`）：`clock` 按 tick 累加，单步增量封顶 `MAX_DT=32ms`（约 2 个 vsync，正常帧率抖动仍保持实时）；UI 停顿只会让动画暂停而不是跳过；墙钟 1500ms 硬上限兜底防无限拉长；`view()` 只读 `clock`，两次 tick 间任意次重绘看到一致的动画时间。新增 `advance()` 同时承载帧间隔探针。
  4. **总览图表延后 + 错峰挂载**：飞行期间 `canvas_ready=0`，趋势图/环图以同尺寸占位（外层 Border 的尺寸/背景/指针事件不变，只把 canvas 槽换成空 Border，`defer_slot`）；落定后每 16ms 放行一个（趋势图→4 个环图，`Msg::CanvasStage`），单个 UI 回合最多多付 ~15ms，不再一次吃 5 个 GPU 设备创建。新一次飞行会重置并中断错峰。
- **诊断**：`GTT_DEBUG` 下每次飞行结算日志增加 首帧延迟 / 最坏帧间隔 / 最慢 `view()`（`[nav] anim settled: N frames in Xms (first gap …, worst gap …, worst view() …)`）。
- **实测（release，`GTT_NAVTEST` 每目标页 8 趟去首趟，"帧"=动画 tick，间隔为 UI 线程消息循环 tick→tick，非 GPU 呈现时间）**：

  | 目标页↔总览 | 旧：帧/墙钟 | 旧：首帧延迟 | 旧：最坏帧间隔 | 新：帧/墙钟 | 新：首帧延迟 | 新：最坏帧间隔 |
  |---|---|---|---|---|---|---|
  | 明细 | 38.3 / 602ms | 65ms（max 81） | 107ms（max 114） | 65.7 / 608ms | 25ms（max 43） | **30ms（max 31）** |
  | 价格 | 25.3 / 603ms | 100ms（max 106） | 166ms（max 180） | 65.7 / 608ms | 24ms（max 44） | **30ms（max 31）** |
  | 配额 | — | — | ~88ms | 66.6 / 620ms | 25ms（max 47） | 40ms（max 42） |
  | 数据源 | — | — | ~90ms | 66.6 / 614ms | 41ms（max 45） | 18ms（max 32） |
  | 配额↔数据源（不含总览，对照） | — | — | — | 68.0 / 604ms | 32ms | **8ms（max 9）** |

  分步归因：仅前三项时明细/价格已到 ~90fps、最坏间隔 86ms；第 4 项（总览 canvas 延后）把最坏间隔从 86ms 压到 ≤42ms。对照行证明去掉总览后飞行零冻结、墙钟恰为 600ms（无停顿被钳制）。价格页帧率 42→~108（tick 频率，受 8ms sleep 步进限制，高于 60Hz vsync，有意义的指标是最坏帧间隔）。
- **视觉验证（PrintWindow 截图，非仅帧数）**：明细/价格首屏满宽、数字列与表头对齐、斑马纹全宽；2000px 高窗口一次实化 ~66 行且内容正确；820px 窄窗口行宽随窗口收缩；数据源页正常；总览静止态与"明细→总览"飞行落定后趋势图 + 4 环图均在；明细翻页两条路径（同长度换内容：第 2/316 页，含更高的 `unpriced` 徽章行；长度变化重置：末页 200→3 行）均正确——用临时 `GTT_DETAILPAGE` 钩子驱动，验完已撤，未留在代码里。
- **验证命令**：`cargo test --workspace` 71 过（core 62 + ui 9，新增 4 个 `nav_clock_tests`：停顿封顶单步 / 正常帧距保持实时 / 完成判定 / 墙钟硬上限）；`cargo clippy --workspace --all-targets -- -D warnings` 0；`cargo fmt --check -p globaltokentracker-ui` 仅剩 HEAD 既有的 2 处漂移（`pages.rs` `frame_page`、`settings_page`，非本步改动，未动）。
- **已知限制 / 未验证（如实）**：
  - **滚动后的实化未能程序化验证**：UIA `ScrollPattern.Scroll` 会挂起、`SetScrollPercent` 被拒（`E_INVALIDARG`），注入输入到不了 content island。替代证据是视口变化时的实化（2000px 窗口）与 UIA 读到的视口占比 13.1%（总高度估算正确）。**建议实机滚动明细页确认**。
  - 虚拟行外壳 `ContentControl` 的 `MinHeight=24`（reactor 内部 `ESTIMATED_ROW_HEIGHT`）使未实化区域按 24px/行估算、实际行 ~27px，滚动条滑块长度会随滚动轻微调整。
  - 图表落定后依次出现（~5×16ms 级联）是有意的；启动首屏与非切页路径不受影响（`canvas_ready` 初值 `usize::MAX`）。
  - 价格表 400 行上限现在只限制可浏览范围、不再影响挂载成本，可放开（涉及 i18n 文案"前 400 条"，未擅自改行为）。
- **范围外发现（未改）**：`Msg::DetailPage` 查询失败走 `Msg::Failed`，会重复 `arm_refresh` 累积刷新定时器；价目表首行是一条模型名为空的 litellm 记录（数据本身）。
- **流程教训**：首版只用帧数验证，漏掉了渲染回归（行宽），由用户截图发现——之后所有 UI 改动补上截图核对。

## S70 切页动画：去淡出 + 位移交给合成器（GPU）驱动 ✅

- **诉求**：切换动画不要淡出；图形绘制尽量转到 GPU 负载。
- **核实**：reactor 0.100 的合成器属性只有 `opacity`/`scale`（及 `*_transition`），无平移（S68 结论成立）。突破口是 `Grid` 的 **Composition host**：`observe_composition_host` 给出窗口的 lifted `Compositor`，`request_set_child_visual` 可挂一个子 Visual；**合成提交后该子 Visual 的 `parent()` 就是宿主元素自己的 Visual**——实验：把它的 `Offset` 平移 300px，整页（标题+卡片+整张表）一起动而筛选栏不动。再配合 `windows-composition`（`reactor` 特性，与 reactor 同属 windows-core 0.100）的 `Compositor::from_host` / `ScalarKeyFrameAnimation` / `Visual::start_animation`，即可让 DWM 合成线程执行位移。
- **设计**（新模块 `gpu_slide.rs`）：
  - **推入式转场**：新页整页宽度从一侧滑入，旧页同弹簧同步推出（两页始终相距恰好一页宽 → 无重叠、无残影，**不需要淡出**，淡出及其 `opacity` 逻辑整体删除）。
  - **逐控件弹簧保留**：每个页面块（前 `MAX_SLIDE=10`）一条合成器关键帧动画，参数沿用 S66/S68 的每块阻尼/刚度/延迟/位移，由弹簧闭式解**预采样 48 帧**（线性插值，末帧精确落在终值）。只动画 `Offset.X`，Y 仍归布局。层弹簧临界阻尼 ω=16（无过冲）。
  - **UI 线程逐帧工作降为零**：删除 8ms tick 链、逐帧 `view()` 重建、margin 布局位移、冻结页面缓存（`NavAnim`/`NavCache`/停顿钳制时钟及其 4 个测试一并移除）。
  - **双层轮换**：两个页面层 `layer0/layer1` 轮流承载页面（`rest_idx`）。切页时新页挂到另一层、先 `opacity=0`（不是淡入：就绪的同一提交里 0→1 一步到位，防止首帧在终点位置闪一下），旧页**原树保留**（图表真实、滚动位置保留——顺带解决 S64 记录的"离场页滚动位置不可保留"）；结束后旧层卸载、新层即静止层（同 key 不重挂载，swapchain 不闪）。
  - **布局全程不变**：动画覆盖在布局 Offset 之上。飞行中改布局会让 XAML 重写 Offset 把动画顶掉，且把入场层布局偏出屏外会使 `ItemsRepeater` 认为自己不可见而只实化 1 行——两者均在实测中踩到。
  - **总览图表**：挂载提前到滑动后半段（启动后 300ms 起每 16ms 一个）。合成器动画与 UI 线程解耦，GPU 设备创建不再拖动画；旧页图表全程保持真实。
  - **降级**：合成器不可用（宿主未就绪/COM 失败，`windows-composition` 内部大量 `unwrap()`，全部包在 `catch_unwind`）→ 直接切页；`GTT_NO_GPU_SLIDE=1` 可强制关闭用于诊断。
- **实测踩坑（如实）**：
  1. 首版在 `NavGo` 同一回合启动动画并把布局 margin 归零 → XAML 重写 Offset，入场页从第一帧就在终点、与旧页重叠（旧页因无布局变更而正常滑出）；
  2. 改成"布局偏出屏外 + 全程保持"后，虚拟表滑入期间只有 1 行、落定才补齐；改为布局不动 + 首帧不透明度 0 解决；
  3. 旧页用"离场副本"时图表占位，开滑前 ~75ms 图表先消失；改双层轮换后 Go 延迟 98–115ms → 19–30ms、图表全程保留；
  4. 快速连点（200–700ms 间隔）使 GPU 滑动永久失效：同 key 层换角色不重挂载、不再触发 `Ready`，而 `reset()` 清空了子 Visual；改为只清缓存的元素 Visual、保留子 Visual。
- **验证证据**：
  - 每种节奏（1500/700/350/230/200ms，明细/价格/配额/数据源/设置 ↔ 总览）8/8 次飞行全部走 GPU 路径、0 失败、无 panic；快速打断后最终页面在静止位置（总览含全部图表 / 价目表满宽）。
  - 连拍接触表（PrintWindow ~16ms/帧，非仅日志）逐帧确认：推入过程两页首尾相接、无重叠无淡出、表格滑动中行完整、返回飞行图表在页面将就位时出现。
  - **CPU（进程 `TotalProcessorTime`，8 次连续切页窗口，旧=已安装的 a78ece4）**：轻页之间 旧 328–984ms → 新 63–188ms（≈5–10×）；涉及总览的切页两版基本持平（新 875–1625ms / 旧 859–1844ms，噪声大）——该部分由总览页自身挂载主导（见下）。
  - **GPU**：进程在 RTX 4060 Ti 上占用 85.5MB 专用显存（`GPU Process Memory` 计数器），确认 D2D 图表用的是硬件设备而非 WARP（`new_or_warp` 硬件优先）。图表本就走 GPU；这次把滑动位移也搬到合成器。
  - `cargo test --workspace` 72 过（core 62 + ui 10，新增 5 个 `gpu_slide` 测试：首末帧/终值精确、进度严格递增、块延迟保持、推入两页恒距一页宽、临界阻尼无过冲）；clippy `-D warnings` 0；fmt 仅剩 HEAD 既有的 `pages.rs` `settings_page` 1 处漂移（未动）。
- **已知限制 / 未做**：
  - 总览页挂载仍要创建 5 个 GPU 设备：`windows_canvas::canvas_invalidated` 每个 canvas 各建 D3D11+D2D 设备且无共享设备的按需重绘变体。把 4 个环图合并成一个 canvas 可省 ~3 次设备创建，未做（改动图表结构）。
  - 飞行期间（~0.7s）命中测试按静止布局位置计算；被新导航打断时到达页直接归位（`snap_home`）。
  - 只在本机（Win11 26200 + 1 块 NVIDIA 独显）验证；旧版 Windows / 其它 GPU / 高 DPI 未测。合成器路径不可用时降级为直接切页而非旧的 margin 动画（已删除该路径）。
  - 未做：位移之外的绘制（文字/卡片圆角等）本就由 XAML 合成走 GPU，无可迁移项。
