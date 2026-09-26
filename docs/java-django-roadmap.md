# Java 与 Django 专项进度

更新：2026-09-26 · v0.18.0。唯一状态源：[tools.json](tools.json)。

原有 14 项 Java/Django 规划均已提供首版，另保留 v0.17.0 的 Java 异常链和 Python Traceback。入口：左侧诊断工作台、首页 Java/Python 分类、Ctrl K、收藏/最近/常用与托盘。每项的输入、导出命令和已知限制见 [使用说明](java-django-tools.md)。

## 已实现

| 原优先级 | 工具 | 首版实际范围 |
| --- | --- | --- |
| P1 | JDK / 构建环境诊断 | 固定版本检查、两个 JDK 对比、JAVA_HOME 与构建入口；不运行 wrapper |
| P1 | Python / Django 环境诊断 | 解释器、venv、Django 分发/模块位置和脚本目录对比；不加载项目设置 |
| P1 | Java 线程转储 | 平台线程状态、监视器锁等待关系、循环线索和原文明确死锁报告 |
| P1 | Maven / Gradle 依赖 | 常规坐标树、版本选择与来源路径；insight 仅提取坐标 |
| P1 | Django 迁移计划 | 列表、显式依赖、循环/状态异常与 SQL 风险提示；不执行迁移 |
| P1 | SQL / N+1 | 请求分组、SQL 指纹、重复 SELECT 与耗时聚合；只标疑似 N+1 |
| P2 | GC 日志 | 常规统一日志 Pause 完成行的停顿/堆/时间线，非所有收集器 |
| P2 | JFR 报告 | JSON 事件或所选 JDK 转换小型录制，采样顶层热点与时间线 |
| P2 | Spring 配置对比 | 两份 JSON/YAML、差异/占位符/敏感键遮盖；不推导生效顺序 |
| P2 | Actuator | health/info/metrics 单次 GET，保留 503 health；不抓 env/configprops |
| P2 | Django URL / reverse | URL 导出脚本、名称/namespace/kwargs 名称检查，不执行 reverse |
| P2 | DRF OpenAPI | OpenAPI 3.x 端点、字段与共享组件差异；不展开外部/循环引用 |
| P2 | Django 部署检查 | 阅读 check --deploy 输出、编号分类与常见规则解释 |
| P2 | Celery 任务诊断 | 标准 worker Task 文本的失败/重试/耗时关联，不连接 broker |

## 架构决定

纯 Rust 核心与离线解析不依赖用户安装 Java/Python。环境检查与 JFR 采用三种固定原生适配器，而非开放任意进程插件：用户显式选择可信 java/python/jfr，固定参数、后台执行、输出与时间限制，Windows Job 管理生命周期。HTTP 只读适配器的请求地址与端点在执行前展示。

因此这批工具无需等待通用进程插件 SDK。现有声明式插件 V1 仍不能执行任意脚本/安装钩子；通用隔离协议、插件签名、Agent、MCP 会话与完整 RAG 继续保留在整体规划中。未来可以在协议与权限边界成熟后把这些结构化诊断能力作为 MCP 工具或模型解释的证据输入，不把模型建议直接转成数据库变更。

## 验证与范围

临时 Java 21 程序生成真实死锁、GC 和 JFR；独立 Django 5.1.6 / SQLite 项目生成迁移、重复查询、URL 和部署检查输出；Python 3.13 / 3.14 及临时 venv 对比验证环境探针。Actuator 本地 HTTP 夹具与各离线输入/边界测试覆盖其余首版范围。

已实现不代表完整 profiler、IDE、APM 或所有日志方言。虚拟线程 JSON、更多 GC 方言、dependencyInsight 反向树、JFR 完整调用树、OpenAPI 引用传播与复杂 reverse 行为属于后续增强，未宣称已经完成。

## 状态同步

本轮将 14 项 planned 转为 implemented，保留具体支持范围。总清单为 50 项内置已实现、66 项规划（共 116 项）；3 个插件示例包 / 6 项可选工具单独统计。通过代码入口校验与生成脚本同步 README、工具文档及官网。
