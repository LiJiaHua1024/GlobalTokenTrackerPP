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
- **与 spec 验收门偏差说明**：state_5.tokens_used=3.31B 是"每线程最终累计/窗口水位"语义（≈Σfinals 3.47B），不是逐调用消耗；spec 的 <0.5% 门建立在该语义理解之前，两口径不可直接比。我们的 5.90B 是"逐调用真实处理量"口径；订阅配额消耗以 rate_limits 的 used_percent 为准（已入 quota_snapshots）。
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

## 待办（S8+）
- [ ] 托盘（tray-icon 事件循环接到 reactor pump）
- [ ] 趋势图升级 windows-canvas Direct2D
- [ ] Grok/WorkBuddy/CodeBuddy/Gemini 适配器（P1-P2）
- [ ] OTel 接收器 + wham/Qoder/Cursor 配额通道（M2/M3）
- [ ] daily_rollups 生成任务、CSV 导出、prune（M4）
- [ ] notify 监听 + 60s 轮询常驻（UI 壳内）
- [ ] Codex 口径差异 2.18% 继续收敛（spec 基线语义修正后重定验收门）
