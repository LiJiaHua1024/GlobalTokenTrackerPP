# 代码签名指南

目标：让 `GlobalTokenTracker-Setup-*.exe` 的签名在收件人机器上显示"已验证的发布者"并让 SmartScreen 信誉随发布持续累积。

> **2025 年规则修正（本文档按当前事实校准）**
>
> - **EV ≠ SmartScreen 秒过**：2024-03 起微软取消 EV 的即时信誉待遇，EV/OV/云签一律按发布者身份累积信誉——花钱买 EV 不再买来"首发免警告"，主要差别只剩 UAC 显示的组织名与内核驱动签名资格。
> - **公共 CA 不再签发 PFX**：2023-06 CA/B 新规后，所有公共可信代码签名私钥必须存于 FIPS 140-2 硬件——但"硬件"可以是**云 HSM**（eSigner、KeyLocker、Artifact Signing），无需实物 USB token。
> - **Azure Artifact Signing（原 Trusted Signing）有区域限制**：个人仅美国/加拿大，组织仅美/加/欧盟/英国——国内主体/个人**不可用**。

## 管线已就绪

`installer/package.ps1` 内置两处签名（见 `installer/sign.ps1`）：

1. `payload.zip` 打包**之前**签 `globaltokentracker-ui.exe` / `-cli.exe`（内嵌二进制本身带签名）
2. dist 落盘后签 `GlobalTokenTracker-Setup-<ver>-win-x64.exe`（SmartScreen 检查的外壳）

签名配置全部走环境变量，仓库里不放任何证书/密钥：

| 环境变量 | 用途 |
|---|---|
| `GTT_SIGN_SHA1` | 证书指纹（token 或云 HSM 适配器装入 `Cert:\CurrentUser\My` 后可用；私钥不出 HSM） |
| `GTT_SIGN_PFX` + `GTT_SIGN_PFX_PASS` | **仅限自签/测试证书**——公共 CA 自 2023-06 起不再签发可导出私钥的 PFX |
| `GTT_SIGN_DLIB` + `GTT_SIGN_DLIB_METADATA` | 云签名 dlib 插件模式（如 Azure Artifact Signing 的 `Azure.CodeSigning.Dlib.dll` + `metadata.json`） |
| `GTT_SIGN_TSA` | 时间戳服务器，默认 `http://timestamp.digicert.com` |

用法：

```powershell
$env:GTT_SIGN_SHA1 = '<证书指纹>'
powershell -ExecutionPolicy Bypass -File installer\package.ps1
```

每个文件签后自动 `signtool verify /pa` 校验；全部带 RFC3161 时间戳（证书过期后签名依然有效）。

## 方案对比（无硬件 / 有硬件）

| 方案 | 约价 | 硬件 | SmartScreen | 适配国内主体 |
|---|---|---|---|---|
| **不签名** | $0 | — | 警告可绕过（"仍要运行"） | ✅ 个人分发够用 |
| **SSL.com eSigner + OV/EV** | 证书 ~$250+/年（EV ~$350） | 无——SSL 云 HSM 托管私钥 | 信誉累积 | ✅ 可买，签 EV 级无 token |
| **DigiCert KeyLocker** | ~$600+/年 | 无——云 HSM | 信誉累积 | ✅ 可买，价高 |
| **Azure Artifact Signing** | ~$9.99/月（Basic 5k 次/月） | 无——Azure 托管 | 信誉累积 | ❌ 区域锁美/加/欧/英 |
| 传统 token EV（Sectigo/GlobalSign 等） | ~$300–700/年 | USB token 邮寄 | 信誉累积（无即时） | ✅ 可买但要等物流+常驻插着 |

**推荐路径（按场景）**：

- **个人/小团队分发**：先不签。SmartScreen 警告一次性"更多信息→仍要运行"即可过，对开发者用户不是障碍。
- **要正式签名**：SSL.com OV/EV 证书 + eSigner 云签——唯一对国内主体开放的无硬件正规路径。eSigner 用自家 `CodeSignTool` 或 CKA（KSP，装好后证书进 `Cert:\CurrentUser\My`，走 `GTT_SIGN_SHA1` 即可）；如走 CodeSignTool REST 路径，`sign.ps1` 需加一个 eSigner 分支（约半小时改动）。
- **将来主体在美/欧**：再评估 Azure Artifact Signing（$9.99/月，管线 `GTT_SIGN_DLIB` 已兼容）。

## 采购流程（eSigner 路径）

1. **确认主体**：OV/EV 需营业执照（公司或个体户，部分 CA 不接个体户）；证书 CN 显示组织名——想好 UAC/SmartScreen 里出现的名字。
2. SSL.com 下单 OV/EV 代码签名证书，交付方式选 **eSigner（云）**而非邮寄 token。
3. **企业验证**：营业执照 + 对公电话回拨，几天到两周。
4. eSigner 开通后设 4 位 PIN；装 eSigner CKA 则证书出现在 `Cert:\CurrentUser\My` → 取指纹填 `GTT_SIGN_SHA1`，之后照常跑 `package.ps1`。

## 已验证

用本地自签名测试证书走通全链路：三个 exe 全部 `Get-AuthenticodeSignature Status=Valid` 且带 DigiCert TSA 时间戳。换成真证书只是改指纹/换签名工具的事，打包流程零改动。

## 注意

- token PIN：多数客户端支持 Single Logon——一次插 token 输一次，整个会话内多次签名不重复弹。
- **别签到一半分发**：签名会改变文件哈希，分享前确认 `signtool verify /pa` 全过。
- 证书指纹每年换（续期）——`GTT_SIGN_SHA1` 记得更新。
- 签名后信誉按"发布者身份"累积：**换证书 CN/换 CA 都会重置信誉**，选定一家后尽量续同一家。
