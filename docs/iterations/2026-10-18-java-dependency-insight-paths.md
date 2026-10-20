# Gradle dependencyInsight 反向路径

日期：2026-10-18

## 目标

补齐 Java 依赖诊断中 dependencyInsight 只提取坐标、无法回答“哪些依赖路径引入它”的缺口。实现依据 Gradle 官方文档展示的 `Selection reasons:` 区段及反向树格式。

## 实现

- 对带 `Selection reasons:` 的 Gradle 报告，识别后续目标坐标和 `+---` / `\---` 连接符，重建报告实际打印的配置/项目到目标组件路径。
- 保留目标请求版本和选中版本；直接配置路径与多级传递路径均输出。普通 `dependencies` 树和无标记报告保持原解析行为。
- 每目标节点最多 2048 项、目标最多 1000 个、单路径最多 64 层、结果最多 128 条；标签最多 240 字符。超限通过 `truncated` 明示，不推断未输出依赖关系。
- `java-dependencies` 提升到 1.3.0，并同步工具目录、路线图、使用说明、WASI 示例和本轮日志。

## 验证

以 Gradle 官方文档中的 commons-codec 示例构造夹具，断言直接路径、httpclient/JGit 传递路径及版本选择，并确认常规依赖树不增加 insight 字段。`cargo test -p zi-diagnostics-core --locked` 通过（15 passed）；`cargo clippy -p zi-diagnostics-core --all-targets --locked -- -D warnings` 与目标 crate fmt 检查通过。native 与 wasm32-wasip1 release 适配器重建成功，230 条 native/WASI 字节级一致性用例通过，其中一条校验本功能的多路径和版本语义。工具目录及诊断插件元数据同步检查通过。

## 遗留

Gradle 输出会随版本、插件和报告选项变化；当前只处理带显式选择原因标记且具有可识别树连接符的输出。Maven 反向依赖分析、所有 Gradle 版本方言和桌面实际报告布局仍未覆盖。
