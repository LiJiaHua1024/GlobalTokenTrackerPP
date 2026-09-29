<div align="center">

<img src="assets/icon.png" width="96" height="96" alt="GlobalTokenTracker++">

# GlobalTokenTracker++

**A Google Material Design 3 desktop app for aggregating AI coding tool usage on your machine (Flutter Desktop + Rust)**

[简体中文](README.md) · **English**

<br>

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-00AEEC?style=flat-square&labelColor=2d3a55)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-1.85+-DEA584?style=flat-square&logo=rust&logoColor=white&labelColor=2d3a55)](https://www.rust-lang.org)
[![Flutter](https://img.shields.io/badge/Flutter-3.29+-02569B?style=flat-square&logo=flutter&logoColor=white&labelColor=2d3a55)](https://flutter.dev)
[![UI](https://img.shields.io/badge/UI-Material%20Design%203-4285F4?style=flat-square&logo=google&logoColor=white&labelColor=2d3a55)](#key-features)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-0078D4?style=flat-square&logo=windows&logoColor=white&labelColor=2d3a55)](#requirements)
[![Release](https://img.shields.io/github/v/release/LiJiaHua1024/GlobalTokenTrackerPP?include_prereleases&style=flat-square&label=release&labelColor=2d3a55)](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases)
[![Issues](https://img.shields.io/github/issues/LiJiaHua1024/GlobalTokenTrackerPP?style=flat-square&labelColor=2d3a55)](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/issues)

[Key Features](#key-features) · [Supported Sources](#supported-sources) · [Download & Usage](#download--usage) · [Building from Source](#building-from-source) · [Privacy](#privacy--security)

</div>

---

## Overview

**GlobalTokenTracker++** is a modern Windows desktop dashboard built with **Google Material Design 3 (Flutter Desktop)** and backed by a high-performance **Rust core engine**.

It incrementally and non-intrusively aggregates local AI coding tool telemetry: token consumption, estimated USD cost according to real-time market pricing, subscription quotas/credits, request latencies, and model distributions. Everything is persisted locally in an on-disk SQLite database — zero external telemetry, zero privacy tracking.

---

## Key Features

- 🎨 **Google Material Design 3 Aesthetics & Motion**
  - **Seamless Window Header**: Blends with Windows 10/11 native look, supports dragging, double-click maximization, and full window controls.
  - **Dynamic Theming**: 5 Material 3 Tonal Palette seed colors, dark/light modes, and system sync.
  - **Modern Layout**: Smooth transitions, NavigationRail sidebar, and responsive cards.

- 📊 **Interactive Usage & Cost Distribution (Pie & Donut Charts)**
  - Toggle between solid Pie charts and Donut charts with a single click.
  - Slices highlight and explode on hover; long model names are displayed cleanly in the card header and legend chips, eliminating label overflow.
  - Donut center hole functions as a KPI anchor with auto-scaling text.

- ⚡ **Zero-Lag, Instant UI Responsiveness**
  - **Rust C-ABI FFI + Background Isolate**: SQLite queries and deserialization execute off the main thread; UI remains pinned at 60~144fps.
  - **Optimistic In-Memory Cache**: Switching time ranges (Today / 7 Days / 30 Days / All) responds with 0ms latency.
  - **Virtualized Lists**: Details and pricing tables load tens of thousands of rows in <5ms using `ListView.builder`.

- 🔢 **Intelligent Large Number Formatting (K / M / B / T)**
  - Automatically abbreviates large token quantities into K, M, B, and T formats with an instant toggle switch in Settings.

---

## Supported Sources

| Tool | Format | Level |
|---|---|---|
| Claude Code | Local JSONL session logs | ✅ Exact tokens + Quota (when credentialed) |
| Codex | Local session logs | ✅ Exact + Quota |
| Devin | OTLP telemetry receiver | ✅ Exact |
| OpenCode / ZCode / Grok / WorkBuddy | Local logs | ✅ Exact |
| MiniMax Code / Kimi Code | Local logs | ✅ Exact |
| Cline | `ui_messages.json` + editor globalStorage | ✅ Exact (with tool-reported cost) |
| Command Code | Local JSONL | ✅ Exact (with tool-reported cost) |
| CodeBuddy IDE | `state.vscdb` / session store | 🟡 Session-level + model multipliers |
| Cursor / Qoder | Credentials + official quota APIs | 🟡 Quota-oriented |

---

## Requirements

- **OS**: Windows 10 (1809+) / Windows 11, x64
- **Dependencies**: Portable standalone package; no external runtimes required (no Windows App Runtime needed).
- **Disk**: ~30 MB.

---

## Download & Usage

Download `GlobalTokenTrackerPP-v<version>-windows-x64.zip` from [GitHub Releases](https://github.com/LiJiaHua1024/GlobalTokenTrackerPP/releases):
1. Extract to any directory.
2. Launch `globaltokentracker_ui.exe`.

---

## Building from Source

```powershell
git clone https://github.com/LiJiaHua1024/GlobalTokenTrackerPP.git
cd GlobalTokenTrackerPP

# 1. Build Rust FFI shared library
cargo build --release -p globaltokentracker-ffi
Copy-Item target\release\globaltokentracker_ffi.dll flutter_ui\

# 2. Run or build Flutter Desktop
cd flutter_ui
flutter pub get
flutter run -d windows
flutter build windows --release
```

---

## Privacy & Security

- **100% Local**: No external telemetry or data collection.
- **Sole Network Request**: Synced model prices from `llmpricing.dev` (with built-in offline seed fallback).
- **Read-Only**: Original logs are strictly untouched.
- **Ledger Path**: `%USERPROFILE%\.globaltokentracker\ledger.db`.

---

## License

Dual-licensed under **MIT OR Apache-2.0**: [LICENSE-MIT](LICENSE-MIT) · [LICENSE-APACHE](LICENSE-APACHE).
