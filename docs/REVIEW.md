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
- unpriced 不猜价：cost=0+cost_source='unpriced'（本机 codex-auto-review、gpt-reserve、custom-alpha 会命中）。
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
