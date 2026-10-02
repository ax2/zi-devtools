# 构建与发版

项目采用 MIT 许可证。产品主页：https://devtools.zicode.com/ 。

## 自动工作流

参考 ZiFile 的标签触发、版本一致性、质量检查、安装包、便携 EXE、SHA-256 和 GitHub Release 流程。Zi DevTools 当前仅发布 Windows x64，安装器使用固定版本 WiX Toolset 5.0.2 生成原生 MSI；没有复用 ZiFile 的 MSIX 身份、签名证书或 ARM64 成熟度声明。

1. 修改 Cargo.toml 版本、docs/tools.json 的版本和日期、docs/release-notes.md；运行 `python scripts/sync_tools.py`。
2. 提交源代码与生成文档，等待 CI 通过。
3. 在 Actions 运行 Release（workflow_dispatch）：读取 Cargo 版本，通过检查后自动创建 `v<version>` 标签和 Release。也可推送同名标签触发。
4. Windows runner 执行文档同步检查、fmt、Clippy、测试、release 构建、安装器生成、静默安装/卸载检查，然后发布 MSI、便携 EXE 与 SHA256SUMS.txt。
5. 已有 Release 不允许覆写；修复需要新版本。发版后下载四件资产，执行 `python scripts/verify_release.py --artifact-dir release/stage-current-public --version <版本号>`，检查三件程序/安装文件与 SHA256SUMS 完整且一致，然后更新官网清单快照。校验和检查检测文件损坏，不等于发布者签名验证。

本地构建：`cargo build --release --locked`。使用 .NET 8 执行 `dotnet tool install --global wix --version 5.0.2` 后，再执行 `pwsh -File scripts/package.ps1`。默认输出到 release/：MSI、桌面便携 EXE、知识库 MCP 便携 EXE、SHA256SUMS，无 ZIP。如果旧便携 EXE 正在运行，可用 `pwsh -File scripts/package.ps1 -OutputDir release/stage-current` 在项目内独立目录打包，并用 `python scripts/installer_smoke.py --artifact-dir release/stage-current` 验证；CI 继续使用默认目录。MSI 同时安装两个程序，便携使用时将两 EXE 放在同一目录。安装为当前用户，不需要管理员权限。最低 Windows 10，x64。当前未配置代码签名，不能声明 Microsoft Store 认证。

## 工具状态同步

唯一来源 docs/tools.json；生成文档 docs/tools.md。CI 用源码入口 ID 检查已实现列表。官网同源快照来自这份 JSON，页面优先读取公开 GitHub 最新清单，失败时显示快照日期。

## 验证基线

Stage 78 / v0.78.0 汇总导航与命名数据实例、备忘录、万年历/日程/提醒，以及 HTTP Bearer 和实验性公共客户端 OAuth。功能增量已通过本地分项、原生 UI 和公开 CI；版本化安装包、最终 CI/互操作与公开资产需按上面的正式流程独立核对。OAuth 的合成 SDK/TLS 证据不等于真实授权服务、系统浏览器或隐藏/休眠恢复验收；完全退出应用后不提供日程提醒。最新功能范围见 [版本说明](release-notes.md)、[日常工具](planner.md)与 [MCP 调试台](mcp-inspector.md)。

Stage 77 的 2025 Streamable HTTP 调试台通过 8 项有界会话与异常集成测试、官方 SDK 1.31.0 JSON/SSE 合成互操作、完整本地检查、真实 eframe 明暗预览、release 构建及 MSI 静默安装/卸载。公开 CI 36954104994、最终说明 CI 36954466886 与 Release 36954783249 均成功；[v0.77.0](https://github.com/ax2/zi-devtools/releases/tag/v0.77.0) 四件公开文件下载后同时匹配 SHA256SUMS 与 GitHub asset digest/size。官网更新已暂存并通过主站构建与文件校验，浏览器读取超时导致最新网页渲染与切换待完成；不宣称本轮手机网页验收通过。认证、2026 协议、独立 GET 流与业务服务器兼容性仍未实现。

Stage 75 的 SQLite 浏览器通过异常表名、分页、源文件未修改、错误文件、慢查询取消、1 MiB BLOB/长文本裁剪和 CSV 不覆盖/公式前缀测试；完整本地测试、格式、严格 Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包及 MSI 静默安装/卸载通过。公开 CI `36872868710`、修复后 CI `36874551475` 成功；首次 Release `36873499881` 因既有服务网络夹具并行抢占端口失败，提交 `ea30044` 串行化夹具后，Release `36875043358` 成功。[v0.75.0](https://github.com/ax2/zi-devtools/releases/tag/v0.75.0) 四件资产齐全，三个公开程序/安装文件下载后按 SHA256SUMS 核验通过。官网首页、清单与新图线上哈希一致，Edge 桌面 1440×1000、手机 390×844 显示 v0.75.0、87 项已实现与返回主站链接，无横向溢出。本轮深度维护清理 10,869,701,712 字节可重建缓存，Stage 75 本地四件完整文件保留、无 ZIP。只读 SQLite 连接可能管理 WAL 辅助文件，页间不保证一致性快照；CSV 只导出当前页预览。

Stage 74 的双目录比较通过内容、大小、类型、空目录、单侧路径、部分扫描不误报缺失、摘要预算、文件变化、取消及 CSV 无覆盖测试；完整本地测试、格式、严格 Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包与 MSI 静默安装/卸载通过。公开 CI `36864815644`、Release `36865282497` 成功；[v0.74.0](https://github.com/ax2/zi-devtools/releases/tag/v0.74.0) 四件资产齐全，三个公开程序/安装文件按 SHA256SUMS 下载核验通过。官网首页、清单与新图线上哈希一致，Edge 桌面 1440×1000 和手机 390×844 显示 v0.74.0、86 项已实现与返回主站链接，无横向溢出。本轮安全清理 10,732,232,599 字节可重建构建缓存，Stage 74 本地四件完整文件保留、无 ZIP。部分扫描不将单侧可见项断言为缺失；文件后续变化无法由本次摘要保证。

Stage 73 的 ASCII 码表、符号/表情/颜文字精选库和 ASCII Art 通过控制符/精确码值、大小写字符查询、未知字形替代、图片黑白映射及尺寸上限测试；完整本地测试、格式、严格 Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包和 MSI 静默安装/卸载通过。可选 Segoe UI Emoji 回退在真实预览中修复了“思考”表情缺字方框。公开 CI `36835947556`、Release `36836397185` 成功；[v0.73.0](https://github.com/ax2/zi-devtools/releases/tag/v0.73.0) 四件资产齐全，三个公开程序/安装文件按 SHA256SUMS 验证。官网首页、清单与新图线上哈希一致，Edge 桌面 1440×1000 和手机 390×844 显示 v0.73.0、85 项已实现与返回主站链接，无横向溢出。本轮安全清理 16,017,233,470 字节可重建缓存；Stage 72/73 各四件完整本地文件均保留、无 ZIP。字符库是精选，Emoji 显示依赖字体；字符画是视觉近似。

Stage 72 的重复文件检查通过同大小不同内容、零字节文件、文件变化、目录与内容读取预算、取消及 SHA-256 分组测试；完整本地测试、格式、严格 Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包与 MSI 静默安装/卸载通过。公开 CI `36832751707` 第二次运行成功（首次为既有服务生命周期测试在 runner 上偶发失败），Release `36833584263` 成功；[v0.72.0](https://github.com/ax2/zi-devtools/releases/tag/v0.72.0) 四件资产齐全，三个公开程序/安装文件按 SHA256SUMS 校验通过。官网首页、清单与新图线上哈希一致，Edge 桌面 1440×1000 和手机 390×844 显示 v0.72.0、82 项已实现与返回主站链接，无横向溢出。构建缓存已与 Stage 73 交付后一并安全清理；重复逻辑大小不是实际可回收量，工具不自动删除。

Stage 71 的目录空间分析通过合成目录的嵌套聚合、逻辑字节大小、分组与最大文件排序、深度/条目部分结果、扫描中取消及环境允许时的链接跳过测试；完整本地测试、格式、严格 Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包与 MSI 静默安装/卸载通过。公开 CI `36829408571`、Release `36829749748` 成功；[v0.71.0](https://github.com/ax2/zi-devtools/releases/tag/v0.71.0) 四件资产齐全，三个公开程序/安装文件下载后按 SHA256SUMS 验证通过。官网首页、清单、新图与本地哈希一致，Edge 桌面 1440×1000、手机 390×844 显示 v0.71.0、新图和返回主站链接，无横向溢出。本轮安全清理 10,818,023,420 字节构建缓存，完整阶段目录保留。扫描只读取文件元数据；逻辑长度不等于实际磁盘分配空间，部分结果不代表完整目录大小。

Stage 70 的知识 MCP 结构化命中按 3 KiB 模型消息装箱，步骤只列出选中的编号来源；重复包装、长片段裁剪、多次调用编号、伪造引用拒绝及 v1/v2/v3 记录兼容有定向测试。完整本地测试、格式、严格 Clippy、真实 eframe 预览、真实本机 Ollama 一次性只读调用、release 构建、WiX 打包和静默安装/卸载通过。公开 CI `36822342054` 与 Release `36822647589` 成功；[v0.70.0](https://github.com/ax2/zi-devtools/releases/tag/v0.70.0) 四件资产齐全，三个公开程序/安装资产下载后按 SHA256SUMS 校验通过。线上官网首页、工具清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口显示 v0.70.0、配图和返回主站链接且无横向溢出。本轮安全清理 12,294,636,146 字节构建缓存，完整阶段目录保留。失败或取消时已选中的最近一步消息可能尚未发送；编号有效不证明答案事实正确。

Stage 69 的 Agent 来源追踪通过有界来源提取、畸形路径/哈希拒绝、默认导出隐私、v1/v2 往返及一次性 MCP 端到端夹具测试；完整本地测试、格式、Clippy、真实 eframe 亮/暗预览、release 构建、WiX 打包和静默安装/卸载通过。公开 CI `36818538631` 与 Release `36818783296` 成功；[v0.69.0](https://github.com/ax2/zi-devtools/releases/tag/v0.69.0) 四件资产齐全，三个公开程序/安装资产下载后按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口显示 v0.69.0、配图和返回主站链接且无横向溢出。来源是 MCP 工具报告的命中，并不证明第三方服务可信或模型答案确实引用。

Stage 68 的 Agent 记录元数据对比通过工具并集、缺失 token 保持未知、复制摘要不含任务正文及异常耗时拒绝的定向测试；完整本地回归、格式、Clippy、真实 eframe 亮/暗合成预览、release 构建、WiX 打包和静默安装/卸载均通过。公开 CI `36809803812` 与 Release `36810074425` 成功；[v0.68.0](https://github.com/ax2/zi-devtools/releases/tag/v0.68.0) 四件资产齐全，三个公开安装/程序资产下载后按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。对比只使用已导入的记录元数据，不重放工具，也不证明记录或答案真实。

Stage 67 的 Agent 记录目录检索已通过有界扫描、默认元数据搜索与可选正文搜索、无效文件跳过、文件数/字节上限、UTC 小数秒排序的合成测试；完整本地测试、Rust 格式和 Clippy、亮/暗主题真实 eframe 合成界面、release 构建、WiX 打包与静默安装/卸载均通过。公开 CI `36806960302` 和 Release `36807202745` 均成功；[v0.67.0](https://github.com/ax2/zi-devtools/releases/tag/v0.67.0) 四件资产齐全，下载的程序与安装包按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。目录只在明确选择后后台读取，结果仅在当前窗口内存中；不重放执行。

Stage 66 的 Agent 记录查看器已通过 Stage 65 默认/含内容 JSON 往返、损坏或越权记录拒绝、完整本地测试、Rust 格式和 Clippy、亮/暗主题合成界面预览、release 构建、WiX 打包与静默安装/卸载。公开 CI `36803566301` 和 Release `36803854487` 均成功；[v0.66.0](https://github.com/ax2/zi-devtools/releases/tag/v0.66.0) 四件资产齐全，下载的程序和安装包按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，Edge 桌面/手机视口均显示 80 项已实现工具、图片和返回主站链接，且无横向溢出。记录只读，不运行模型/MCP；结构合法的文件也不证明记录真实。

Stage 65 增加 Agent 单次运行记录的默认不含内容 JSON 预览与显式保存；合成敏感值测试验证服务路径、参数、MCP schema/结果不会进入导出记录。完整本地测试、格式、Clippy、release 构建、WiX 打包及静默安装/卸载已通过。公开 CI `36799718105` 和 Release `36800037757` 均成功；[v0.65.0](https://github.com/ax2/zi-devtools/releases/tag/v0.65.0) 四件资产齐全，三个二进制/安装资产下载后按 SHA256SUMS 校验通过。线上官网首页、清单和配图与本地 SHA-256 一致，桌面/手机视口已检查。记录不可导入或重放，失败/取消时不伪造模型 token 统计。

Stage 64 的本机 Ollama 只读 Agent 工作台已通过一次性模型/MCP 夹具、真实 `qwen2.5:7b` 合成任务、本地完整测试、格式和 Clippy、release 构建、WiX 打包、静默安装/卸载及桌面/手机合成预览。公开 CI `36795540958`、Release `36795877549` 均成功；[v0.64.0](https://github.com/ax2/zi-devtools/releases/tag/v0.64.0) 四件资产齐全，下载的程序与安装包通过 SHA256SUMS，线上官网的页面、清单和图片哈希及桌面/手机视口已核对。计划批准前不调用工具；执行只接受白名单中已直接授权的低影响只读 MCP 定义，并逐次复核。写操作、费用计费、其他模型协议和长期恢复仍待设计。

Stage 63 的 MCP stdio 调试台持久工具规则、调用前复核与撤销已公开发布：一次性夹具、本地和公开 CI/Release、Windows 安装/卸载、四件下载资产哈希及官网桌面/手机均验证通过，本机阶段构建缓存安全清理约 13.14 GiB。当时尚未接入 Agent 自动调用。Stage 62 的关键词/混合同题对照与可选混合证据模型评测已公开验证。更大语料和事实忠实度评测仍需实现；录屏非主屏、混合 DPI、其他声卡、真实热拔插与更长时间仍缺实际验收；其他第三方 MCP 服务、远程 HTTP 与持久会话也未验收。

升级前请从系统托盘选择退出 Zi DevTools。卸载仅移除程序与快捷方式，用户目录中的配置和服务状态保留。

## 软件官网配图预算

用户要求落地网站配图统一 WebP，以控制计费下行流量。发布截图时，在个人网站仓库运行 scripts/optimize-software-images.py 及 --check；生成带内容哈希的文件名和移动/桌面 srcset。桌面截图最大 1600px / 192 KiB，手机版本最大 800px / 80 KiB，徽标最大 128px / 16 KiB；编码质量不低于 80，超预算必须人工调整和检查可读性。

只上传页面实际引用的 WebP 与必要小型 SVG；源 PNG 留本地，不整目录上传 assets。检查代表性 WebP 文字、桌面/手机页面布局与主站返回链接，主站构建后再切换。构建产物包含无关工作时不得整站部署。静态格式/字节预算校验不能代替网页渲染验收。
