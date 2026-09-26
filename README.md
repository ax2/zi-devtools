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

Stage 15 / v0.15.0：Windows 可运行目录 `dist\Stage-15`。

- **界面**：新工具首页、名称/用途搜索、收藏、Ctrl K 快速打开；↑↓选择、Enter 打开、Esc 关闭。主题与收藏自动保存到配置所在目录的 `ui-preferences.json`。
- **交互**：单列工具编辑、示例、输入字节数、Ctrl Enter 执行转换、只读结果、复制反馈、清空确认。
- **本轮新增 6 项**：JSON 路径查询、JSON 结构对比、数据质量检查、Cron 预览、随机数据生成、Unicode 检查。[用法与限制](docs/advanced-tools.md)。
- **数据工作台**：CSV/TSV 与 JSON 对象数组导入、筛选、排序、表格预览、导出及保存。UTF-8 输入最多 2 MiB、10000 行、128 列；预览 200 行，导出包含全部筛选结果。CSV 保留字符串（文字排序），JSON 保留值类型（数字数值排序），缺失字段补 null。输入有内容时先选中并清空，再载入另一个文件；编辑后点“解析数据”刷新。保存拒绝覆盖已有文件。
- **批量文件校验**：每行一个文件路径或拖放文件，最多 64 个；后台流式计算 SHA-256/SHA-512、进度、取消、预期摘要比对和复制。
- 新增本地处理工具不上传数据、不保存输入草稿；原 HTTP 与网络诊断仍按用户操作访问网络。

验证：43 项单元测试、4 项 Windows 生命周期集成测试、格式检查与 Clippy 通过。21 张真实应用渲染截图覆盖亮暗主题和窄窗口；Ctrl K、方向键与 Enter 导航已验证。发布构建与 EXE 冒烟记录见 `docs/releasing.md`。本轮未人工验证托盘鼠标操作和系统拖放。

开发档案保留在本地，公开仓库只发布源码、产品文档和可分享的测试说明。

## 工具规划与同步

<!-- tools-summary:start -->
当前包含 **26 个小工具、5 个开发工作台和本地服务管理**，另有 **24 项规划 / 开发中能力**。完整列表见 [工具清单](docs/tools.md)。

优先推进：数据列变换、CSV 合并与关联、校验清单、目录内容对比。规划不代表已实现。
<!-- tools-summary:end -->

清单以 `docs/tools.json` 为唯一来源，运行 `python scripts/sync_tools.py` 更新文档。CI 校验源码入口、清单和生成文档，具体规则见工具清单末尾。
