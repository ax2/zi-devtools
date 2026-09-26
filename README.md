# Zi DevTools

[官网](https://devtools.zicode.com/) · [下载](https://github.com/ax2/zi-devtools/releases/latest) · [已实现工具与规划](docs/tools.md) · [发版流程](docs/releasing.md) · [MIT License](LICENSE)

MIT 开源的 Windows 原生 Rust 桌面开发工具。

![Zi DevTools 工具首页](docs/images/workspace.png)

主要能力：

- 读取独立的 `~/.zi-devtools/services.yml`。
- 查看本地服务状态、健康检查、端口、PID、日志与配置文件摘要。
- 启动、停止、重启服务，并保存期望运行状态。
- Windows 系统托盘快速查看与逐项控制服务。
- 仅停止本程序确认托管的进程；遇到外部端口占用时拒绝启动，不会自动结束外部进程。
- 可为单个服务配置优雅停止命令；超时后才强制结束已验证的托管进程树。
- “小工具”包含 JSON、Base64、URL、SHA-256、时间戳、UUID、JWT 内容查看、正则、进制转换和二维码生成／PNG 保存。
- “开发工具”下每个复杂工具有独立菜单项。HTTP 请求调试包含最多 12 个请求标签、站点配置与最近 100 条历史；文本差异对比独立成页。
- 新增独立的“网络诊断”开发工具：DNS 解析与单端口 TCP 连通性测试。小工具新增颜色转换与文本统计；各小工具的输入/输出草稿在本次运行中分别保留。
- 左侧菜单统一左对齐；托盘右键可从“小工具”或“开发工具”子菜单直达每个工具并恢复主窗口。
- 托盘右键“本地服务”子菜单汇总所有服务、状态与批量启停；HTTP 历史可搜索，清空时需要二次确认。
- HTTP 请求头、请求体和响应只保存在当前进程内存；站点只保存名称与基础地址，历史只保存方法、去掉查询参数的 URL、状态和耗时，存于 `%USERPROFILE%\.zi-devtools\http-workbench.json`。历史重新打开不会恢复认证信息；URL 路径如含敏感值仍应避免使用历史记录。

## 开发

```powershell
cargo run
cargo test
cargo build --release
```

默认配置路径为 `%USERPROFILE%\.zi-devtools\services.yml`。服务 YAML 的 `env` 字段只用于启动进程，不在 UI 展开；`config_files` 明确列出的文件仍可由用户手工预览。程序不会把环境变量值写入项目档案。

项目完全独立，不依赖任何公司项目、私有包、私有配置或公司运行环境。服务管理模型和实现均保存在本仓库内。

可选启动参数：

```powershell
ZiDevTools.exe --config C:\path\to\services.yml --no-restore
```

`--no-restore` 会跳过启动时的期望服务恢复，适合只读检查配置和状态。

从另一份兼容 YAML 一次性导入服务定义：

```powershell
ZiDevTools.exe --import-config C:\path\to\existing-services.yml
```

导入时会改用 Zi DevTools 自己的状态目录，不复制旧 PID、日志或期望运行状态。目标文件已存在时，需要显式增加 `--force-import`。

服务可选配置 `stop_command` 和 `stop_timeout_ms`（100–60000，默认 5000）。停止命令在该服务的工作目录与环境变量下执行；仅当服务由 Zi DevTools 托管时才运行。停止命令退出但服务仍运行、或达到超时后，将回退到强制停止。未配置停止命令的服务保持原有强制停止方式。停止命令应由用户针对服务自身支持的停机接口编写，不要在命令中嵌入凭据。

## 交付状态

Stage 18 / v0.18.0：Windows 可运行目录 `dist\Stage-18`。

- **Java / Django 专项**：原有 14 项规划均已提供首版入口；独立诊断工作台、分类/收藏/最近/Ctrl K/托盘直达、后台任务、文件导入与报告保存。[支持格式与使用说明](docs/java-django-tools.md)。环境检查与 JFR 需要所选可信 JDK/Python；日志分析不需要安装它们。
- **Java / Django 诊断**：离线 Java 异常链、Django / Python Traceback 整理，输出结构化帧与规则提示；[开发价值与路线](docs/java-django-roadmap.md)。不运行用户项目，不把异常链末端当作已确认根因。
- **体验优化**：目录每页 18 项；筛选自动回到首页；插件离页后仍收取结果并提示；运行时参数锁定、模型必填检查、结构化错误摘要与复制反馈；清单浏览避免反复复制请求体。
- **统一目录**：十类分类、收藏、最近 20 项、按打开次数排序的常用视图；关键词搜索与 Ctrl K 同时覆盖内置和插件工具。
- **插件机制**：JSON 清单预览、安装、启停、刷新、卸载与内容指纹。支持内置配方和非流式 HTTP JSON 适配器，输入/结果/令牌不落盘。[插件使用与开发](docs/plugins.md)。
- **可选连接器**：3 个示例包、6 项工具，覆盖本地文本、Ollama、OpenAI 兼容服务。需要已有服务与模型，不自动下载或启动。
- **本机发现**：只读检查 PATH 入口和常见服务端口；本次调查发现 Ollama、AnythingLLM、Codex CLI、Docker 等，具体运行与集成边界见 [架构与路线](docs/platform-roadmap.md)。
- **扩展规划**：覆盖模型、MCP、RAG、Agent、插件生态和本地数据能力，当前目录共 116 项（50 内置已实现、66 规划）。模型连接器不等于完整 Agent/MCP/RAG。
- 现有 28 项轻量工具、CSV/JSON 工作台、文件校验、HTTP、差异、网络诊断及服务管理。数据操作限制见工具文档。

![Java / Django 诊断工作台](docs/images/java-django.png)

验证：Rust 单元测试、4 项 Windows 生命周期集成测试、fmt、Clippy、真实应用截图、安装包和发布工作流；具体证据见 [发版说明](docs/release-notes.md)。托盘新增插件中心/本机发现入口；系统全局快捷键和进程沙箱尚未实现。

开发档案保留在本地，公开仓库只发布源码、产品文档和可分享的测试说明。

## 工具规划与同步

<!-- tools-summary:start -->
当前包含 **28 个小工具、21 个开发工作台和本地服务管理**，另有 **66 项规划 / 开发中能力**。完整列表见 [工具清单](docs/tools.md)。

优先推进：数据列变换、CSV 合并与关联、校验清单、目录内容对比、模型连接配置、多轮模型工作台、提示词模板库、MCP 连接管理等；完整优先级见清单。规划不代表已实现。
<!-- tools-summary:end -->

清单以 `docs/tools.json` 为唯一来源，运行 `python scripts/sync_tools.py` 更新文档。CI 校验源码入口、清单和生成文档，具体规则见工具清单末尾。
