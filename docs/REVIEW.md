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
- 诊断：`CL_DEBUG=1` 才开 stderr 日志；`CL_NOTRAY` 跳过托盘安装。
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
- **Claude OTel 引导** ✅：`codeledger otel-setup` 合并写 `~/.claude/settings.json` env 块（serde_json `preserve_order` 保住 cc-switch 键序）；实测既有键全保留。
- **配额轮询器** ✅（`quota.rs`，tokcat 源码级复核）：
  - Codex wham `GET /wham/usage`：Bearer + `ChatGPT-Account-Id`；`last_refresh>8d` 或 401/403 触发 OAuth 刷新回写 auth.json；primary/secondary 按 `limit_window_seconds` 分 `5h_block`/`weekly`；实测 **weekly 0% + reset 命中真数据**。
  - Cursor `POST api2.cursor.sh .../GetCurrentPeriodUsage`（Connect RPC JSON，`Connect-Protocol-Version:1`）：`planUsage` lenient 双形数字、cents→USD、auto/api 池分行；实测 3 行落库。
  - UI 30 分钟低频门（`CL_NO_QUOTA` 关），CLI `codeledger quota` 手动。UI 实测 4 行 0 错。
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
- **实现**：新 crate `codeledger-setup`——`include_bytes!` 内嵌 `payload.zip`（build.rs 兜底 22B 空 zip，dev 构建不受影响），运行时 zip-deflate 解压到 `%LOCALAPPDATA%\Programs\CodeLedger`。
- **安装动作**：WinAppRuntime 检测（`Get-AppxPackage Microsoft.WindowsAppRuntime*`）→ 缺失则提示下载微软官方 aka.ms 安装包（ureq+rustls）→ taskkill 旧实例 → 释放文件 → 复制自身为卸载器 → WScript.Shell 快捷方式（powershell COM，免引 windows crate COM 面）→ HKCU `Uninstall\CodeLedger` 注册（DisplayVersion/EstimatedSize/QuietUninstallString）→ 用户 PATH 追加（去重）。
- **卸载**：`--uninstall [--dir]`——杀进程、删快捷方式、删 HKCU 项、PATH 回滚、detach cmd `rmdir` 自删目录（程序运行中 exe 不可删 → 延迟 cmd）；用户数据 `~/.codeledger` 明确保留。
- **踩过的坑（实测抓出）**：
  1. `Command::arg()` 对 cmd `/C` 字符串做 `\"` 转义 → cmd 读到字面 `\"` 解析失败，自删静默不执行——改 `raw_arg()` 原样透传。
  2. UninstallString 原先不带 `--dir`——自定义目录安装后走注册表卸载会清错路径——始终显式携带。
- **产出**：`installer/package.ps1` 一条命令：release 构建 → Compress-Archive 打 payload → 嵌包构建 → `dist\CodeLedger-Setup-<ver>-win-x64.exe`（**8.78MB**）+ SHA256。
- **实测**：默认路径 install/uninstall 全链路、带空格 `--dir` 路径 install/uninstall（注册表串直接复用验证）、快捷方式/PATH/注册表落点逐项核对、CLI 安装后报表出真数据。
- **边界**：WinAppRuntime 安装步骤本身可能弹 UAC（微软安装器行为，非我们可控）；`--quiet` 自动接受运行时安装；dev 桩 exe 拒绝安装并提示走 package.ps1。

## 审计增量（S10+S11 后，2026-09-27 复测）

| 指标 | 实测 | 判定 |
|---|---|---|
| 安装器体积 | `CodeLedger-Setup-0.1.0-win-x64.exe` 8.78MB（双 exe deflate 压缩） | ✅ |
| 联网价目刷新 | 2.4s（dev 7750 行 + litellm 3635 行 + 落库 + reprice 扫描），仅 UI 启动时 24h 陈旧时后台跑 | ✅ 不阻塞首帧 |
| UI 启动/内存 | release 正常起窗，121.7MB RSS 与上轮一致（刷新在 load_all 内非阻塞） | ✅ 无回归 |
| 新增出站端点 | models.dev + raw.githubusercontent.com（固定 HTTPS，ureq/rustls） | ✅ |
| 卸载完整性 | 自删目录含运行中 exe（raw_arg/cmd 延迟 rmdir）、注册表/PATH/快捷方式全清、带空格路径验证过 | ✅ |
| 供应链 | zip 8.6.0（2026-04-25）/ winreg 0.55 / ureq 3.4.2，均远超 7 天沉淀 | ✅ |
| 测试 | 15/15 绿，clippy 全工作区 0 警告 | ✅ |

