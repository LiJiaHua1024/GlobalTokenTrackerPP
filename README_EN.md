<div align="center">

<img src="assets/icon.png" width="96" height="96" alt="GlobalTokenTracker">

# GlobalTokenTracker

**A WinUI 3 desktop app that aggregates usage from every AI coding tool on your machine**

[简体中文](README.md) · **English**

<br>

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-00AEEC?style=flat-square&labelColor=2d3a55)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-1.90-DEA584?style=flat-square&logo=rust&logoColor=white&labelColor=2d3a55)](https://www.rust-lang.org)
[![Windows](https://img.shields.io/badge/Windows-10%201809%2B-0078D4?style=flat-square&logo=windows&logoColor=white&labelColor=2d3a55)](#requirements)
[![UI](https://img.shields.io/badge/UI-WinUI%203-00AEEC?style=flat-square&labelColor=2d3a55)](#requirements)
[![Release](https://img.shields.io/github/v/release/jichuo1/GlobalTokenTracker?include_prereleases&style=flat-square&label=release&labelColor=2d3a55)](https://github.com/jichuo1/GlobalTokenTracker/releases)
[![Issues](https://img.shields.io/github/issues/jichuo1/GlobalTokenTracker?style=flat-square&labelColor=2d3a55)](https://github.com/jichuo1/GlobalTokenTracker/issues)

</div>

---

## Overview

GlobalTokenTracker unifies local usage stats from AI coding tools: token counts, estimated USD cost from a live price book, subscription quota/credit balances, latency and model distribution. It incrementally scans on-disk logs **read-only** into a local SQLite ledger — nothing is uploaded.

The UI is native WinUI 3 (Rust + windows-reactor), built for small binary size, fast cold start and stable tray-resident operation. The statistics engine is decoupled from the UI and designed with future macOS portability in mind.

## Features

- Five views: Overview / Detail / Quota / Sources / Prices, with persistent tool+model filtering
- Exact bucketing: non-cached input / cache-read / cache-write / output tokens
- Price book synced from `llmpricing.dev` every 12 h (once forced per launch) with an offline seed fallback
- USD estimates kept strictly separate from subscription credits/percentages
- Incremental byte-cursor scanning; truncation/rotation handled; read-only on all sources
- Ledger snapshot backup (two generations) with open-time self-healing
- Periodic or file-watch-only refresh modes; tray-resident
- In-place upgrades: the installer remembers custom dirs and stops running processes
- OTLP/HTTP receiver on `127.0.0.1:4318` for tools that emit telemetry

## Supported sources

| Tool | Form | Level |
|---|---|---|
| Claude Code | Local JSONL logs | ✅ exact + quota (when credentialed) |
| Codex | Local sessions | ✅ exact + quota |
| Devin | OTLP receiver | ✅ exact |
| OpenCode / ZCode / Grok / WorkBuddy / MiniMax Code / Kimi Code | Local logs | ✅ exact |
| Cline | `ui_messages.json` + editor globalStorage | ✅ exact (self-reported cost) |
| Command Code | Local JSONL | ✅ exact (self-reported cost) |
| CodeBuddy IDE | `state.vscdb` / sessions | 🟡 session-level + model multipliers |
| Cursor / Qoder | Credentials + official quota APIs | 🟡 quota-oriented |

Uninstalled tools are skipped and marked on the Sources page. Deleting a tool's local data does not lose already-ledgered stats.

## Requirements

Windows 10 **1809+** x64 · Windows App Runtime 1.5+ (auto-installed by the setup on first run; may trigger UAC once) · no admin rights needed · ~20 MB disk.

## Install & update

Grab `GlobalTokenTracker-Setup-<ver>-win-x64.exe` from [Releases](https://github.com/jichuo1/GlobalTokenTracker/releases). Double-click for the GUI, or `--quiet` / `--dir <path>` / `--uninstall` for console control. To upgrade, just run the newer installer — it reuses your install location (custom dirs included), stops running processes and preserves all user data under `%USERPROFILE%\.globaltokentracker`.

> [!WARNING]
> The package is currently unsigned — SmartScreen will warn ("More info → Run anyway"). Verify the SHA-256 on the Release page.

## Build

```powershell
cargo build --release --workspace   # Rust 1.90+, edition 2024 + Windows SDK
cargo test --workspace
powershell -ExecutionPolicy Bypass -File installer\package.ps1   # single-file setup → dist\
```

## Privacy

The only outbound traffic is the public price-book fetch from `llmpricing.dev` (offline seed covers failure). The OTLP receiver binds loopback only. Source logs are never modified. User data lives in `%USERPROFILE%\.globaltokentracker` and survives updates and uninstalls.

## License

Dual-licensed under **MIT OR Apache-2.0** — [LICENSE-MIT](LICENSE-MIT) · [LICENSE-APACHE](LICENSE-APACHE).

## Acknowledgements

Research references (no code reused): [cc-switch](https://github.com/farion1231/cc-switch) · [TokenTracker](https://github.com/xiufengsun/TokenTracker) · [cursor-usage](https://github.com/chocolatemale/cursor-usage) · [tokcat](https://github.com/handlecusion/tokcat) · [agent-trail](https://github.com/camtrik/agent-trail) · [windows-reactor](https://crates.io/crates/windows-reactor) · pricing data from [llmpricing.dev](https://llmpricing.dev), [models.dev](https://models.dev), [LiteLLM](https://github.com/BerriAI/litellm).
