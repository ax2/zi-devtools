# Java 与 Django 工具路线

更新：2026-09-26 · v0.17.0。状态以 [tools.json](tools.json) 为准。

值得开发。优先选择“已有日志或输出 → 可读诊断”的高频任务，再扩展需要项目环境的检查。Java 和 Django 不作为核心程序运行依赖；重型解析与运行环境检查通过后续插件协议按需提供。

## 已实现：两种离线堆栈整理

在工具目录搜索 `Java`、`Spring`、`Django`、`Python` 或 `traceback`，也可从“扩展与集成”分类进入，收藏后可以直达。

- **Java 异常链**：粘贴标准 printStackTrace 文本，整理异常类型、消息、cause/suppressed 父子关系、调用位置和省略帧数量。分别保留多个异常根节点，suppressed 分支不会替代主 cause 链末端。支持模块前缀、Native Method 和 Unknown Source 位置。
- **Django / Python Traceback**：粘贴标准文本回溯，整理文件、行号、函数、异常类型、链式关系与最后可见帧。对 URL reverse、模板、数据库、迁移、配置和模块导入等常见异常给出规则提示；不提供无条件放宽 ALLOWED_HOSTS 等建议。

输出为可复制 JSON。单次输入最多 1 MiB / 20,000 行，额外限制异常和帧数量。没有网络请求、不运行项目、不导入 settings、不访问数据库。输出可能包含原日志中的业务信息，分享前应自行脱敏。

**边界**：整理异常结构不能证明真实根因；Java 不展开省略帧、不处理所有日志前缀；Python 不支持 HTML 调试页和 ExceptionGroup 树，截断片段会标记不完整。规则提示是排查起点，修复需结合代码与运行环境。

## 下一批优先工具

| 优先级 | 工具 | 开发价值 | 首版验收范围 |
| --- | --- | --- | --- |
| P1 | Java 线程转储分析 | 卡死、锁等待和线程池耗尽是常见排障成本 | 导入 jstack/jcmd 文本，线程状态聚合、锁等待关系；已证实死锁与单次采样阻塞分开；注明虚拟线程支持范围 |
| P1 | Maven / Gradle 依赖冲突 | 升级后 NoSuchMethodError、类重复通常难以追踪 | 解析 dependency:tree / dependencies / dependencyInsight 的支持格式，显示版本选择及来源路径；不自动执行 wrapper |
| P1 | JDK / 构建环境诊断 | IDE、JAVA_HOME、终端 JDK 不一致容易造成构建问题 | 用户选择入口后对比版本和路径，提示兼容范围；不自动改系统环境 |
| P1 | Python / Django 环境诊断 | django-admin 与 python 属于不同解释器时，“已安装”仍会导入失败 | 显式选择解释器，检查版本和 Django 模块位置；不加载项目 settings，不自动安装包 |
| P1 | Django 迁移计划检查 | 多分支迁移冲突、危险 SQL 和遗漏依赖影响上线 | 导入 showmigrations、plan、sqlmigrate 输出，展示依赖和风险；不执行 migrate、回滚或 --fake |
| P1 | Django SQL / N+1 分析 | 页面变慢时需要快速找到重复查询和高耗时请求 | 导入脱敏 SQL 和请求分组，聚合同类查询；缺少请求上下文时只标“疑似 N+1” |

建议实施顺序：**解释器环境诊断 → Java 线程转储 → Django 迁移计划 → 依赖冲突 → SQL / N+1**。前两类覆盖日常环境和运行问题，其余涉及更多输入格式，需要独立夹具库。

## 后续扩展

| 方向 | 工具 | 边界 |
| --- | --- | --- |
| JVM 性能 | GC 日志、JFR 事件报告 | 支持范围明确到日志/事件版本；JFR 使用可选解析器；不替代 VisualVM/JDK Mission Control |
| Spring | 配置差异、Actuator health/info/metrics | 配置先脱敏；不默认读取 env/configprops，不声称从静态文件推导完整生效配置 |
| Django / DRF | URL 与 reverse、OpenAPI 契约差异 | 导出项目元数据可能执行 Python，必须由用户明确运行；静态 schema 不等同权限验证 |
| Django 部署 | check --deploy 报告阅读器 | 解析现有输出并解释检查编号；不自动修改生产设置 |
| Celery | 任务失败、重试与耗时关联 | 首版离线导入，不连接 broker、不重放任务 |

## 与 Agent、MCP、RAG 的组合

诊断先产生有来源行号与确定边界的结构化结果，再允许用户选择模型进行解释。模型结论单独显示为建议，保留原始证据；不把模型建议直接转换成迁移、系统环境修改或服务控制。后续可将这些只读诊断能力暴露为 MCP 工具，项目知识库用于检索版本文档与团队排障手册。

当前插件 V1 仅支持内置配方和 HTTP JSON 适配器，**尚不能安装任意 JVM/Python 进程插件**。需先完成受控进程协议、超时、输出限制与显式项目选择，再提供环境诊断插件。无需为了日志整理打包整个 JDK 或 Python。

## 本机验证对规划的影响

本轮只读验证发现 Java 21 与 Django 5.1.6 可用，但默认 Python 3.14.7 没有 Django；Django 安装在 Python 3.13 环境。这个差异是环境诊断工具的具体使用场景，不意味着安装损坏。Maven/Gradle 在本轮 PATH 中未发现，不代表未安装。应用新增 Java/javac/Maven/Gradle/django-admin 的 PATH 入口检测，仅检查文件存在，不执行版本命令。

## 状态同步

本轮落地 2 项工具，新增 14 项规划。总清单 36 项内置已实现、80 项规划，插件示例单独统计。每项完成后必须有工作入口、支持格式、异常边界与测试证据，之后才能由 planned 改为 implemented，并同步 README、生成清单与官网。
