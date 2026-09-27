# 代码签名指南

目标：让 `GlobalTokenTracker-Setup-*.exe` 在收件人机器上**直接过 SmartScreen**，不再弹"Windows 已保护你的电脑"。

## 管线已就绪

`installer/package.ps1` 内置两处签名（见 `installer/sign.ps1`）：

1. `payload.zip` 打包**之前**签 `globaltokentracker-ui.exe` / `-cli.exe`（内嵌二进制本身带签名）
2. dist 落盘后签 `GlobalTokenTracker-Setup-<ver>-win-x64.exe`（SmartScreen 检查的外壳）

签名配置全部走环境变量，仓库里不放任何证书/密钥：

| 环境变量 | 用途 |
|---|---|
| `GTT_SIGN_SHA1` | **EV 证书路径**：证书指纹。token 客户端（SafeNet/eToken/KeyLocker）装好后证书出现在 `Cert:\CurrentUser\My`，私钥不出 HSM |
| `GTT_SIGN_PFX` + `GTT_SIGN_PFX_PASS` | PFX 文件路径+密码（OV 证书/本地测试证书用） |
| `GTT_SIGN_DLIB` + `GTT_SIGN_DLIB_METADATA` | 云签名（如 Azure Trusted Signing 的 `Azure.CodeSigning.Dlib.dll` + `metadata.json`） |
| `GTT_SIGN_TSA` | 时间戳服务器，默认 `http://timestamp.digicert.com` |

用法：

```powershell
$env:GTT_SIGN_SHA1 = '<证书指纹>'
powershell -ExecutionPolicy Bypass -File installer\package.ps1
```

每个文件签后自动 `signtool verify /pa` 校验；全部带 RFC3161 时间戳（证书过期后签名依然有效）。

## 你要做的采购部分

EV 代码签名证书**只签给组织**（公司/注册主体），个人拿不到——这是第一道门槛。

1. **确认主体**：需要营业执照（公司或个体工商户均可，部分 CA 不接个体户）。证书 CN 会显示为**组织名**——想好希望 UAC/SmartScreen 里出现什么名字。
2. **选 CA 下单**（2025 年大致价位，以官网为准）：
   | CA | 约价/年 | 交付方式 |
   |---|---|---|
   | SSL.com | ~$250–350 | 云 HSM（eSigner）或邮寄 token |
   | Sectigo | ~$300–400 | SafeNet eToken 邮寄 |
   | DigiCert | ~$600–700 | KeyLocker 云 / token |
   | GlobalSign / Entrust | ~$400–700 | token / HSM |
3. **企业验证**：营业执照 + 对公电话回拨 + 可能要求律师信/邓白氏编码——周期几天到两周。
4. **收货/开通**：token 到货装客户端驱动；云 HSM 则拿凭证。之后证书出现在 `Cert:\CurrentUser\My` → 取指纹填 `GTT_SIGN_SHA1`。
5. 每次发布照常跑 `package.ps1`——签名自动发生。

## 备选：Azure Trusted Signing（值得先评估）

微软自家的云签名服务（原 Azure Code Signing），**~$10/月**，云端 HSM 无需硬件，对 SmartScreen 信誉的支持等同于托管证书（发布即获得信誉基线）。比传统 EV 便宜一个数量级、无 token 物流；代价是要一个 Azure 订阅 + 身份验证（以组织验证为主）。如果目的只是"过 SmartScreen"，它大概率是性价比最优解——管线已支持（`GTT_SIGN_DLIB` + `GTT_SIGN_DLIB_METADATA`）。

## 已验证

用本地自签名测试证书走通全链路：三个 exe 全部 `Get-AuthenticodeSignature Status=Valid` 且带 DigiCert TSA 时间戳。换成真 EV 证书只是改指纹的事，代码零改动。

## 注意

- token PIN：多数客户端支持 Single Logon——一次插 token 输一次，整个会话内多次签名不重复弹。
- **别签到一半分发**：签名会改变文件哈希，分享前确认 `signtool verify /pa` 全过。
- 证书指纹每年换（续期）——`GTT_SIGN_SHA1` 记得更新。
