<div align="center">

<img src="assets/icon.png" width="96" height="96" alt="GlobalTokenTracker">

# GlobalTokenTracker

**聚合本机 AI 编码工具用量的 WinUI 3 桌面应用**

**简体中文** · [English](README_EN.md)

<br>

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-00AEEC?style=flat-square&labelColor=2d3a55)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-1.90-DEA584?style=flat-square&logo=rust&logoColor=white&labelColor=2d3a55)](https://www.rust-lang.org)
[![Windows](https://img.shields.io/badge/Windows-10%201809%2B-0078D4?style=flat-square&logo=windows&logoColor=white&labelColor=2d3a55)](#环境要求)
[![UI](https://img.shields.io/badge/UI-WinUI%203-00AEEC?style=flat-square&labelColor=2d3a55)](#环境要求)
[![Release](https://img.shields.io/github/v/release/jichuo1/GlobalTokenTracker?include_prereleases&style=flat-square&label=release&labelColor=2d3a55)](https://github.com/jichuo1/GlobalTokenTracker/releases)
[![Issues](https://img.shields.io/github/issues/jichuo1/GlobalTokenTracker?style=flat-square&labelColor=2d3a55)](https://github.com/jichuo1/GlobalTokenTracker/issues)

[功能](#功能) · [支持的源](#支持的源) · [环境要求](#环境要求) · [安装与更新](#安装与更新) · [使用](#使用) · [构建](#构建项目) · [隐私](#隐私说明)

</div>

---

<details>
<summary><b>目录</b></summary>

- [项目简介](#项目简介)
- [功能](#功能)
- [支持的源](#支持的源)
- [环境要求](#环境要求)
- [安装与更新](#安装与更新)
- [使用](#使用)
- [构建项目](#构建项目)
- [隐私说明](#隐私说明)
- [许可证](#许可证)
- [致谢](#致谢)

</details>

---

## 项目简介

GlobalTokenTracker 是一个 Windows 桌面程序，用来统一统计本机各 AI 编码工具的用量：token 计数、按价目表估算的美元成本、订阅配额/额度余量、延迟与模型分布。它只读取各工具写在磁盘上的本地日志与状态文件做增量统计，数据全部存放在本机 SQLite 账本中，过程不产生网络上报。

---

## ✨ GlobalTokenTracker++ (Material Design 3 重构版)

本项目是独立维护的增强版（Plus Plus），在保留 Rust 原生高可靠本地统计账本核心的同时，彻底重塑了用户界面与交互体验，采用 **Google Material Design 3 (Flutter Desktop for Windows)** 构建：

- 🎨 **Google Material Design 3 现代设计语言**：无缝融入 Windows 10/11 的自定义沉浸式标题栏、动态色彩基调（Tonal Palette）、深浅主题自由切换与精致动效。
- 📊 **全息用量与成本占比分布**：全新增加实心饼图（Pie Chart）与环形图（Donut Chart）一键动态无缝切换，支持按工具与模型维度统计、鼠标悬停扇区联动高亮及防遮挡指标锚点。
- ⚡ **完全跟手的极致性能**：底层通过高效的 C-ABI 共享库（FFI）在后台 OS Isolate 线程中并发完成 SQLite 聚合与数据反序列化；结合内存高速缓存机制，切换时间跨度（今日 / 近7天 / 近30天 / 全部）实现 0ms 瞬时跟手反馈。
- 🚀 **虚拟化高吞吐列表渲染**：彻底解决传统表格在数千条模型价格和海量事件日志下的卡顿问题，采用视口虚拟化，加载与滚动帧率稳如 60~144fps。
- 🔢 **大额数值智能缩写 (K / M / B / T)**：支持将百万、十亿级长数字自动格式化，并在设置中支持一键切换完整数值/紧凑模式。

---

## 功能

<table>
<tr>
<td width="50%" valign="top">

#### 统计视图

- 总览 / 明细 / 配额 / 数据源 / 价格 五页视图
- 工具 + 模型级联筛选，选择持久化
- 30 日 token 趋势条（Direct2D 按需绘制）
- 单事件粒度明细：模型、各桶 token、成本、耗时、来源文件

</td>
<td width="50%" valign="top">

#### 计量与计价

- 非缓存输入 / 缓存读 / 缓存写 / 输出 分桶精确计量
- `llmpricing.dev` 价目表每 12h 同步 + 每次启动强制一次
- 离线种子兜底，断网仍可计价
- USD 估算与订阅 credits/百分比严格分列，不混算

</td>
</tr>
<tr>
<td width="50%" valign="top">

#### 可靠性与数据安全

- 增量字节游标扫描：日志追写只读新段，截断/轮转自动识别
- 账本双代快照备份，文件丢失/损坏打开时自愈
- 只读扫描，绝不回写任何源文件
- 配额历史有界化（30 天滚动，每键保最新行）

</td>
<td width="50%" valign="top">

#### 体验与集成

- 定时刷新 / 仅文件变更监视 双模式
- 系统托盘常驻，开始菜单快捷方式，可选加入用户 PATH
- 安装器原位升级：自定义目录记忆、运行进程自动处理
- OTLP/HTTP 接收器（`127.0.0.1:4318`）接入支持遥测的工具

</td>
</tr>
</table>

---

## 支持的源

| 工具 | 数据形态 | 计量级别 |
|---|---|---|
| Claude Code | 本地 JSONL 会话日志 | ✅ token 精确 + 配额（凭据可用时） |
| Codex | 本地会话日志 | ✅ 精确 + 配额 |
| Devin | OTLP 遥测接收 | ✅ 精确 |
| OpenCode / ZCode / Grok / WorkBuddy | 本地日志 | ✅ 精确 |
| MiniMax Code / Kimi Code | 本地日志 | ✅ 精确 |
| Cline | `ui_messages.json` + 编辑器 globalStorage | ✅ 精确（含工具自报成本） |
| Command Code | 本地 JSONL | ✅ 精确（含工具自报成本） |
| CodeBuddy IDE | `state.vscdb` / 会话库 | 🟡 会话级 + 模型倍率 |
| Cursor / Qoder | 凭据 + 官方配额 API | 🟡 配额为主 |

> [!NOTE]
> 未安装的源自动跳过并在"数据源"页标记；工具删本地数据不影响已入账统计（账本在源之外）。精确度以"数据允许"为上限——源文件里没有的字段绝不编造。

---

## 环境要求

| 项 | 要求 |
|---|---|
| 系统 | Windows 10 **1809+**（build 17763），x64 |
| 运行时 | Windows App Runtime 1.5+——**安装程序自动检测并引导安装**（首次需联网，可能弹一次 UAC） |
| 权限 | 不需要管理员，装到 `%LOCALAPPDATA%\Programs\GlobalTokenTracker` |
| 磁盘 | 约 20 MB |

---

## 安装与更新

从 [Releases](https://github.com/jichuo1/GlobalTokenTracker/releases) 下载 `GlobalTokenTracker-Setup-<版本>-win-x64.exe`：

```
双击                  → 图形界面安装（可选目录/快捷方式/PATH）
--quiet               → 静默安装
--dir D:\Tools\GTT    → 自定义目录
--uninstall           → 卸载（保留 %USERPROFILE%\.globaltokentracker 用户数据）
```

**升级**：直接运行更新版本的安装程序即可原位更新——会记住你上次的安装目录（包括自定义目录），自动结束运行中的进程，更新卸载注册项。更新过程不碰账本与配置。

> [!WARNING]
> 安装包当前未签名，SmartScreen 会提示"Windows 已保护你的电脑"——点"更多信息 → 仍要运行"即可。下载后可在 Release 页核对 SHA-256。

---

## 使用

GUI 开箱即用（开始菜单 → GlobalTokenTracker）。同时附带 CLI（安装时勾选"加入用户 PATH"后可直接调用）：

```
globaltokentracker-cli scan             # 手动全量扫描
globaltokentracker-cli prices --update  # 强制刷新价目表
globaltokentracker-cli quota            # 立即轮询配额通道
globaltokentracker-cli prune --days 90  # 裁剪历史事件
globaltokentracker-cli export out.csv   # 导出明细 CSV
```

刷新频率与刷新模式（定时 / 仅文件变更）在界面"设置"里调整。

---

## 构建项目

```powershell
git clone https://github.com/jichuo1/GlobalTokenTracker.git
cd GlobalTokenTracker

# 需要 Rust 1.90+（edition 2024）与 Windows SDK
cargo build --release --workspace   # 产出 ui/cli/setup 三个 exe
cargo test --workspace              # 测试
cargo clippy --all-targets -- -D warnings

# 打包单文件安装程序（内嵌 payload → dist\）
powershell -ExecutionPolicy Bypass -File installer\package.ps1
```

签名属可选增强：`installer/sign.ps1` 支持证书指纹 / PFX（测试）/ 云签 dlib 三种模式，经环境变量驱动，详见 [docs/SIGNING.md](docs/SIGNING.md)。

---

## 隐私说明

- **唯一的网络请求**是向 `llmpricing.dev` 拉取公开价目表（每 12 小时一次 + 每次启动一次）；断网时回落内置离线种子，功能不受影响。
- **OTLP 接收器仅监听 `127.0.0.1:4318`**，只收本机回环，不接受局域网连接。
- 所有源日志**只读**，应用不会修改、删除或上传任何工具的数据文件。
- 账本与配置存于 `%USERPROFILE%\.globaltokentracker`，卸载和升级均保留；程序目录内不存任何用户数据。

---

## 许可证

按 **MIT OR Apache-2.0** 双许可证分发，任选其一遵守：[LICENSE-MIT](LICENSE-MIT) · [LICENSE-APACHE](LICENSE-APACHE)。

---

## 致谢

调研与实现过程中参考（未直接引用代码）：

- [cc-switch](https://github.com/farion1231/cc-switch) · [TokenTracker](https://github.com/xiufengsun/TokenTracker) · [cursor-usage](https://github.com/chocolatemale/cursor-usage) · [tokcat](https://github.com/handlecusion/tokcat) · [agent-trail](https://github.com/camtrik/agent-trail)
- [windows-reactor](https://crates.io/crates/windows-reactor)（Rust ↔ WinUI 3 反应式绑定）
- 价目数据来源：[llmpricing.dev](https://llmpricing.dev) · [models.dev](https://models.dev) · [LiteLLM](https://github.com/BerriAI/litellm)
