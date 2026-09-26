# CodeLedger / GlobalTokenTracker — 工程实施计划

> 依据 `docs/spec-v2.0.md`（v2.0 调研方案）。技术栈按新要求调整：**纯 Rust + WinUI 3（windows-reactor）**，替代原方案的 Tauri+React。
> 日期：2026-09-26 ｜ 状态：进行中

## 1. 关键技术决策（已联网核验）

| 决策 | 选择 | 依据 |
|---|---|---|
| UI 框架 | **windows-reactor 0.100**（微软官方 crates.io，kennykerr，2026-09-03 发布） | 纯 Rust 声明式 WinUI 3：Component/View + hooks；控件覆盖 NavigationView/TabView/ListView/ItemsRepeater/VirtualSource/WebView2/InfoBar/TeachingTip/ContentDialog 等 90+ 组件 |
| 图表 | **windows-canvas**（Direct2D）经 `SwapChainPanel` 嵌入 | 官方同族 crate；柱状/环形/热力图手绘，无 WebView 依赖 |
| WinAppSDK 部署 | **framework-dependent + 引导安装**（windows-reactor-setup） | 本机已装 WinAppRuntime 1.5–2.5；目标机缺运行时时引导安装。self-contained 备选（+~60MB，牺牲体积） |
| 托盘 | `tray-icon` crate | 跨平台（Windows/macOS/Linux），与未来 Mac 版一致 |
| 数据库 | `rusqlite`（bundled SQLite，WAL） | 无外部 C 依赖，schema 按 spec §5 |
| JSON 解析 | `serde_json` 流式（BufRead 逐行） | 稳定优先；后期性能不够再上 `simd-json` feature |
| 异步 | `tokio` | 文件监听/轮询/HTTP |
| 时间 | `jiff` | chrono 的现代继任者，本地午夜对齐 rollup 必需 |
| 全依赖策略 | **一律最新发布版**，`cargo add` 取最新 | 用户明确要求 |

### 为什么不是 windows-rs 裸写 WinUI3
XAML 编译器只支持 C#/C++，纯 Rust 只能 code-only 手写控件树（社区样例 sotanakamura/winui-rust 可行但脆弱）。windows-reactor 是微软官方为这条路做的封装，带 reconcile 引擎和控件 builder，稳定性远高于手写 COM 事件管道。

### Mac 兼容策略（核心约束）
**引擎与外壳严格分离**：
- `core` crate 零 Windows 依赖 —— 解析、归一化、计价、SQLite、同步调度全部可移植。
- UI 只做渲染：所有 ViewModel（页面数据、格式化文本、图表点列）由 core 产出，UI 不碰业务逻辑。
- Mac 版 = 新外壳（SwiftUI 经 FFI 或 Tauri/eGPUI 二选一）+ 同一个 core。**预计移植工作量集中在 UI 层**。

## 2. 工作区结构

```
GlobalTokenTracker/
├── crates/
│   ├── core/        # codeledger-core —— 跨平台引擎（spec §4-§8 的全部逻辑）
│   │   ├── src/model.rs       # UsageEvent/CostSource/Provenance/InputSemantics
│   │   ├── src/store/         # SQLite schema + 迁移 + 查询（ViewModel SQL 也在这里）
│   │   ├── src/adapters/      # 12 个 SourceAdapter（P0 先做 4 个）
│   │   ├── src/normalize.rs   # §7.1 字段变体归一 + §7.2 模型别名 BFS
│   │   ├── src/pricing/       # models.dev + LiteLLM + 覆写层 + 离线种子
│   │   ├── src/sync/          # 字节游标 + 尾部指纹 + notify 监听 + 60s 轮询
│   │   └── src/engine.rs      # 管线编排：scan → normalize → price → upsert → rollup
│   ├── cli/         # codeledger-cli —— scan/report/reconcile/prices（开发验证 + 高级用户）
│   └── ui/          # codeledger-ui —— windows-reactor WinUI3（cfg windows only）
└── docs/            # spec-v2.0.md（原始调研）、PLAN.md、REVIEW.md（每步审查记录）
```

## 3. 阶段划分与验收（对齐 spec §10，逐步 review）

| 步骤 | 内容 | Review 门（不过不进下一步） |
|---|---|---|
| **S0** 地基 | Rust stable-msvc 工具链；workspace；CI 友好的 fmt/clippy 配置 | `cargo build` 绿、`cargo clippy -- -D warnings` 绿 |
| **S1** 存储层 | §5 全部 8 张表 + WAL + UPSERT 规则（避开 cc-switch #6994） | 单元测试：upsert 覆盖语义、游标截断钉 EOF 不重放 |
| **S2** Claude 适配器 | JSONL 流式解析 + **message.id 去重取最后一条** + 计量 gate | 本机 26 文件全量跑：**与 cc-switch 误差 <1%**（基线 0.1%） |
| **S3** Codex 适配器 | rollout JSONL token_count 增量 + rate_limits→quota + state_5.sqlite 校验 | `tokens_used` 与明细差值 <0.5% |
| **S4** OpenCode+ZCode | opencode.db session 表（provider_reported cost）；zcode model_usage + 跨源跳过规则 | 行数对上（model_usage 4759 行）；provider∈{anthropic,openai,google} 行被跳过且计数 |
| **S5** 计价管线 | pricedata/models.dev.json 做内置种子 + LiteLLM 分档 + price_overrides + 别名 BFS | 本机 15 模型名：13 命中 + 2 unpriced 徽标，不猜价 |
| **S6** CLI 报表 | `scan`/`report daily`/`reconcile`（vs cc-switch） | 三类对账全绿，输出贴进 REVIEW.md |
| **S7** WinUI3 壳 | reactor App + NavigationView 五页骨架 + Mica/亚克力 + 托盘 | 窗口可开、读真库、深色浅色切换 |
| **S8** 总览页+明细页 | 卡片/环形图/趋势（canvas）+ 四维透视 + raw_ref 溯源展开 | 与 CLI 数字一致 |
| **S9+** | Grok/WorkBuddy/CodeBuddy/Gemini 适配器、OTel、配额通道、CSV 导出、prune | 按 spec M1–M4 验收 |

## 4. 设计红线（spec 中的铁律，写进代码注释与测试）

1. Claude 流式多行必须按 `message.id` 去重取最后 —— 不去重会多算 2.1 倍。
2. `INSERT OR IGNORE` 禁用 —— `ON CONFLICT(dedup_key) DO UPDATE WHERE 新行更完整`。
3. 游标截断/指纹不符 → 钉到 EOF，绝不重放。
4. 成本一律自己算，官方字段只作 provenance 参照；unpriced 标徽标不猜价。
5. API 美元与订阅 credits/百分比绝不混算。
6. 只读用户文件与凭据，永不外传，日志脱敏。

## 5. 风险记录

- windows-reactor 是 0.x 新 crate（刚发 23 天）：API 可能变动 → 钉 `=0.100.0` 精确版本，升级单独评审。
- WinUI3 托盘需走 Win32 Shell_NotifyIcon + AppWindow 显隐（tray-icon crate 处理）。
- Charts 若 canvas 集成遇阻 → 降级用 Rectangle/Grid 组合出柱状/热力图（够用且零依赖）。
