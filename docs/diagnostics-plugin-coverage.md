# 诊断插件0.1.4范围与验收边界

WASI外层沿用公共`compute.wasi.v1@=1.0.0-rc.1`与`output.schema.v1@=1.0.0-rc.1`，并声明`tool.directory.v1@=1.0.0-preview.1`；内部诊断报告提案升为`contracts/diagnostics/v2`（2.0.0-rc.1）。插件包、core/adapter各0.1.4。新增线程转储 JSON 向量，验证 JDK 容器与显式 virtual 字段，旧格式缺字段保持未知。manifest不要求Host尚未支持的diagnostics requiredProfile；逐操作输入/报告schema分别列明secondary适用性。本表不表示Studio接受。各操作输入最多8192 UTF8字节、请求/结果最多48KiB；portable预算向量覆盖ASCII/中文精确输入、超限、精确请求、未知字段、无效报告、大结果；共享序列化example另验证ASCII/中文/转义的49151/49152/49153字节结果。实际运行摘要与fuel按交付fixtures核对。

成功报告现在封装为`diagnosticsProtocol`、`generatedBy`和`data`。来源版本读取生成的目录常量；runtime fingerprint固定描述`wasi/wasm32/plugin` portable profile，确保原生/WASI候选向量逐字节一致，不描述实际Host环境。

0.1.4源声明对11能力使用内联输出schema与49152字节上限，校验整个成功/错误外壳；字符串内部报告schema由提供方离线验证，不能声称Host会自动解析报告。tool.directory.v1 1.0.0-preview.1声明code分类、中英文别名与导入报告用途。新源声明和静态预检通过不等于新候选已完成交付。

| 能力 | 已实现的导入报告范围 | 明确缺口 |
| --- | --- | --- |
| java.threads | 文本及Thread.dump_to_file JSON、JDK容器、显式virtual字段、状态/锁等待边/死锁线索 | ownable synchronizer全部格式、缺失virtual字段时不能判断类型、实时采集 |
| java.dependencies | Maven/Gradle坐标、版本选择、约束/重复树/失败标记、带Selection reasons标记的dependencyInsight反向路径 | 任意Gradle方言、Maven反向路径、跨configuration运行时冲突证明、执行构建 |
| java.gc | 常规Pause完成行、堆、暂停统计/分位数/采样频率 | ZGC/Shenandoah专有事件、并发CPU、完整进程频率 |
| java.jfr_json | JFR JSON类型计数、duration、顶层ExecutionSample样本、时间线及最多256节点/128层采样调用树 | 二进制JFR、无限规模调用树/完整火焰图、CPU百分比 |
| spring.config | JSON/单文档YAML差异、Pointer、敏感键遮盖、未解析占位符 | properties原文、有效配置优先级、任意业务秘密识别、Actuator |
| django.migrations | 显式依赖、循环/缺失/未应用警告、叶分支线索、SQL风险 | 无依赖时无法证明分支、完整SQL方言、数据库/迁移执行 |
| django.sql | 字面量/注释指纹、请求分组、耗时/重复SELECT线索 | SQL语义等价、N+1证明、数据库执行 |
| django.urls | 导出列表、重名、kwargs缺失/额外名称、Unicode参数 | converter值、正则/defaults实际reverse、项目加载 |
| django.openapi | OpenAPI3端点/字段/components差异、共享参数、同文档Schema引用传播及3.0/3.1相邻字段区别，循环/未解析引用显式报告 | 非Schema引用完整解析、远程引用、无限循环展开、请求响应方向完整兼容性、联网 |
| django.checks | 编号/级别/行号、固定安全提示、明确无问题 | manage.py执行、配置实际生效、自动修复 |
| celery.report | 输入顺序任务关联、失败/重试/持续时间/异常类型 | broker实时状态、完整事件还原、非标准日志、主动重试 |

验证不枚举任意合法输入或全部框架版本。图可达性仍有复杂度保护；病态深层/高边数报告需单独评估，不能用有限fixtures声称全输入1000万fuel保证。Host安装、授权、撤权、取消、升级/回滚、签名和真实Pi调用由Studio单独验收，154项迁移accepted仍0。

业务错误仅允许INVALID_INPUT、INPUT_TOO_LARGE、UNSUPPORTED_OPERATION、INVALID_REPORT、OUTPUT_TOO_LARGE；固定中文消息不回显报告/底层错误。Host Error.code为可选兼容字段，View固定映射fuel/memory/stack/timeout/cancelled/busy/disabled/failed及未知fallback。页面等待超时不代表Host已取消或回滚。不声明尚未支持的诊断专属profile，不要求扩大Host预算；通用output.schema.v1和tool.directory.v1按manifest明确协商。

0.1.3冷验证区分官方独立C-API36.0.2与Studio生产EXE；View用实际Edge页面和本机WASI模块验证11项操作、两种视口/主题、截图及页面错误。不使用Studio源码/EXE替代独立工具，也不宣称诊断profile已采纳。

## 未纳入 WASI 候选的桌面能力

下列三个独立工具仍在桌面版中实现，未出现在 0.1.4 的11项纯计算插件 capability 中。`compute.wasi.v1` 不提供文件读取、环境枚举、进程启动或网络访问；不能通过本机 HTTP、任意路径、Shell 命令或EXE委托绕过该边界。只有Studio提供并验收对应的版本化 Host API 和权限后，才能另行构建候选。报告导入/摘要规范化等纯计算部分可以继续复用 core；实际探测应作为单独 Host 操作。

| 源工具 ID / 当前版本 | 可复用的便携部分 | 未交付的桌面操作及所需 Host 能力 | 授权、取消与边界 |
| --- | --- | --- | --- |
| `java-environment` / 1.1.1 | 对显式提供的 Java 属性文本做摘要解析 | 用户选择绝对 `java[.exe]` 路径；Host 以固定参数 `-XshowSettings:properties -version` 启动一次，解析属性；可选比较第二个显式选择的 JDK。若继续显示 Maven/Gradle 是否在 PATH，仅允许 Host 做受限 PATH 查找，不执行这些程序。需要受信任路径的固定参数进程 API。 | 每个可执行文件都由用户选择；不接收 Shell/任意参数/标准输入；限制工作目录、环境继承、时长和输出字节；取消只终止本次 Host 创建的进程树。Windows 先行，其他 OS 由各自 Host 实现。 |
| `django-environment` / 1.1.1 | 对显式提供的 Python/Django 属性文本做摘要解析 | 用户选择绝对 `python[.exe]`；Host 以固定 `-I -c <固定探测脚本>` 检查解释器、venv、Django/Celery 是否可导入及脚本目录；可选比较第二个显式选择的解释器。需要受信任路径的固定参数进程 API。 | 不导入项目 `settings`、不运行 `manage.py`、不触碰数据库；用户逐个选择解释器；取消终止本次进程树。Python `-I` 不是安全沙箱，系统 site 启动钩子仍需作为风险说明；Windows 先行，跨平台需分别验证。 |
| `spring-actuator` / 1.1.1 | 对 Host 明确允许的 health/info/metrics JSON 做 View 展示/摘要 | `plugins/spring-actuator/` 已有独立 HTTP-only View 候选，并把任务生命周期、报告资源分开写成未声明 proposal；候选不等于 Studio 支持或接受。 | Studio HTTP 草案要求逐次确认、DNS 地址绑定、无代理/重定向/凭据、HTTPS 公网或明确确认的 loopback/RFC1918 HTTP、连接3秒/总时长8秒/响应1 MiB。候选没有可选 Bearer、单指标 `metrics/<name>` 或 context path；相关功能仍未移植。 |

这些仍是接口草案与提供方候选，不代表 Studio 已提供任务或网络权限。迁移表保留 HTTP 摘要为候选能力，但 `acceptedCapabilities` 仍为 0；后台任务和报告导出保持 `not_delivered`。任务生命周期 proposal 对齐 Studio 的 `plugin.task.lifecycle.v1@1.0.0-preview.1` 文档草案及五个 `/v2/plugin-tasks/*` 路由，但 profile 尚未实现/发布，且不会出现在 manifest `requiredProfiles`。报告继续等待 Studio 独立结果/资源 profile。

`java-jfr` / 1.2.1 是部分迁移而非第四个完全遗漏工具：候选仅接收不超过8192 UTF-8字节的 `jfr print --json` 导出文本。桌面版对用户显式选中的 `.jfr`（最多64 MiB）调用受信任 JDK 的 `jfr print --json --events jdk.ExecutionSample,jdk.GarbageCollection,jdk.JavaMonitorEnter,jdk.ThreadPark <file>`，需要另一个限定文件读取 + 固定参数进程能力，最多30秒、结果最多8 MiB；该二进制转换不属于当前 WASI 候选。Host 需绑定被选文件和可执行文件句柄/规范路径，禁止任意路径探测与额外参数。

Studio 对 Actuator 的服务接口预览见 [`contracts/studio-host-capabilities/service-http-get-preview-20261015/`](../contracts/studio-host-capabilities/service-http-get-preview-20261015/)；旧的单文件适配原型为 [`plugins/diagnostics/views/spring-actuator-preview.html`](../plugins/diagnostics/views/spring-actuator-preview.html)。当前独立候选在 [`plugins/spring-actuator/`](../plugins/spring-actuator/)，只使用 `window.zicode.services.http.get`，没有 fetch/WASI 网络回退。HTTP profile 自身仍处于 Studio 未实现草案；本候选的输入/响应 schema 和有限自定义验证器属于提供方待复核素材，不构成 Studio conformance、正式包或端到端验收。源工具完整范围仍未交付，接受矩阵仍为 0/154。
