# ZiDevTools Diagnostics 0.1.1 - command adapter

本仓库的 `zi-diagnostics-core` 同时供原生桌面端和 `zi-diagnostics-wasi` 使用。11 项能力可分析导入报告；不会启动 JVM/Python、运行迁移或 SQL、连接 Broker、读取用户文件或访问网络。提供自包含 View、元数据及 unsigned 候选目录构建脚本，**没有正式签名发行包，也未通过 Studio 接入验收**。

| Capability suffix | 输入 | secondary |
| --- | --- | --- |
| java.threads | jstack/jcmd 平台线程文本 | 不使用 |
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

输入输出沿用版本 `1.0.0-rc.1` 的 command 外层结构，但诊断 input/schema 是本仓库独立的提供方提案，见 `contracts/diagnostics/v1`；不能宣称接收方已采纳。capabilityId 为 `devtools.diagnostics.` 加上表中后缀，commandId 必须明确为 null。stdin 接收一份请求，stdout 输出一份 JSON envelope：contractVersion、ok、data、error。成功 data.text 为完整 JSON 报告字符串。

请求和完整序列化结果各最多48 KiB，两个输入各最多8192 UTF-8字节；sceneId 1–128 UTF-8字节。Schema 的 maxLength 按字符计，运行时字节限制另行执行。拒绝外层重复/未知字段、缺失处理器槽位和无效 UTF-8。报告内部沿用独立版解析规则，不能据此宣称所有内部 JSON 都有严格重复键/安全整数检查。原生独立版保留较大的报告预算。

结果超限返回 OUTPUT_TOO_LARGE，分析失败返回 INVALID_REPORT，不泄露解析器错误或输入内容。这两个错误码是本诊断提案的扩展，**不在既有文本契约错误枚举内**；Host 必须确认诊断 schema/profile 后再接入，不能直接按既有文本契约验收。运行时只导入 WASI Preview 1 标准接口，验证 runner 不提供目录、环境变量或启动参数；Node 功能验证不代表生产沙箱或 fuel 计量验收。

```powershell
cargo build -p zi-diagnostics-wasi
cargo build -p zi-diagnostics-wasi --release --target wasm32-wasip1 --config 'target.wasm32-wasip1.rustflags=["-C","link-arg=--max-memory=67108864","-C","link-arg=-zstack-size=2097152"]'
python scripts/verify_diagnostics_wasi.py --native target/debug/zi-diagnostics-wasi.exe --wasm target/wasm32-wasip1/release/zi-diagnostics-wasi.wasm --output <archive>/runtime-proof.json
python scripts/sync_diagnostics_plugin.py --check
python scripts/package_diagnostics_plugin.py --wasm target/wasm32-wasip1/release/zi-diagnostics-wasi.wasm --proof <archive>/runtime-proof.json --view-proof <archive>/view-qa.json --fuel-proof <archive>/budget-fixtures --serializer-proof <archive>/serializer-fixtures --serializer-module target/wasm32-wasip1/release/examples/output_boundary.wasm --output release/<new-candidate>
```

验证器覆盖全部11能力的独立结果断言、脱敏、错误报告、缺失/重复/未知字段、未知能力、请求/UTF-8输入预算及真实结果膨胀；至少188组相同请求的原生/WASI逐字节比较，读取实际内存声明并检查64 MiB上限，记录模块/原生程序及每组请求/结果摘要。

View 使用已有公开 window.zicode.ready/context.get/capabilities.invoke/setDirty 接口，不添加私有桥接。Java/Django/Celery 分类，按工具保留内存草稿/结果；左右配置/OpenAPI等显示补充输入，字节超限禁止调用，Ctrl+Enter分析，结果手动选中复制。修改输入将结果标为旧结果，失败保留输入/结果，过期响应不覆盖新输入，8秒页面等待超时不宣称取消Host任务；可选Error.code固定中文映射，不显示原始错误消息；fuel耗尽提示缩小范围或更新算法，避免盲目重试。关闭后不恢复报告；没有共享独立版用户数据。

候选目录仅用于开发交接；不能要求用户绕过宿主签名校验。包外 verification.json记录准确文件摘要、运行/View证据及未验收范围；packageSha256保持null，不以目录摘要冒充签名容器摘要。可选Pi工具注册仍需显式授权。正式签名、安装、升级/回滚、撤权、卸载和真实Host/Pi链等待接收方按公开诊断profile验收。

0.1.1 使用 Unicode 属性扫描降低冷启动 regex NFA 编译开销，JFR duration 不再逐事件编译正则，Spring JSON 配置使用快速 JSON 路径并保留 YAML fallback 与既有重复键处理规则。独立程序仍支持较大报告；插件按实际 pretty JSON 及外层字符串转义计算结果预算，超过预算返回完整错误。

独立预算验证使用官方 wasmtime Python 49.0.0（仅开发验证依赖，不是桌面/插件运行依赖），每请求新 Store：10M fuel、5s epoch中断、64MiB、2MiB guest stack。Fuel计量因Wasmtime版本不同，不用此结果代替Studio Host验收。`scripts/verify_diagnostics_fuel.py` 提供可移植请求、期望结果和SHA摘要；`output_boundary` 是只验证共享序列化器的独立 example，不进入插件 runtime 或注册能力。它验证ASCII、UTF8和转义数据的49151/49152/49153字节结果。

本包不声明 diagnostics 或 output.schema.v1 是已采纳的 requiredProfiles；schema仍为提供方1.0.0-rc.1提案。完整工具迁移仍154项、Host accepted=0；11项仅导入报告子能力。不得用源目录、独立EXE或本地HTTP代替Host接口。

逐能力实现范围和缺口见docs/diagnostics-plugin-coverage.md；交付目录内提供contracts/coverage.md。

交付目录可不依赖本仓库重跑（独立开发环境安装verification/requirements.txt）：
```
python verification/verify_diagnostics_fuel.py --wasm runtime/tools.wasm --fixtures fixtures/budget --output <new-local-proof-directory>
python verification/verify_diagnostics_fuel.py --serializer-boundary --wasm verification/serializer.wasm --fixtures fixtures/serializer --output <new-local-serializer-proof-directory>
```
serializer.wasm仅验证函数，不是插件runtime；不要注册为能力。交付请求/期望文件逐个记录SHA256，重跑器核对后实际执行新Store。
