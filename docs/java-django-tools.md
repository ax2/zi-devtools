# Java / Django 诊断工具使用说明

版本：v0.18.0。14 项专项工具已提供首版，原有 Java 异常链与 Python Traceback 继续保留。

## 打开与使用

左侧选择“Java / Django 诊断”，或在首页“Java 与 JVM”“Python 与 Django”分类中找到工具。Ctrl K 搜索名称、ID 或关键词，Enter 打开；Ctrl Enter 执行。工具可以单独收藏、进入最近/常用，并从托盘“开发工具”直达。

每个工具保留自己的会话输入和报告。离开页面不会丢失正在处理的结果，完成后显示提示。文本可粘贴或从 UTF-8 文件导入；报告可复制或保存到新文件，默认不覆盖已有文件。输入、报告与凭据不自动落盘；显式保存的报告可能包含业务信息，分享前脱敏。

通用限制：2 MiB / 20,000 行输入，报告最多 8 MiB；复杂依赖图和等待图另有限额，超限会明确报错，不悄悄截断为成功结果。

## Java 与 JVM

### JDK / 构建环境

选择可信 `java.exe` 的绝对路径，可点击“填入 PATH 入口”。可选第二个 JDK 路径进行对比。检查固定执行 `java -XshowSettings:properties -version`，只保留版本、供应商、java.home 和架构属性；显示 JAVA_HOME 路径差异、javac 是否存在以及 Maven/Gradle 的 PATH 入口。

不读取 IDE 配置、不执行项目 wrapper、不修改环境变量。不同完整版本只是差异，不自动断言项目构建不兼容；还需结合项目语言版本、Gradle/Maven 和插件要求。

### Java 线程转储

由用户在目标环境明确执行，例如 `jcmd <PID> Thread.print`，将文本导入。工具显示线程状态分布、帧、已持有的监视器锁、等待关系与循环涉及的线程索引。

支持标准平台线程文本；不支持虚拟线程 JSON，也不完整解析 ownable synchronizers。单次 WAITING/BLOCKED 不是死锁或线程池耗尽的证明。`explicitJvmDeadlockReport` 只在原文 JVM 明确报告死锁时为 true；`cycleThreadIndices` 是解析得到的等待环线索。最多 4096 线程、10000 等待边；复杂图需缩小范围。

### Maven / Gradle 依赖

输入示例来源：`mvn dependency:tree -Dverbose`、`gradle dependencies`、`gradle dependencyInsight --dependency <name>`。这些命令可能加载项目插件，应在可信项目中由用户自行运行；Zi DevTools 不执行它们。

首版支持常规 Maven 坐标/树、Gradle `group:artifact:version -> selected`、约束 `(c)`、重复子树 `(*)` 和 FAILED 标记。依赖报告保留输入行号、版本选择与路径。dependencyInsight 只提取坐标，不重建反向依赖树。多个 configuration/scope 的版本差异不能直接等同运行时冲突。

### GC 日志

导入现有 JDK 11–21 G1、Parallel 或 Serial 统一日志，识别带 `GC(n) ... Pause ... Nms` 的完成行。输出暂停次数、总量、最大值、最近秩 P95、堆变化和时间线。频率仅基于首末识别事件的时间跨度。

不解释 ZGC/Shenandoah 的全部特有事件，也不统计并发阶段 CPU 开销；无可识别行时会提示格式不支持。不得把“未识别”解释为“没有 GC”。

### JFR 录制报告

两种入口：

1. 粘贴或读取 `jfr print --json` 输出，最多 8 MiB / 20000 事件。
2. 选择可信 JDK 的 `jfr.exe` 和 ≤64 MiB `.jfr` 文件，点击“转换 .jfr 并分析”。固定读取 `jdk.ExecutionSample`、`jdk.GarbageCollection`、`jdk.JavaMonitorEnter`、`jdk.ThreadPark` 四类事件，输出最多 8 MiB、30 秒超时。

报告包含事件数量、时间线、可解析的 ISO 时长，以及 ExecutionSample 顶层帧热点。热点是样本计数，不是 CPU 百分比；没有完整调用树、火焰图或 JMC 的所有能力。所选 JDK 是可选外部解析器，不随核心程序捆绑。

### Spring 配置差异

左右粘贴两份 JSON 或单文档 YAML，例如不同 profile 的脱敏导出。报告使用 JSON Pointer 路径，区分新增、删除、变化和未解析 `${...}` 占位符。数组作为整体比较。

password、secret、token 等敏感键对应值自动遮盖，但无法识别任意业务密钥；输入仍应先脱敏。不直接解析 `.properties`，不解析占位符，不模拟 profile、环境变量与命令行的实际覆盖优先级。

### Spring Actuator

输入 Actuator 基础地址，选择 `health`、`info`、`metrics`，或填写 `metrics/jvm.memory.used` 等单个指标端点。点击运行才发送 GET。远程地址仅支持 HTTPS，HTTP 仅允许回环；不跟随重定向、不使用环境代理。可选 Bearer 令牌只在内存保留，切换工具/页面时清空；已发出的请求持有自身副本直至结束。

不枚举 env/configprops，不发送管理写请求。JSON health 的 503/DOWN 仍作为有效诊断证据展示 HTTP 状态。非 JSON 响应报错，连接超时 5 秒、总超时 20 秒、响应上限 2 MiB。

## Python 与 Django

### Python / Django 环境

选择可信 `python.exe`，可选第二个解释器进行比较。固定以 `-I` 运行探针，读取 Python 版本、prefix/basePrefix、虚拟环境状态、Django 分发版本/位置与模块定位。脚本目录来自该解释器的 sysconfig，用于与 PATH 的 django-admin 入口比较。

探针不 `import django`、不加载项目 settings，不安装包。解释器自身仍可能执行已安装的 site/.pth 启动钩子，因此需要选择可信环境。目录不一致是线索，不证明 CLI 不可用。核心应用不依赖 Python。

### Django 迁移计划

推荐导入 `python manage.py showmigrations --plan --verbosity 2` 输出，其明确列出依赖。也支持基础 showmigrations 列表和 migrate --plan 的迁移标题；没有依赖信息时不会根据顺序伪造边。

可在第二输入框放入 `python manage.py sqlmigrate app migration` 的 SQL。报告显示已应用状态、缺失依赖、状态不一致、循环与可能分支；SQL 根据标记化语句提示 DROP/TRUNCATE、无 WHERE DELETE 和 ALTER 风险。它不是完整方言分析器，也不能证明操作安全。

这些导出命令需要加载可信项目设置，可能连接数据库，必须由用户在合适环境自行执行。工具本身不运行 manage.py、不 migrate、不回滚、不 --fake。

### SQL / N+1

输入脱敏 JSON 数组：

```json
[
  {"requestId":"request-a","sql":"SELECT name FROM users WHERE id=1","durationMs":2.5},
  {"requestId":"request-a","sql":"SELECT name FROM users WHERE id=2","durationMs":3.0}
]
```

可从用户明确采集的 `connection.queries` 导出；其中 `time` 是秒，应乘 1000 转为 durationMs，并补充请求分组。不要把多个请求无标记混在一起。

工具移除注释、归一化数据字面量、保留标识符，按 requestId 和 SQL 指纹聚合计数、总/平均耗时。同请求至少三次同形 SELECT 标记疑似 N+1；缺少 requestId 时仅报告跨记录重复，不能推断 N+1。耗时总和不是请求墙钟时长。最多 10000 条。

### URL / reverse

在可信项目 Django shell 中显式运行 [URL 导出脚本](../scripts/export_django_urls.py)，只输出 route、name、namespace、parameters 和 regexRoute 元数据，不导出回调源码或设置。示例：

```powershell
python manage.py shell -c "exec(open('C:/tools/zi-devtools/scripts/export_django_urls.py', encoding='utf-8').read())" > urls.json
```

导入数组后检查完整命名空间和同名路由；重复名称可能是合法重载。可在第二输入框使用 `{"name":"api:detail","kwargs":{"pk":42}}` 检查参数名称。首版不实际 reverse、不验证转换器的值或 defaults，不处理位置 args；正则路由须导出 parameters。

### DRF OpenAPI

左右输入旧版与新版 OpenAPI 3.x JSON，例如由项目已有 schema 生成器导出的文件。比较端点移除/新增、operation 字段与共享 components。端点移除标记为破坏性；required/type/enum 等变化提示可能不兼容。

不支持 Swagger 2，不展开外部/循环 `$ref`；共享组件差异单独显示，需复核引用处。数组按整体比较，顺序变化也可能产生差异。不执行 API、不验证运行时权限或数据库行为。

### 部署检查

输入 `python manage.py check --deploy` 的脱敏标准输出。按检查编号与严重度分类，常见安全配置编号给出针对性提示，未知编号保留消息。只有原文明示 `System check identified no issues` 才标记无问题；无法证明线上配置已生效，不自动修复。

### Celery 任务诊断

输入按时间排序的标准 worker `Task name[id] received/started/succeeded/failed/raised unexpected/retry/retried/revoked` 文本。按任务 ID 关联失败、重试、耗时和最后观测状态，不展示任务参数与返回值。

这不是 broker 实时状态。缺失、乱序、自定义日志格式无法完整还原生命周期；不连接 broker、不取消或重放任务。

## 进程与验证边界

环境检查使用固定参数、无 shell 拼接、不运行 batch/wrapper；移除 JVM agent/options 和 Python 路径等干扰环境变量。版本检查单次最多 15 秒、64 KiB 输出；JFR 为 30 秒。Windows Job 在结束/超时/输出超限时清理检测进程。Job 是生命周期控制，不是恶意程序安全沙箱；不能把不可信程序改名 java.exe 后当作安全输入。

本轮以临时 Java 21 程序验证真实线程死锁、GC 与 JFR，以独立 Django 5.1.6 + SQLite 夹具验证迁移、SQL、URL 和部署检查；未连接用户业务数据库。另验证 Python 3.13、Python 3.14 和无 Django 的临时 venv。Actuator 使用本地 HTTP 夹具验证 GET、Bearer 与 503 health 行为，未声称连接真实 Spring 服务。Maven/Gradle、OpenAPI、Spring 配置、Celery 以支持格式夹具验证，没有声称已运行相应项目。

后续增强包括虚拟线程、更多依赖/GC 方言、完整 JFR 调用树、自动引用追踪和更丰富的规则。当前状态只覆盖本文首版范围，不等于完整 APM、IDE 或自主 Agent。

## 开发者复验

常规 `cargo test --locked` 不需要 Java/Python。可选真实框架复验：先准备可信 JDK 21、已安装 Django 的 Python，以及未安装 Django 的比较解释器，再运行：

```powershell
./scripts/verify-framework-fixtures.ps1 -JavaHome 'C:/path/to/jdk' -Python 'C:/path/to/django-python/python.exe' -ComparisonPython 'C:/path/to/empty-venv/Scripts/python.exe'
```

脚本只建立独立临时项目/SQLite、编译随仓库提供的 Java 夹具，生成线程/GC/JFR/迁移/SQL/URL/check 输入，并用 `framework_verify` 执行报告断言。临时证据目录保留，不触碰现有项目或数据库。该可选流程不在普通 CI 隐式安装运行时。
