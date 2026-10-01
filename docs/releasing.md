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

Stage 68 的 Agent 记录元数据对比通过工具并集、缺失 token 保持未知、复制摘要不含任务正文及异常耗时拒绝的定向测试；完整本地回归、格式、Clippy、真实 eframe 亮/暗合成预览、release 构建、WiX 打包和静默安装/卸载均通过。公开 CI `36809803812` 与 Release `36810074425` 成功；[v0.68.0](https://github.com/ax2/zi-devtools/releases/tag/v0.68.0) 四件资产齐全，三个公开安装/程序资产下载后按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。对比只使用已导入的记录元数据，不重放工具，也不证明记录或答案真实。

Stage 67 的 Agent 记录目录检索已通过有界扫描、默认元数据搜索与可选正文搜索、无效文件跳过、文件数/字节上限、UTC 小数秒排序的合成测试；完整本地测试、Rust 格式和 Clippy、亮/暗主题真实 eframe 合成界面、release 构建、WiX 打包与静默安装/卸载均通过。公开 CI `36806960302` 和 Release `36807202745` 均成功；[v0.67.0](https://github.com/ax2/zi-devtools/releases/tag/v0.67.0) 四件资产齐全，下载的程序与安装包按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。目录只在明确选择后后台读取，结果仅在当前窗口内存中；不重放执行。

Stage 66 的 Agent 记录查看器已通过 Stage 65 默认/含内容 JSON 往返、损坏或越权记录拒绝、完整本地测试、Rust 格式和 Clippy、亮/暗主题合成界面预览、release 构建、WiX 打包与静默安装/卸载。公开 CI `36803566301` 和 Release `36803854487` 均成功；[v0.66.0](https://github.com/ax2/zi-devtools/releases/tag/v0.66.0) 四件资产齐全，下载的程序和安装包按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。记录只读，不运行模型/MCP；结构合法的文件也不证明记录真实。

Stage 65 增加 Agent 单次运行记录的默认不含内容 JSON 预览与显式保存；合成敏感值测试验证服务路径、参数、MCP schema/结果不会进入导出记录。完整本地测试、格式、Clippy、release 构建、WiX 打包及静默安装/卸载已通过。公开 CI `36799718105` 和 Release `36800037757` 均成功；[v0.65.0](https://github.com/ax2/zi-devtools/releases/tag/v0.65.0) 四件资产齐全，三个二进制/安装资产下载后按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，桌面/手机视口已检查。记录不可导入或重放，失败/取消时不伪造模型 token 统计。

Stage 64 的本机 Ollama 只读 Agent 工作台已通过一次性模型/MCP 夹具、真实 `qwen2.5:7b` 合成任务、本地完整测试、格式和 Clippy、release 构建、WiX 打包、静默安装/卸载及桌面/手机合成预览。公开 CI `36795540958`、Release `36795877549` 均成功；[v0.64.0](https://github.com/ax2/zi-devtools/releases/tag/v0.64.0) 四件资产齐全，下载的程序与安装包通过 SHA256SUMS，线上官网的页面、清单和图片哈希及桌面/手机视口已核对。计划批准前不调用工具；执行只接受白名单中已直接授权的低影响只读 MCP 定义，并逐次复核。写操作、费用计费、其他模型协议和长期恢复仍待设计。

Stage 63 的 MCP stdio 调试台持久工具规则、调用前复核与撤销已公开发布：一次性夹具、本地和公开 CI/Release、Windows 安装/卸载、四件下载资产哈希及官网桌面/手机均验证通过，本机阶段构建缓存安全清理约 13.14 GiB。当时尚未接入 Agent 自动调用。Stage 62 的关键词/混合同题对照与可选混合证据模型评测已公开验证。更大语料和事实忠实度评测仍需实现；录屏非主屏、混合 DPI、其他声卡、真实热拔插与更长时间仍缺实际验收；其他第三方 MCP 服务、远程 HTTP 与持久会话也未验收。

升级前请从系统托盘选择退出 Zi DevTools。卸载仅移除程序与快捷方式，用户目录中的配置和服务状态保留。
