# Java / Django 堆栈分析插件（提供方候选说明）

`com.zicode.devtools.trace/0.1.0` 使用同仓 `zi-trace-core`，独立桌面程序直接调用同一个核心。根程序仍为 `0.82.0-dev.105`，两项源工具为 `1.0.3`。冻结交付以独立 release 候选目录及其外部 verification 为准；unsigned候选不代表实际Studio接受，也不等于整个154项工具迁移完成。

| 操作 | 完整既有范围 |
| --- | --- |
| `java.trace` | 标准 printStackTrace；独立异常、cause/suppressed 层级、主链终点、模块/Native/Unknown Source 帧及省略帧计数。省略帧不还原，终点不等于确认根因。 |
| `django.trace` | 文本 Python Traceback、直接因果/处理期间/独立异常链、路径/行号/函数、缺头或截断标记及既有 Django 规则提示。不运行项目，不支持 HTML 调试页或 ExceptionGroup 树。 |

冻结 rc.1 外层保持不变；只特化插件 ID。输入 `text` 最多 8192 UTF-8 字节，完整请求与序列化结果最多 48 KiB；2 MiB 模块、10M fuel、5秒、64MiB 内存和2MiB栈。所有路径只是粘贴文本，不访问文件、环境、进程或网络。View 通过公共 SDK 调用能力；可选 Pi 声明需要 Host 单独授权。

原正则实现完整保留为测试专用参照 `crates/zi-trace-core/src/legacy.rs`。运行时改为固定语法扫描器，Unicode `\w`/`\d` 范围由锁定的 regex-syntax 在构建时生成；测试比较捕获组、CRLF/Unicode/空白、完整报告与错误。正则不进入该插件运行依赖，不缩减既有功能。

最终模块241,191字节，SHA256 `bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e`。59条实际native/Node-WASI逐字节对照、独立Wasmtime 36.0.2计算预算和全新进程冷启动全部通过。最大fuel 7,932,681、执行5ms、内存2,686,976字节；冷启动最大504.374ms，59个所属子进程全部关闭。保持原10M/5秒/64MiB/2MiB栈预算。

向量覆盖350/351/352/512节点的超大报告、513节点解析错误、精确UTF-8边界、重复键、类型错误和恶意重复分隔符。只对必定超限的输出使用保守下界提前拒绝，其他超大报告仍由实际序列化器拒绝；独立版完整报告不受插件结果限制。测试检查真正输出的字节，不能要求所有内部中间结果都已经携带错误。初次正则提取13条预算失败、512节点构造超时和错误测试假设均保留在技术档案，没有删用例或放宽预算。

最终公开SDK注入+实际WASI View通过2操作、1180浅色/390深色、搜索、草稿恢复、业务/宿主错误保留旧结果、UTF-8预算、过期响应、Ctrl+Enter/Ctrl+K、选中复制及无Host禁用。两张截图已目视检查，证据绑定上述最终模块及View摘要。这不是原生独立版或实际Studio UI/Pi验收。

交付catalog、59结果及19成功请求/输入通过实际JSON Schema校验；请求schema只特化插件ID，冻结rc.1原件保持不变。

复现：`python scripts/sync_trace_plugin.py --check`；通过 `scripts/dev.ps1` 构建 `zi-trace-wasi` 原生与现有 plugin/WASI profile，再运行 `verify_transforms_wasi.py --plugin trace`、`verify_transforms_schema.py --plugin trace`，独立预算/冷启动使用既有验证脚本。独立验证依赖留在本地 target，不成为产品依赖。

Pi 声明使用冻结字段 `capability`（不是 `capabilityId`）；独立元数据检查将路由、单文本输入结构和可选授权与冻结示例对照。候选生成器已支持 `--plugin trace`，仅在逐条绑定的预算和完整冷启动证据全部通过后才允许生成新目录。

原生优化构建19m20s完成，新EXE隔离服务启停2171ms通过、exit0且端口释放；根dev105不变。最终工作区回归和当前主CI分别记录，不把其他工作流成功当作主CI通过。没有签名或正式发布；实际Host安装/权限/生命周期/Pi未验收。旧 text/diagnostics/transforms/inspect 候选及 Inspect 0.1.1 的三轮失败记录不被本插件覆盖。
