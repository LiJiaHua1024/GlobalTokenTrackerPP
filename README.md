<div align="center">

<img src="assets/icon.png" width="96" height="96" alt="GlobalTokenTracker++">

# GlobalTokenTracker++

**聚合本机 AI 编码工具用量的 Google Material Design 3 桌面应用 (Flutter Desktop + Rust)**

**简体中文** · [English](README_EN.md)

<br>

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-00AEEC?style=flat-square&labelColor=2d3a55)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-1.85+-DEA584?style=flat-square&logo=rust&logoColor=white&labelColor=2d3a55)](https://www.rust-lang.org)
[![Flutter](https://img.shields.io/badge/Flutter-3.29+-02569B?style=flat-square&logo=flutter&logoColor=white&labelColor=2d3a55)](https://flutter.dev)
[![UI](https://img.shields.io/badge/UI-Material%20Design%203-4285F4?style=flat-square&logo=google&logoColor=white&labelColor=2d3a55)](#核心特性)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4?style=flat-square&logo=windows&logoColor=white&labelColor=2d3a55)](#环境要求)
[![Release](https://img.shields.io/github/v/release/LiJiaHua1024/GlobalTokenTrackerPP?include_prereleases&style=flat-square&label=release&labelColor=2d3a55)](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases)
[![Issues](https://img.shields.io/github/issues/LiJiaHua1024/GlobalTokenTrackerPP?style=flat-square&labelColor=2d3a55)](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/issues)

[核心特性](#核心特性) · [功能全景](#功能全景) · [支持的源](#支持的源) · [安装与使用](#安装与使用) · [本地构建](#本地构建) · [隐私与安全](#隐私与安全说明)

</div>

---

<details>
<summary><b>目录</b></summary>

- [项目简介](#项目简介)
- [核心特性](#核心特性)
- [功能全景](#功能全景)
- [支持的源](#支持的源)
- [环境要求](#环境要求)
- [安装与使用](#安装与使用)
- [本地构建](#本地构建)
- [隐私与安全说明](#隐私与安全说明)
- [许可证](#许可证)
- [致谢](#致谢)

</details>

---

## 项目简介

**GlobalTokenTracker++** 是一个现代化的 Windows 桌面端用量分析中心，基于 **Google Material Design 3 (Flutter Desktop)** 与 **Rust 高性能统计引擎** 深度重塑打造。

它可以无感知、增量扫描本机各主流 AI 编码工具在磁盘上留下的日志与凭据，集中呈现精确的 Token 消耗、按市场最新价目表估算的美元成本、订阅额度/配额使用率、响应延迟与模型分布。所有数据全部存储在本机 SQLite 账本中，绝无隐私遥测与数据回传。

---

## 核心特性

- 🎨 **Google Material Design 3 原生视觉与动效**
  - **沉浸式无缝标题栏**：无缝融入 Windows 10/11，支持标题栏拖拽移动、双击最大化与标准窗口控制。
  - **动态色彩体系 (Tonal Palette)**：内置 Google Blue、Emerald Green、Deep Violet 等 5 套 M3 动态基调色，支持明暗模式与跟随系统。
  - **现代组件交互**：优雅的 NavigationRail 侧边导航、平滑切页动画与 Material 3 卡片布局。

- 📊 **交互式用量与成本占比分布 (Pie & Donut Charts)**
  - 按工具、按模型维度统计成本与 Token 消耗，支持一键在“实心饼图 ↔ 环形图”之间丝滑切换。
  - 扇区鼠标悬停放大、双向联动高亮，长模型名称智能外置于卡片副标题与图例，彻底杜绝文字越界遮挡。
  - 环形图中心区域作为 KPI 锚点，数值自适应等比缩放。

- ⚡ **0ms 极致“跟手”流畅性能**
  - **Rust C-ABI FFI + Background Isolate**：数据检索与反序列化运行在后台 OS 独立线程，UI 主线程帧率恒定 60~144fps。
  - **内存级乐观缓存**：切换时间范围（今日 / 近 7 天 / 近 30 天 / 全部）0ms 瞬时切换无卡顿。
  - **视口虚拟化滚动**：明细列表与价目表采用 `ListView.builder` 局部视口虚拟化，上万条数据加载耗时低于 5ms。

- 🔢 **智能大数字紧凑呈现 (K / M / B / T)**
  - 针对百万、十亿级 Token 智能归整缩写；设置页支持一键切换完整数值精确显示或紧凑模式。

---

## 功能全景

<table>
<tr>
<td width="50%" valign="top">

#### 统计与视图

- **总览**：4 大核心 KPI 指标、4 组饼图/环形图分布、30 日用量趋势柱状图、订阅配额进度卡片
- **明细**：事件级全量明细表（时间、工具、模型、各桶 Token、成本、耗时）
- **价格表**：内置与云端同步的各大模型输入/输出价格，支持按模型即时搜索
- **配额与数据源**：多模型配额用量仪表盘与本机工具扫描状态感知

</td>
<td width="50%" valign="top">

#### 计量与计价

- 非缓存输入 / 缓存读 / 缓存写 / 输出 四桶严格精确计量
- `llmpricing.dev` 价目表云端自动同步 + 离线种子兜底，断网仍可精确计价
- USD 成本估算与订阅 Credits / 消耗百分比严格隔离，不混淆计算
- 工具 + 模型级联筛选持久化

</td>
</tr>
<tr>
<td width="50%" valign="top">

#### 可靠性与数据安全

- **增量字节游标扫描**：日志追写只读新段，自动识别日志截断与轮转
- **双代快照热备**：账本自动双备份，损坏/异常打开时秒级自愈
- **只读扫描**：绝不回写或篡改任何工具的本地数据文件
- **历史有界化**：配额变动滚动保全最新状态

</td>
<td width="50%" valign="top">

#### 本地集成与接口

- 手动增量扫描与后台数据监视
- 内置 OTLP/HTTP 接收器（`127.0.0.1:4318`）直接接收遥测工具上报
- 绿色便携，单目录独立解压即跑，无需管理员权限

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
> 未安装的工具会在扫描时自动跳过；工具清理自身缓存或日志不影响已入账的历史统计。

---

## 环境要求

| 项 | 要求 |
|---|---|
| 操作系统 | Windows 10（1809+）/ Windows 11，64位（x64） |
| 运行依赖 | **绿色便携，无需安装额外运行时或组件，解压即跑** |
| 权限要求 | 普通用户权限即可（无需管理员权限） |
| 磁盘占用 | 约 30 MB |

---

## 安装与使用

你可以根据使用习惯任选以下方式之一：

### 选项 A：单文件图形安装包（推荐）
从 [GitHub Releases](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases) 下载 `GlobalTokenTrackerPP-Setup-<版本>-win-x64.exe`：
- **独立隔离**：默认独立安装至 `%LOCALAPPDATA%\Programs\GlobalTokenTrackerPP`，与原版软件完全隔离，无任何覆盖或冲突。
- **开箱即用**：自动创建开始菜单与桌面快捷方式，支持添加用户 PATH。
- **免管理员提权**：基于绿色原生 Win32/GDI 安装引擎，无需管理员权限，无需 Windows App Runtime。
- **干净卸载**：支持在 Windows “已安装的应用” 中一键平滑升级与干净移除。

```powershell
双击运行              # 打开暗黑风图形向导安装
--quiet               # 静默后台安装
--dir D:\Tools\GTT_PP # 自定义安装目录
--uninstall           # 卸载移除（保留用户本地账本）
```

### 选项 B：免安装便携版 (Portable Zip)
从 [GitHub Releases](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases) 下载 `GlobalTokenTrackerPP-v<版本>-windows-x64.zip`：
1. 解压压缩包到任意本地目录（如 `D:\Tools\GlobalTokenTrackerPP`）；
2. 双击运行 `globaltokentracker_ui.exe` 即可直接进入现代化 Material 3 仪表盘！

---

## 本地构建

本项目由两个核心部分组成：
1. **Rust C-ABI 共享库 (`crates/ffi`)**：负责高效扫描磁盘日志并提供本地 SQLite 聚合接口；
2. **Flutter 桌面客户端 (`flutter_ui`)**：负责 Google Material Design 3 界面与图形渲染。

### 1. 编译 Rust 核心 FFI 动态库
```powershell
git clone https://github.com/LiJiaHua1024/GlobalTokenTrackerPP.git
cd GlobalTokenTrackerPP

# 编译 C-ABI 共享动态链接库
cargo build --release -p globaltokentracker-ffi

# 将产出的 DLL 复制到 Flutter 运行目录
Copy-Item target\release\globaltokentracker_ffi.dll flutter_ui\
```

### 2. 运行与构建 Flutter 桌面客户端
```powershell
cd flutter_ui

# 安装依赖
flutter pub get

# 本地调试运行
flutter run -d windows

# 打包发布 Release
flutter build windows --release
```
编译产物位于 `flutter_ui\build\windows\x64\runner\Release\`。

---

## 隐私与安全说明

- **100% 纯本地运行**：没有任何第三方数据收集或遥测上报。
- **唯一外部网络请求**：向 `llmpricing.dev` 同步公共模型价目表（断网时自动启用本地离线种子，完全不影响核心功能）。
- **OTLP 接收器仅监听本地回环**（`127.0.0.1:4318`），不接受外部网络连接。
- **源文件绝对只读**：应用绝不会修改、回写或删除各 AI 编码工具的任何原始日志。
- **本地账本安全存储**：数据账本保存在 `%USERPROFILE%\.globaltokentracker\ledger.db`。

---

## 许可证

本项目遵循 **MIT OR Apache-2.0** 双重开源许可协议：[LICENSE-MIT](LICENSE-MIT) · [LICENSE-APACHE](LICENSE-APACHE)。

---

## 致谢

- 灵感来源与参考：[jichuo1/GlobalTokenTracker](https://github.com/jichuo1/GlobalTokenTracker)、[cc-switch](https://github.com/farion1231/cc-switch)、[TokenTracker](https://github.com/xiufengsun/TokenTracker)、[cursor-usage](https://github.com/chocolatemale/cursor-usage)
- 公共价目数据：[llmpricing.dev](https://llmpricing.dev) · [models.dev](https://models.dev) · [LiteLLM](https://github.com/BerriAI/litellm)
