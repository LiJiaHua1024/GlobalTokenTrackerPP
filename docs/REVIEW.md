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

## 待办（S7+）
- [ ] windows-reactor UI 壳：NavigationView 五页、Mica、托盘、明暗主题
- [ ] Grok/WorkBuddy/CodeBuddy/Gemini 适配器（P1-P2）
- [ ] OTel 接收器 + wham/Qoder/Cursor 配额通道（M2/M3）
- [ ] daily_rollups 生成任务、CSV 导出、prune（M4）
- [ ] notify 监听 + 60s 轮询常驻（UI 壳内）
