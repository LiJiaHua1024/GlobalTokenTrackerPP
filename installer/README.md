# GlobalTokenTracker — 安装

聚合本机 AI 编程工具用量（tokens / 费用 / 配额 / 延迟）的桌面应用。

## 要求

- Windows 10 **1809 (build 17763)** 或更高，x64
- **Windows App Runtime 1.5+**（GUI 依赖；缺失时安装程序会引导下载微软官方安装包）
- 约 20 MB 磁盘；**无需管理员权限**（装到 `%LOCALAPPDATA%\Programs\GlobalTokenTracker`）

## 安装

双击 `GlobalTokenTracker-Setup-<ver>-win-x64.exe`，或静默：

```
GlobalTokenTracker-Setup-0.1.0-win-x64.exe --quiet
GlobalTokenTracker-Setup-0.1.0-win-x64.exe --dir D:\Tools\GlobalTokenTracker
```

安装动作：释放 `globaltokentracker-ui.exe` / `globaltokentracker-cli.exe` / `globaltokentracker-setup.exe`
（卸载器）→ 开始菜单快捷方式 → HKCU 卸载注册项 → 加入用户 PATH。

## 卸载

开始菜单 → "Uninstall GlobalTokenTracker"，或设置 → 应用 → GlobalTokenTracker，或：

```
"%LOCALAPPDATA%\Programs\GlobalTokenTracker\globaltokentracker-setup.exe" --uninstall
```

程序文件、快捷方式、PATH 项与卸载注册项都会被移除；**用户数据保留**在
`%USERPROFILE%\.globaltokentracker`（ledger.db、ui.json），不需要可手动删除。

## 构建安装包

```powershell
powershell -ExecutionPolicy Bypass -File installer\package.ps1
```

产出 `dist\GlobalTokenTracker-Setup-<ver>-win-x64.exe`（附 SHA256）。

## CLI

| 命令 | 作用 |
|---|---|
| `globaltokentracker-cli scan` | 增量扫描所有数据源 |
| `globaltokentracker-cli report all` | 聚合报表 |
| `globaltokentracker-cli quota` | 轮询订阅配额（Codex wham / Cursor） |
| `globaltokentracker-cli prices --update` | 联网拉取最新模型价目并重估未计价事件 |
| `globaltokentracker-cli export all --out out.csv` | 导出明细 CSV |
| `globaltokentracker-cli otel-setup` | 给 Claude Code 写入 OTel 遥测 env |

数据文件位于 `%USERPROFILE%\.globaltokentracker`；OTel 接收器只监听 `127.0.0.1:4318`。
