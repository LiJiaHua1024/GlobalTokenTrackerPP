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

## 待办（S9+）
- [ ] 托盘"最小化到托盘"依赖 reactor 暴露 window hide 或 HWND（上游）
- [ ] 趋势图升级 windows-canvas Direct2D
- [ ] Grok/WorkBuddy/CodeBuddy/Gemini 适配器（P1-P2）
- [ ] OTel 接收器 + wham/Qoder/Cursor 配额通道（M2/M3）
- [ ] daily_rollups 生成任务、CSV 导出、prune（M4）
- [x] ~~Codex 验收门~~ → 过，Δ=0.01%（tokens_used=会话水位终值语义锁定）
