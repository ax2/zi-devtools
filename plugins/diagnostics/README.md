# ZiDevTools Diagnostics 0.1.4 - command adapter

本仓库的 `zi-diagnostics-core` 同时供原生桌面端和 `zi-diagnostics-wasi` 使用。11 项能力可分析导入报告；不会启动 JVM/Python、运行迁移或 SQL、连接 Broker、读取用户文件或访问网络。提供自包含 View、元数据及 unsigned 候选目录构建脚本，**没有正式签名发行包，也未通过 Studio 接入验收**。

0.1.4 源声明已为全部11能力添加 `output.schema.v1@=1.0.0-rc.1` 的内联成功/错误外壳 schema 与 `maxResultBytes=49152`，同步快照为 `contracts/diagnostics/v2/result.schema.json`。这项门禁校验整个 JSON envelope，不解析 `data.text` 内的诊断报告。`tool.directory.v1@=1.0.0-preview.1` 同时声明 code 分类、中英文别名与“仅分析导入报告”的范围说明。Studio 只读静态预检通过；0.1.4已完成258项唯一用例的原生/WASI、独立fuel与冷启动验证，9项序列化边界及两种主题视口的22次View调用；旧0.1.3候选保持原样。提供方验证不代表Studio Host验收。

输出验证使用开发侧 `jsonschema==4.25.1`，安装在用户验证缓存并通过 `PYTHONPATH` 加载，不加入产品依赖。`python -m unittest discover -s scripts/tests` 包含成功/错误结构、缺字段/额外字段/类型错误、UTF-8与转义后49151/49152/49153字节边界。`verify_diagnostics_wasi.py` 对每个实际输出执行同一schema和字节检查，证明绑定当前manifest/schema摘要；打包器拒绝没有该证据或证据过期的输入。

| Capability suffix | 输入 | secondary |
| --- | --- | --- |
| java.threads | jstack/jcmd 线程文本或 Thread.dump_to_file JSON | 不使用 |
| java.dependencies | Maven/Gradle 依赖文本 | 不使用 |
| java.gc | 支持格式的 GC Pause 文本 | 不使用 |
| java.jfr_json | JFR 导出 JSON | 不使用 |
| spring.config | 左侧 JSON/单文档 YAML | 右侧配置 |
| django.migrations | 迁移计划文本 | 可选 SQL 文本，仅分析 |
| django.sql | 查询日志 JSON 数组，sql/durationMs/可选 requestId | 不使用 |
| django.urls | route/name/namespace JSON 数组 | 可选 name/kwargs JSON |
| django.openapi | 左侧 OpenAPI 3.x JSON | 右侧 OpenAPI |
| django.checks | Django check 输出文本 | 不使用 |
| celery.report | 标准 Task 日志文本 | 不使用 |

插件 manifest 使用 schemaVersion 2，要求公共契约 `compute.wasi.v1@=1.0.0-rc.1`、`output.schema.v1@=1.0.0-rc.1` 和 `tool.directory.v1@=1.0.0-preview.1`；不会声明未支持的 diagnostics requiredProfile。WASI 请求/结果继续使用 `zicode.devtools-plugin/1.0.0-rc.1` 外层格式。成功 `data.text` 是内部诊断报告 JSON，报告 schema 提案升为 `2.0.0-rc.1`，包含 `diagnosticsProtocol`、`generatedBy` 与分析数据。该兼容性仅表示外层可以装入通用文本结果，不表示 Studio 已同意诊断报告 schema 或已接受候选。capabilityId 为 `devtools.diagnostics.` 加上表中后缀，commandId 必须明确为 null。逐操作输入/报告 schema 在 `contracts/diagnostics/v2/operations`。

Spring Actuator 另有一个未注册的 Host 服务 View 预览文件；它只调用 Studio 提案的 `window.zicode.services.http.get`，没有直接网络回退。profile `service.http.get.v1@1.0.0-preview.1` 尚未实现，且其端点/认证/响应预算比桌面工具范围更窄，所以这个预览不属于 0.1.4 manifest 或候选包。具体兼容差异、schema、正反例和 Host 尚未验证的边界见 [`docs/diagnostics-plugin-coverage.md`](../../docs/diagnostics-plugin-coverage.md) 与 `contracts/studio-host-capabilities/service-http-get-preview-20261015/`。

每份成功报告的 `generatedBy` 记录源工具 ID、当前独立工具版本，以及便携运行环境指纹。指纹描述固定 `wasi` / `wasm32` / `plugin` 目标配置和程序版本，不采集实际 Host 的系统/设备信息；SHA-256 只是这些字段的校验摘要，不是模块签名或真实性证明。原生/WASI字节一致性验证使用同一候选 profile 指纹。

请求和完整序列化结果各最多48 KiB，两个输入各最多8192 UTF-8字节；sceneId 1–128 UTF-8字节。Schema 的 maxLength 按字符计，运行时字节限制另行执行。拒绝外层重复/未知字段、缺失处理器槽位和无效 UTF-8。报告内部沿用独立版解析规则，不能据此宣称所有内部 JSON 都有严格重复键/安全整数检查。原生独立版保留较大的报告预算。

结果超限返回 OUTPUT_TOO_LARGE，分析失败返回 INVALID_REPORT，不泄露解析器错误或输入内容。这两个错误码是本诊断提案的扩展，**不在既有文本契约错误枚举内**；Host 必须确认诊断 schema/profile 后再接入，不能直接按既有文本契约验收。运行时只导入 WASI Preview 1 标准接口，验证 runner 不提供目录、环境变量或启动参数；Node 功能验证不代表生产沙箱或 fuel 计量验收。

```powershell
cargo build -p zi-diagnostics-wasi
cargo build -p zi-diagnostics-wasi --profile plugin --target wasm32-wasip1 --config 'target.wasm32-wasip1.rustflags=["-C","link-arg=--max-memory=67108864","-C","link-arg=-zstack-size=2097152"]'
python scripts/verify_diagnostics_wasi.py --native target/debug/zi-diagnostics-wasi.exe --wasm target/wasm32-wasip1/plugin/zi-diagnostics-wasi.wasm --output <archive>/runtime-proof.json
python scripts/sync_diagnostics_plugin.py --check
python scripts/package_diagnostics_plugin.py --wasm target/wasm32-wasip1/plugin/zi-diagnostics-wasi.wasm --proof <archive>/runtime-proof.json --view-proof <archive>/view-qa.json --fuel-proof <archive>/budget-fixtures --serializer-proof <archive>/serializer-fixtures --serializer-module target/wasm32-wasip1/plugin/examples/output_boundary.wasm --cold-proof <archive>/cold-proof.json --output release/<new-candidate>
```

候选的完整复核还需要独立 Wasmtime C API 36.0.2 运行时和浏览器视图验收。首次准备本机证据时执行：

```powershell
python scripts/bootstrap_diagnostics_engine.py --cache-dir target/diagnostics-verification-python36 --provenance target/diagnostics-v013/engine-provenance.json
$env:PYTHONPATH='target/diagnostics-verification-python36'
python scripts/verify_diagnostics_fuel.py --wasm target/wasm32-wasip1/plugin/zi-diagnostics-wasi.wasm --native target/debug/zi-diagnostics-wasi.exe --output target/diagnostics-v013/budget
python scripts/verify_diagnostics_fuel.py --serializer-boundary --wasm target/wasm32-wasip1/plugin/examples/output_boundary.wasm --output target/diagnostics-v013/serializer --engine-provenance target/diagnostics-v013/engine-provenance.json
python scripts/verify_diagnostics_cold.py --wasm target/wasm32-wasip1/plugin/zi-diagnostics-wasi.wasm --engine-provenance target/diagnostics-v013/engine-provenance.json --fixtures target/diagnostics-v013/budget --output target/diagnostics-v013/cold-proof.json
python scripts/verify_diagnostics_view.py --wasm target/wasm32-wasip1/plugin/zi-diagnostics-wasi.wasm --engine-provenance target/diagnostics-v013/engine-provenance.json --output target/diagnostics-v013/view-proof.json
```

View 验收使用 Edge/Chrome DevTools 固定页面视口和亮/暗主题，通过 loopback-only 测试桥调用实际 WASI 模块；包内保留两张视口截图、SHA-256、DPR 和空页面错误列表。它不连接 Studio，也不构成 Host 接入验收。

验证器覆盖全部11能力的独立结果断言、脱敏、错误报告、缺失/重复/未知字段、未知能力、请求/UTF-8输入预算及真实结果膨胀；当前258组具有唯一标识的相同请求的原生/WASI逐字节比较，读取实际内存声明并检查64 MiB上限，记录模块/原生程序及每组请求/结果摘要。

View 使用已有公开 window.zicode.ready/context.get/capabilities.invoke/setDirty 接口，不添加私有桥接。Java/Django/Celery 分类，按工具保留内存草稿/结果；左右配置/OpenAPI等显示补充输入，字节超限禁止调用，Ctrl+Enter分析，结果手动选中复制。修改输入将结果标为旧结果，失败保留输入/结果，过期响应不覆盖新输入，8秒页面等待超时不宣称取消Host任务；可选Error.code固定中文映射，不显示原始错误消息；fuel耗尽提示缩小范围或更新算法，避免盲目重试。关闭后不恢复报告；没有共享独立版用户数据。

候选目录仅用于开发交接；不能要求用户绕过宿主签名校验。包外 verification.json记录准确文件摘要、运行/View证据及未验收范围；packageSha256保持null，不以目录摘要冒充签名容器摘要。可选Pi工具注册仍需显式授权。正式签名、安装、升级/回滚、撤权、卸载和真实Host/Pi链等待接收方按公开诊断profile验收。

0.1.1曾使用 Unicode 属性扫描降低冷启动 regex NFA 编译开销，JFR duration 不再逐事件编译正则，Spring JSON 配置使用快速 JSON 路径并保留 YAML fallback 与既有重复键处理规则。独立程序仍支持较大报告；插件按实际 pretty JSON 及外层字符串转义计算结果预算，超过预算返回完整错误。

早期0.1.1预算验证使用官方 wasmtime Python 49.0.0（仅开发验证依赖，不是桌面/插件运行依赖），每请求新 Store：10M fuel、5s epoch中断、64MiB、2MiB guest stack。Fuel计量因Wasmtime版本不同，不用此结果代替Studio Host验收。`scripts/verify_diagnostics_fuel.py` 提供可移植请求、期望结果和SHA摘要；`output_boundary` 是只验证共享序列化器的独立 example，不进入插件 runtime 或注册能力。它验证ASCII、UTF8和转义数据的49151/49152/49153字节结果。

本包不声明未支持的 diagnostics requiredProfile；diagnostic report schema 2.0.0-rc.1 仍是提供方提案。完整工具迁移仍154项、Host accepted=0；11项仅为导入报告子能力。不得用源目录、独立EXE或本地HTTP代替Host接口。

逐能力实现范围和缺口见docs/diagnostics-plugin-coverage.md；交付目录内提供contracts/coverage.md。

交付目录可不依赖本仓库重跑（独立开发环境安装verification/requirements.txt）：
```
python verification/verify_diagnostics_fuel.py --wasm runtime/tools.wasm --fixtures fixtures/budget --output <new-local-proof-directory>
python verification/verify_diagnostics_fuel.py --serializer-boundary --wasm verification/serializer.wasm --fixtures fixtures/serializer --output <new-local-serializer-proof-directory>
```
serializer.wasm仅验证函数，不是插件runtime；不要注册为能力。交付请求/期望文件逐个记录SHA256，重跑器核对后实际执行新Store。

0.1.2进一步移除运行时Regex依赖；线程状态/锁/omitted捕获与旧正则进行差分测试，Unicode Nd由锁定regex-syntax在构建时生成、全部有效Unicode码点对比。标准release保持thin LTO；独立plugin采用opt-level3、fat LTO、panic abort，避免单纯opt-z缩小体积导致fuel增长。

独立验证环境：Python绑定36.0.0 + 官方C-API36.0.2 DLL（严格摘要）；仅依赖requirements.txt会使用36.0.0引擎，不能冒称36.0.2。需要精确36.0.2时使用verification/bootstrap_diagnostics_engine.py在自有target缓存安装，并在新进程设置PYTHONPATH，再向验证器传入--engine-provenance。DLL与第三方ZIP不进入产品运行依赖；下载ZIP仅内存消费，不保留ZIP。冷验证使用每向量新Python进程，计量setup/compile/instantiate/execute与完整外部5秒时间，无提前编译缓存；这不等于Studio完整EXE/Worker请求重建方式。

Studio candidate02已验证11基本View调用，22墙钟边界仍失败、46不能原样传递用例仍skipped。提供方独立全部188raw向量不能将这些skipped或失败改称Studio通过。154完整工具接入accepted仍0。

当前打包要求功能、fuel、冷启动三套证明的用例集合及请求/结果摘要一致，拒绝重名用例、不完整重放与诊断计时模式。历史188项结果仅表示当时版本；当前新增样本与边界必须重新运行。

View增加11类结果概览、来源版本、截断提示、旧结果标记与“查看结果”导航；完整JSON及范围说明仍可展开。冻结candidate-01保留旧界面；各候选的实际界面以包内View及view-proof.json的摘要为准。源码迭代记录：docs/iterations/2026-10-18-diagnostics-result-overview.md。
