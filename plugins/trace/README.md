# Java / Django 堆栈分析插件（开发中）

`com.zicode.devtools.trace/0.1.0` 使用同仓 `zi-trace-core`，独立桌面程序直接调用同一个核心。根程序仍为 `0.82.0-dev.105`，两项源工具为 `1.0.3`。本目录不是已冻结交付包，不代表 Studio 接受；完整目录的这两项仍为 `not_delivered`。

| 操作 | 完整既有范围 |
| --- | --- |
| `java.trace` | 标准 printStackTrace；独立异常、cause/suppressed 层级、主链终点、模块/Native/Unknown Source 帧及省略帧计数。省略帧不还原，终点不等于确认根因。 |
| `django.trace` | 文本 Python Traceback、直接因果/处理期间/独立异常链、路径/行号/函数、缺头或截断标记及既有 Django 规则提示。不运行项目，不支持 HTML 调试页或 ExceptionGroup 树。 |

冻结 rc.1 外层保持不变；只特化插件 ID。输入 `text` 最多 8192 UTF-8 字节，完整请求与序列化结果最多 48 KiB；2 MiB 模块、10M fuel、5秒、64MiB 内存和2MiB栈。所有路径只是粘贴文本，不访问文件、环境、进程或网络。View 通过公共 SDK 调用能力；可选 Pi 声明需要 Host 单独授权。

原正则实现完整保留为测试专用参照 `crates/zi-trace-core/src/legacy.rs`。运行时改为固定语法扫描器，Unicode `\w`/`\d` 范围由锁定的 regex-syntax 在构建时生成；测试比较捕获组、CRLF/Unicode/空白、完整报告与错误。正则不进入该插件运行依赖，不缩减既有功能。

初次直接提取模块 1,361,817 字节，47条预算检查有13条失败，失败证据保留。扫描器检查点模块 241,986 字节（SHA256 `5bb214b0188bf5499cb6fe3814cba6ce9cc8890d309a03e2d47ec57d47a80797`），56条真实 native/Node-WASI 字节对照通过；预算55/56通过，512节点的保证超大报告仍耗尽 fuel。源码已添加只针对必定超过结果预算的保守下界检查，等待最终模块重建、完整重放；不能将旧模块的通过项套用该改动。

同一检查点的公开 SDK 注入与实际 WASI View 已通过2操作、1180浅色/390深色、搜索、草稿恢复、业务/宿主错误保留旧结果、UTF-8预算、过期响应、Ctrl+Enter/Ctrl+K、选中复制及无Host禁用；两张渲染截图已目视检查。这不是原生独立版或实际 Studio UI/Pi 验收。最终模块变化后必须重新绑定页面证据。

复现：`python scripts/sync_trace_plugin.py --check`；通过 `scripts/dev.ps1` 构建 `zi-trace-wasi` 原生与现有 plugin/WASI profile，再运行 `verify_transforms_wasi.py --plugin trace`、`verify_transforms_schema.py --plugin trace`，独立预算/冷启动使用既有验证脚本。独立验证依赖留在本地 target，不成为产品依赖。

Pi 声明使用冻结字段 `capability`（不是 `capabilityId`）；独立元数据检查将路由、单文本输入结构和可选授权与冻结示例对照。候选生成器已支持 `--plugin trace`，仅在逐条绑定的预算和完整冷启动证据全部通过后才允许生成新目录。

尚未冻结、签名或发布；最终原生构建和服务检查、所有边界预算、完整独立冷启动及实际 Host 安装/权限/生命周期/Pi 都需如实推进。旧 text/diagnostics/transforms/inspect 候选及 Inspect 0.1.1 的三轮失败记录不被本插件覆盖。
