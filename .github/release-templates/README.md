# Release templates

`alpha-release.yml` / `stable-release.yml` 共用的发布说明模板与生成工具。

## 用法

| 命令 | 说明 |
| --- | --- |
| `python render_release_template.py --template alpha.md --output out.md --version vX.Y.Z-alpha.N --commit <sha> --release-date YYYY-MM-DD --sha256 <hex> --asset-filename <file> --changelog-file changelog.md` | 渲染 Alpha 发布说明 |
| `python render_release_template.py --template stable.md ... --release-summary "…"` | 渲染 Stable 发布说明（`{{RELEASE_SUMMARY}}` 必填） |
| `python validate_build.py --cargo-toml Cargo.toml --channel stable --release-tag vX.Y.Z` | 校验 tag 与 workspace 版本一致性 |
| `python generate_release_notes.py --channel stable --repo-root . --repository-url <url> --release-tag vX.Y.Z --commit <sha> --changelog-output out.md` | 生成 changelog（有 LLM 槽位时尝试 LLM，否则规则回退） |

版本规则：Stable tag 必须等于 `Cargo.toml` 的 `[workspace.package] version`；Alpha tag 为 `vX.Y.Z-alpha.N` 且 X.Y.Z 等于 workspace 版本。

## 模板 token

`{{VERSION}}` `{{COMMIT_SHA}}` `{{RELEASE_DATE}}` `{{ASSET_FILENAME}}` `{{ASSET_SHA256}}` `{{CHANGELOG}}` `{{RELEASE_SUMMARY}}`（仅 stable.md）——渲染后不允许残留任何 `{{…}}`。

## 可选 LLM 槽位

最多两个提供方，按序尝试；`API_KEY` 为空则跳过，全部不可用时回退规则生成：

| 变量 | 位置 | 说明 |
| --- | --- | --- |
| `RELEASE_NOTES_LLM_<N>_API_KEY` | Secret | 必填才启用该槽位 |
| `RELEASE_NOTES_LLM_<N>_PROTOCOL` | Variable | `anthropic`（默认）或 `openai` |
| `RELEASE_NOTES_LLM_<N>_BASE_URL` | Secret 或 Variable | 接口地址 |
| `RELEASE_NOTES_LLM_<N>_MODEL` | Variable | 模型名 |
| `RELEASE_NOTES_LLM_<N>_RESPONSE_FORMAT` | Variable | `auto`（默认，json_schema → json_object → 纯提示词逐级降级）、`json_object`（仅 openai）或 `none`（纯提示词）。DeepSeek 两种格式都会拒，设为 `none` |

`release_notes_config.json` 控制产品语境、禁用术语、路径过滤与输出上限。
