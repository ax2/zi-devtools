# 构建与发版

项目采用 MIT 许可证。产品主页：https://devtools.zicode.com/ 。

## 自动工作流

参考 ZiFile 的标签触发、版本一致性、质量检查、安装包、便携 EXE、SHA-256 和 GitHub Release 流程。Zi DevTools 当前仅发布 Windows x64，安装器使用固定版本 WiX Toolset 5.0.2 生成原生 MSI；没有复用 ZiFile 的 MSIX 身份、签名证书或 ARM64 成熟度声明。

1. 修改 Cargo.toml 版本、docs/tools.json 的版本和日期、docs/release-notes.md；运行 `python scripts/sync_tools.py`。
2. 提交源代码与生成文档，等待 CI 通过。
3. 在 Actions 运行 Release（workflow_dispatch）：读取 Cargo 版本，通过检查后自动创建 `v<version>` 标签和 Release。也可推送同名标签触发。
4. Windows runner 执行文档同步检查、fmt、Clippy、测试、release 构建、安装器生成、静默安装/卸载检查，然后发布 MSI、便携 EXE 与 SHA256SUMS.txt。
5. 已有 Release 不允许覆写；修复需要新版本。发版后检查下载和校验和，并更新官网清单快照。

本地构建：`cargo build --release --locked`。使用 .NET 8 执行 `dotnet tool install --global wix --version 5.0.2` 后，再执行 `pwsh -File scripts/package.ps1`。输出到 release/，无 ZIP。安装为当前用户，不需要管理员权限。最低 Windows 10，x64。当前未配置代码签名，不能声明 Microsoft Store 认证。

## 工具状态同步

唯一来源 docs/tools.json；生成文档 docs/tools.md。CI 用源码入口 ID 检查已实现列表。官网同源快照来自这份 JSON，页面优先读取公开 GitHub 最新清单，失败时显示快照日期。

## 验证基线

Stage 17 已通过 56 项单元测试、4 项 Windows 生命周期集成测试和本地 EXE 冒烟。工作流另行验证干净 Windows runner 的完整构建及安装卸载；具体结果以 Actions 为准。

升级前请从系统托盘选择退出 Zi DevTools。卸载仅移除程序与快捷方式，用户目录中的配置和服务状态保留。
