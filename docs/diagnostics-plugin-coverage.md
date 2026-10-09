# 诊断插件0.1.1范围与验收边界

提供方契约沿用contracts/diagnostics/v1（1.0.0-rc.1提案），插件包和core/adapter各0.1.1。本表不表示Studio接受。所有11动作共享双输入8192 UTF8字节、请求/结果48KiB、严格外壳；portable预算向量覆盖ASCII/中文精确输入、超限、精确请求、未知字段、无效报告、大结果；共享序列化example另验证ASCII/中文/转义的49151/49152/49153字节结果。实际运行摘要与fuel按交付fixtures核对。

| 能力 | 已实现的导入报告范围 | 明确缺口 |
| --- | --- | --- |
| java.threads | 平台线程状态、锁等待边、环与JVM显式死锁文字 | ownable synchronizer全部格式、虚拟线程JSON、实时采集 |
| java.dependencies | Maven/Gradle坐标、版本选择、约束/重复树/失败标记、路径 | dependencyInsight反向路径、跨configuration运行时冲突证明、执行构建 |
| java.gc | 常规Pause完成行、堆、暂停统计/分位数/采样频率 | ZGC/Shenandoah专有事件、并发CPU、完整进程频率 |
| java.jfr_json | JFR JSON类型计数、duration、顶层ExecutionSample样本、时间线 | 二进制JFR、完整调用树/火焰图、CPU百分比 |
| spring.config | JSON/单文档YAML差异、Pointer、敏感键遮盖、未解析占位符 | properties原文、有效配置优先级、任意业务秘密识别、Actuator |
| django.migrations | 显式依赖、循环/缺失/未应用警告、叶分支线索、SQL风险 | 无依赖时无法证明分支、完整SQL方言、数据库/迁移执行 |
| django.sql | 字面量/注释指纹、请求分组、耗时/重复SELECT线索 | SQL语义等价、N+1证明、数据库执行 |
| django.urls | 导出列表、重名、kwargs缺失/额外名称、Unicode参数 | converter值、正则/defaults实际reverse、项目加载 |
| django.openapi | OpenAPI3端点/字段/components差异、共享参数 | 外部/循环$ref解引用、请求响应方向完整兼容性、联网 |
| django.checks | 编号/级别/行号、固定安全提示、明确无问题 | manage.py执行、配置实际生效、自动修复 |
| celery.report | 输入顺序任务关联、失败/重试/持续时间/异常类型 | broker实时状态、完整事件还原、非标准日志、主动重试 |

验证不枚举任意合法输入或全部框架版本。图可达性仍有复杂度保护；病态深层/高边数报告需单独评估，不能用有限fixtures声称全输入1000万fuel保证。Host安装、授权、撤权、取消、升级/回滚、签名和真实Pi调用由Studio单独验收，154项迁移accepted仍0。

业务错误仅允许INVALID_INPUT、INPUT_TOO_LARGE、UNSUPPORTED_OPERATION、INVALID_REPORT、OUTPUT_TOO_LARGE；固定中文消息不回显报告/底层错误。Host Error.code为可选兼容字段，View固定映射fuel/memory/stack/timeout/cancelled/busy/disabled/failed及未知fallback。页面等待超时不代表Host已取消或回滚。没有声明新diagnostics/output.schema.v1 requiredProfiles，不要求扩大Host预算。
