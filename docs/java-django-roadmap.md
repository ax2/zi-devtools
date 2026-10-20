# Java 与 Django 专项进度

更新：2026-10-18 · 程序开发版 0.82.0-dev.105。唯一状态源：[tools.json](tools.json)。

原有 14 项 Java/Django 规划均已提供首版，另保留 v0.17.0 的 Java 异常链和 Python Traceback。入口：左侧诊断工作台、首页 Java/Python 分类、Ctrl K、收藏/最近/常用与托盘。每项的输入、导出命令和已知限制见 [使用说明](java-django-tools.md)。 2026-10-15 起，14 项 JSON 报告统一包含生成工具版本和便携运行环境指纹；每个工具分别递增 patch 版本。

## 已实现

| 原优先级 | 工具 | 首版实际范围 |
| --- | --- | --- |
| P1 | JDK / 构建环境诊断 | 固定版本检查、两个 JDK 对比、JAVA_HOME 与构建入口；不运行 wrapper |
| P1 | Python / Django 环境诊断 | 解释器、venv、Django 分发/模块位置和脚本目录对比；不加载项目设置 |
| P1 | Java 线程转储 | jstack / Thread.print 文本的监视器锁等待线索；Thread.dump_to_file JSON 的线程容器、可用状态、栈帧与显式 virtual 标志，缺失字段保持未知 |
| P1 | Maven / Gradle 依赖 | Maven 常规坐标树、Gradle 版本选择与来源路径；可还原带 Selection reasons 标记的 dependencyInsight 反向路径 |
| P1 | Django 迁移计划 | 列表、显式依赖、循环/状态异常与 SQL 风险提示；不执行迁移 |
| P1 | SQL / N+1 | 请求分组、SQL 指纹、重复 SELECT 与耗时聚合；只标疑似 N+1 |
| P2 | GC 日志 | 常规统一日志 Pause 完成行的停顿/堆/时间线，非所有收集器 |
| P2 | JFR 报告 | JSON 事件或所选 JDK 转换小型录制，采样顶层热点、有界调用树与时间线 |
| P2 | Spring 配置对比 | 两份 JSON/YAML、差异/占位符/敏感键遮盖；不推导生效顺序 |
| P2 | Actuator | health/info/metrics 单次 GET，保留 503 health；不抓 env/configprops |
| P2 | Django URL / reverse | URL 导出脚本、名称/namespace/kwargs 名称检查，不执行 reverse |
| P2 | DRF OpenAPI | OpenAPI 3.x 端点、字段与共享组件差异；传播有界同文档 JSON Pointer Schema 引用，报告循环/未解析引用 |
| P2 | Django 部署检查 | 阅读 check --deploy 输出、编号分类与常见规则解释 |
| P2 | Celery 任务诊断 | 标准 worker Task 文本的失败/重试/耗时关联，不连接 broker |

## 架构决定

纯 Rust 核心与离线解析不依赖用户安装 Java/Python。环境检查与 JFR 采用三种固定原生适配器，而非开放任意进程插件：用户显式选择可信 java/python/jfr，固定参数、后台执行、输出与时间限制，Windows Job 管理生命周期。HTTP 只读适配器的请求地址与端点在执行前展示。

因此这批工具无需等待通用进程插件 SDK。现有声明式插件 V1 仍不能执行任意脚本/安装钩子；通用隔离协议、插件签名、Agent、MCP 会话与完整 RAG 继续保留在整体规划中。未来可以在协议与权限边界成熟后把这些结构化诊断能力作为 MCP 工具或模型解释的证据输入，不把模型建议直接转成数据库变更。

## 验证与范围

临时 Java 21 程序生成真实死锁、GC 和 JFR；独立 Django 5.1.6 / SQLite 项目生成迁移、重复查询、URL 和部署检查输出；Python 3.13 / 3.14 及临时 venv 对比验证环境探针。Actuator 本地 HTTP 夹具与各离线输入/边界测试覆盖其余首版范围。

已实现不代表完整 profiler、IDE、APM 或所有日志方言。更多 GC 方言、其他 dependencyInsight 输出方言、JFR 完整火焰图/JMC 深度分析、OpenAPI 非 Schema 引用、外部文档/锚点及完整 JSON Schema 方言，以及复杂 reverse 行为属于后续增强，未宣称已经完成。JFR 报告已提供有界采样调用树；其节点数、栈深与截断语义见专项工具说明。Thread.dump_to_file JSON 不含死锁检测且较早 JDK 可缺少虚拟类型字段；工具不会从容器或栈帧推断死锁/线程类型。

## 状态同步

原有 14 项专项工具均已提供首版，具体范围见上表；两项异常链 / Traceback 工具独立列入小工具目录。当前专项目录统计由 `docs/tools.json` 自动生成：

<!-- framework-catalog-summary:start -->
截至 2026-10-20 · 工具目录 v0.82.0：Java/JVM、Spring、Python/Django 与 Celery 专项诊断 14 项（14 项已实现，0 项规划 / 开发中）；另有 2 项异常链 / Traceback 工具。共 16 项，16 项已实现、0 项规划 / 开发中。来源：`docs/tools.json`。
<!-- framework-catalog-summary:end -->

插件迁移、版本及实现边界以各自文档为准；工具状态变更后运行 `python scripts/sync_tools.py` 和 `python scripts/sync_tools.py --check`，并同步实现证据、阶段记录及官网快照。
