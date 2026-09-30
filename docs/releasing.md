# 构建与发版

项目采用 MIT 许可证。产品主页：https://devtools.zicode.com/ 。

## 自动工作流

参考 ZiFile 的标签触发、版本一致性、质量检查、安装包、便携 EXE、SHA-256 和 GitHub Release 流程。Zi DevTools 当前仅发布 Windows x64，安装器使用固定版本 WiX Toolset 5.0.2 生成原生 MSI；没有复用 ZiFile 的 MSIX 身份、签名证书或 ARM64 成熟度声明。

1. 修改 Cargo.toml 版本、docs/tools.json 的版本和日期、docs/release-notes.md；运行 `python scripts/sync_tools.py`。
2. 提交源代码与生成文档，等待 CI 通过。
3. 在 Actions 运行 Release（workflow_dispatch）：读取 Cargo 版本，通过检查后自动创建 `v<version>` 标签和 Release。也可推送同名标签触发。
4. Windows runner 执行文档同步检查、fmt、Clippy、测试、release 构建、安装器生成、静默安装/卸载检查，然后发布 MSI、便携 EXE 与 SHA256SUMS.txt。
5. 已有 Release 不允许覆写；修复需要新版本。发版后检查下载和校验和，并更新官网清单快照。

本地构建：`cargo build --release --locked`。使用 .NET 8 执行 `dotnet tool install --global wix --version 5.0.2` 后，再执行 `pwsh -File scripts/package.ps1`。默认输出到 release/：MSI、桌面便携 EXE、知识库 MCP 便携 EXE、SHA256SUMS，无 ZIP。如果旧便携 EXE 正在运行，可用 `pwsh -File scripts/package.ps1 -OutputDir release/stage-current` 在项目内独立目录打包，并用 `python scripts/installer_smoke.py --artifact-dir release/stage-current` 验证；CI 继续使用默认目录。MSI 同时安装两个程序，便携使用时将两 EXE 放在同一目录。安装为当前用户，不需要管理员权限。最低 Windows 10，x64。当前未配置代码签名，不能声明 Microsoft Store 认证。

## 工具状态同步

唯一来源 docs/tools.json；生成文档 docs/tools.md。CI 用源码入口 ID 检查已实现列表。官网同源快照来自这份 JSON，页面优先读取公开 GitHub 最新清单，失败时显示快照日期。

## 验证基线

Stage 62 的关键词/混合同题对照与可选混合证据模型评测已公开发布：一次性夹具、真实本机 `bge-m3:latest` + `qwen2.5:7b` 合成文档、公开 CI/Release、Windows 安装/卸载、四件下载资产哈希及官网桌面/手机均验证通过；本机阶段构建缓存安全清理约 10.57 GiB。更大语料和事实忠实度评测仍需实现；录屏非主屏、混合 DPI、其他声卡、真实热拔插与更长时间仍缺实际验收；其他第三方 MCP 服务、远程 HTTP 与持久会话也未验收。

升级前请从系统托盘选择退出 Zi DevTools。卸载仅移除程序与快捷方式，用户目录中的配置和服务状态保留。
