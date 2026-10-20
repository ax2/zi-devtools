# Inspect 插件 `tool.open.v1` 预览候选

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
插件包版本：`com.zicode.devtools.inspect 0.1.2`\
Studio Host profile：`tool.open.v1@=1.0.0-preview.1`

## 目标与边界

为 Inspect 插件的主 View 实现 Host 发来的 `openTool({ capabilityId })` 导航回调，让 Studio 能打开插件并直达指定工具。候选只负责 UI 定位，不调用任何插件能力，不授予权限、不处理文件或服务，也不代表 Studio Host 已接受此协议。冻结候选 `inspect-0.1.1-candidate-02` 未修改；新包复用相同 WASM。

Studio 合约快照位于 `contracts/studio-tool-open/1.0.0-preview.1/README.md`。候选由 `scripts/package_inspect_tool_open_plugin.py` 从冻结包独立复制生成，验证器为 `scripts/verify_inspect_tool_open_plugin.py`；验证回执在包外，避免把自校验回执算入候选自身。

## 能力与界面映射

| Capability ID | UI action ID |
| --- | --- |
| `devtools.inspect.color.convert` | `color.convert` |
| `devtools.inspect.yaml.to_json` | `yaml.to_json` |
| `devtools.inspect.yaml.from_json` | `yaml.from_json` |
| `devtools.inspect.cidr.inspect` | `cidr.inspect` |
| `devtools.inspect.jwt.inspect` | `jwt.inspect` |
| `devtools.inspect.unicode.inspect` | `unicode.inspect` |
| `devtools.inspect.unicode.nfc` | `unicode.nfc` |
| `devtools.inspect.unicode.nfkc` | `unicode.nfkc` |

映射按照 capability ID 精确匹配。成功时选中操作、刷新选项并聚焦输入；未知 ID 返回 `not_found`，缺少界面节点返回 `unsupported`。三种情况均不执行能力。用户需在界面检查输入并主动启动处理。

## 修改与验证

- 新候选目录：`release/plugin-candidates/inspect-0.1.2-candidate-01/`；外部回执：`release/plugin-candidates/inspect-0.1.2-candidate-01-verification.json`。
- 清单同时精确声明 `tool.directory.v1` 与 `tool.open.v1` 的 `=1.0.0-preview.1` profile，提供唯一 `location: main` 的 View。
- 实际浏览器验证正向定位与未知 ID，并保存亮、暗主题截图。慢回调 fixture 延迟 4.5 秒，超过 4 秒预算后返回 `unsupported`，并验证迟到 ACK 不覆盖超时结果；慢回调仅在验收 fixture 中。
- 冻结基线核对通过，原有 WASM SHA-256 保持 `1db3fc37df01cd3a8371320d4f897a7fdf750c87e6202b736ed6870f046b8f29`。
- 候选收据：616 个文件、2,341,254 字节；按排序相对路径、文件长度与 SHA-256 计算的目录索引为 `1d3af04bb9e35fa9594a673ff71058e56ffbb94e516f4086dba275472d1678ee`。
- `python scripts/verify_inspect_tool_open_plugin.py`：通过，浏览器截图 2 张。
- `python scripts/sync_studio_plugin_map.py` 与 `--check`：通过，Studio 工具迁移目录 154 项，已接受能力仍为 0。
- `python scripts/sync_tools.py --check`：通过；工具目录 92 项已实现、62 项规划/进行中。
- `cargo fmt --all -- --check` 与 `git diff --check`：通过。

## 当前状态与遗留

这是未签名的提供方候选目录，不是可安装 `.zicode-plugin` 容器，也不是 Studio 接受证明。Studio 侧仍需在真实 Host 中确认请求通道、请求 ID、4/5 秒预算、View 销毁、迟到回执、撤权、卸载以及跨工作区/场景切换；现有候选不能证明这些接收端生命周期行为。未经 Host/Pi 验收，迁移目录的 `acceptedCapabilities` 保持 0。
