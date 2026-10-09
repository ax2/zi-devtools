# Studio 插件提供方交接

## 当前开发：JSON完整共享核心与查询多字段提案

[JSON核心说明](json-plugin-design.md)：完整JSONPath子集和有序/无序差异提取zi-json-core，独立版复用；原1MiB输入、4096字节查询、128步、10000项、8MiB输出及重复键/宽整数/Pointer语义保留。修正usize造成的大索引跨平台差异，实际native64/WASI32固定fixture10条独立预期一致，5组核心兼容边界测试通过；这是算法fixture，不是通用WASI适配或Host协议执行。原生完整构建仍在进行，根dev105，两工具1.0.3，均not_delivered。

[多字段提案0.1.1](../proposals/studio-fields/v0.1.1/README.md)新增json.path(text/query)，连同前四项共五操作，明确1.1.0-proposal.2；98请求/17结果结构与UTF-8预算验证通过，前版68/17仍通过，旧目录与冻结rc.1不变。Host尚未协商接受，regex/text.diff完整运行及500ms计时接口问题仍待解决；不把结构验证推广为运行/Host/Pi通过。

## 当前交付：Java / Django 堆栈候选

[完整交付说明](trace-plugin-handoff.md)：两项完整文本堆栈操作共用 zi-trace-core，源工具 1.0.3，根 dev105 保持。unsigned `release/trace-0.1.0-candidate-01` 已冻结，346 文件共 1,403,946 字节；最终模块 241,191 字节。59 条实际字节对照、Wasmtime 36.0.2 预算及全新进程冷启动全部通过，最慢 504.374ms，所有验证子进程关闭。最终 Schema、两主题公开 SDK + 实际 WASI View 检查通过；完整工作区测试 829 通过、0 失败、34 忽略，严格 Clippy 和原生优化构建/隔离服务启停通过。两项映射更新为提供方 candidate，实际 Host 安装/生命周期/Worker/Pi 未验收，完整接受仍为 0。

[主 CI 37928511885](https://github.com/ax2/zi-devtools/actions/runs/37928511885) 对 c431fe9 已完成并通过。更正早期记录：37889396659 是增量更新工作流，不是主 CI；先前三次主 CI 被同一输出边界测试假设拦住，现已修复并通过，原失败证据保留。

接收方另报旧inspect01的实际Pi复查yaml.from_json超时，仍需分段定位，暂不能记Pi通过；此前View/7样例及84可表达向量报告不覆盖Pi或10 raw输入。接收方补充transforms共用Pi harness也出现url.decode超时，双方Pi当前均未通过，不能将问题仅归因Inspect模块。没有据此重建旧包或扩大预算。

## 当前开发：颜色、页面接力与多字段提案

格式检查组件 0.1.1 新增完整颜色转换，共 5 工具/8 操作；121 条实际原生/WASI对照及独立预算、两主题页面颜色预览与跨操作接力验证通过。接力明确预览/确认填入，不自动执行，取消保留工作。新版尚未冻结：三轮完整独立冷启动各 121 条，依次有 6、3、2 条外层超时；第三轮开始时未检测到编译进程，仍不能满足五秒门禁。全部验证子进程均已关闭，三轮原始失败均保留。825 项工作区测试、严格 Clippy 及源码 f11c5d0 的 GitHub CI 37886844979 通过。优化桌面构建完成（14m52s），新 EXE 隔离服务启停通过，端口已释放；这不是原生 UI 验收。包生成器新增精确冷证据绑定和可重复历史参数，6 项针对篡改/漏项风险的测试通过；不能用通过标志掩盖结果错误或五秒期限。旧 `inspect-0.1.0-candidate-01` 全部 493 文件再次复核保持不变，不要求接收方重建旧包。

启动诊断工具 `scripts/profile_wasi_startup.py` 对三轮失败用例并集及一条基线进行分段观测，共 12 条，绑定同一模块摘要并保持五秒外层期限。两条较慢样本的 `wasmtime._ffi` 导入自耗时约 2.05/2.21 秒，模块执行仅约 1.73/2.07 毫秒；另一慢样本执行约 38.77 毫秒，尚有进程启动/退出等未归属开销。导入跟踪本身增加开销，诊断样本成功不能覆盖完整复查失败，更不能作为 Host 冷启动通过证据。不通过反复重跑筛选成功结果，不放宽门禁，不因该诊断重建模块。

[多字段提案](../proposals/studio-fields/v0.1.0/README.md)独立于冻结契约：正则 text/pattern、文本与 JSON 差异 left/right，JSON 两模式用两个能力显式选择。68 请求和17结果仅验证结构/字节，不代表算法/WASI/Host已支持。文本diff现有500ms单调计时必须先明确 Host 能力或共同版本化规则，不能暗中加入时钟或用fuel假装语义相同。全154项范围继续保留，完整接受为0。

## 当前增量：2026-10-09 格式与检查候选

[独立交付说明](inspect-plugin-handoff.md)：4 工具、7 操作共享 Rust 核心，94 条实际字节对照、预算与冷启动通过。unsigned 目录 `release/inspect-0.1.0-candidate-01` 已冻结，493 文件逐件摘要复核；不覆盖现有 text、diagnostics、transforms 候选，不将提供方验证当作 Host 接受。全目录仍为 154 项，完整接受为 0。

## 当前增量：2026-10-09 文本转换分组候选

提供方算法源码 `66a83bf797a87e13532f49b0d61e4569dbcf28a6`，请求 schema 绑定修订 `255c967f0d45892bcd1f509f10fe3d19798907ab`。本地桌面版保持 `0.82.0-dev.105`，不因每轮开发递增版本或创建正式 Release。新增 `com.zicode.devtools.transforms/0.1.0`，与原文本和诊断候选并存，不覆盖旧目录。

- 候选目录：`E:\zi-devtools\release\transforms-0.1.0-candidate-02`，798 件文件、3,920,494 字节；外部同名 `-verification.json`。无签名目录，`packageSha256=null`，不是安装容器。
- 模块：549,303 字节，SHA-256 `de8c6e2429c0abd282c4e339951e8fcece4c7ddb5e393983a9de7f3bb66f15bb`；目录索引 SHA-256 `53474d18d37accfc9a5747979bb737ec023a9da34b6bf8644c3bbfb8d21c6de4`。
- 9 个独立版工具的 23 个全部既有转换操作，在输入/输出预算内迁移：URL、HTML、JSON 字符串、命名转换、统计、行处理、四进制、Hex、URL 拆解。逐操作范围与语义见 [候选说明](../plugins/transforms/README.md)，不能将此覆盖推广到整个 154 项目录。
- candidate01 的随包请求 schema 遗留旧试点插件 ID，已记录为被替代候选并保留原目录。candidate02 的请求 schema 只特化 `pluginId.const`，原冻结快照保持原摘要。真实 Draft202012Validator 验证 23 项目录、174 个结果和 67 个成功请求/输入通过；所有 798 文件摘要/大小核对一致。模块与 View 字节和 candidate01 完全相同，原始算法/界面测试仍对应同一摘要；无需因本次元数据修订重复冷编译模块。
- 注册表生成清单、catalog、View 和 23 个 Pi 声明；`sync_transforms_plugin.py --check` 校验同步。使用一个活动入口，按组搜索，保留各操作草稿/结果、标记旧结果、忽略迟到响应，支持显式结果复用。
- 174 个独立预期向量通过真实 native/Node-WASI 逐字节一致；旧文本插件 36 个用例保持一致。174 项独立官方 C-API Wasmtime 36.0.2 预算全部通过，最大 fuel 8,966,602、执行 4 ms、内存 2,621,440 字节。174 项全新独立进程冷启动全部通过，最慢 3,704.001 ms，剩余验证进程为 0。无预算扩张。
- View 使用公共 SDK 注入和实际模块完成 23 操作、1180 浅色/390 深色、搜索、超限、错误/旧结果保留、清空、复用、选中复制、光标保持和离线验证；4 张渲染截图已检查。这不是实际 Studio 桥接、权限、场景、Pi 或安装生命周期验收。
- 外层及错误枚举保持冻结 rc.1；`input.text` 最多 8192 UTF-8 字节，完整请求/序列化结果最多 48 KiB。模块 ≤2 MiB、10M fuel/5秒/64MiB/2MiB stack。场景 ID 在新候选中限定 1–128 字节；原文本包行为不变。
- `fixtures/parity/`、`fixtures/budget/` 含逐条原始请求、预期结果和 SHA-256。独立预算重放使用附带 `verification/verify_diagnostics_fuel.py --text-results`（该开关保持完整字节比较，只跳过诊断报告 JSON 的二次解析），并绑定模块和官方引擎摘要；冷启动可用附带 `verify_diagnostics_cold.py`。独立验证工具不是产品运行依赖。
- 独立桌面验证：完整 28 组工作区测试 818 通过、0 失败、34 原有跳过；fmt 和全特性严格 clippy 通过；复用原 release 目标目录优化构建 10m54s。新 `target/release/ZiDevTools.exe` 的隔离服务启停通过、健康响应正确、退出码0且端口释放。没有新增阶段完整运行目录、安装包、ZIP、tag 或正式 Release；本轮没有把 SDK fixture 当作原生 GUI 验收。
- 当前 [完整映射](studio-plugin-migration.json)包含 154 工具：41 纯计算候选、50 等 Host 接口、63 独立版未完成；重叠整合仍为 0，`acceptedCapabilities=0`。9 工具各操作已记为提供方 candidate；生产 Worker 冷启动/管道隔离、安装/更新/回滚/撤权/卸载和 Pi 授权需接收方实际验证。

后续优先继续 YAML/JSON、IPv4 CIDR、JWT 检视、Unicode 的既有纯计算操作；双输入 JSON/文本差异、查询参数及数据工作台需要逐操作列出参数与完整范围后确定有界协议。图片/文件/桌面/服务能力保持明确 Host 接口需求，不以独立 EXE、共享数据库或本地 HTTP 绕过。以下为历史记录，数量和状态以本节及生成映射为准。

2026-10-07：接收到 Studio 会话的插件交付契约和优先接续要求。流程结果检查器检查点已完成；本轮已推进五项文本能力实际候选与完整目录迁移映射。

契约：`zicode.devtools-plugin/1.0.0-rc.1`。已一次性复制并逐项核对10件文件，连同索引共11件，位于 `contracts/studio-devtools/v1/`。索引SHA-256：`dd0688c009fa2d0600ab2d5af038a19efb5e1e26910619c1aede84aa668873f9`。此后构建/CI只使用本仓库快照，不读取Studio仓库，不引入跨仓path依赖、EXE启动或共享可变用户数据库。

试点：`com.zicode.devtools.text`，JSON格式化/压缩、Base64编解码、SHA-256五项。提取同一纯Rust核心用于独立egui和WASI command适配；View自包含HTML，按协议输入/结果和16条正反向量验证。compute.wasi.v1：2MiB模块、64MiB内存/10M fuel/5秒限制，文本8192 UTF-8字节，请求/结果48KiB。不把限额或静态样例视为实际Host验收。

交付状态：尚无插件产物或签名。无正式私钥时只交unsigned候选，不借用Studio密钥，不擅自发布。将输出包外verification.json、目录/模块大小和摘要及规范对照；包内不含自身包摘要。Host真实安装/UI/Pi Broker和proc_exit(0)/管道排空由接收方验收，提供方记录实际遇到的问题。resource/service/desktop profile待Host接口，不以路径直读/本地HTTP绕过。

全部 tools.json 条目都将记录可迁移、重叠能力整合、等Host接口或独立版未完成；不把独立版implemented当成插件完成。保持独立EXE及阶段档案，复用现有Cargo缓存，不增设重复target，不生成额外ZIP。

## 全目录首轮映射

[迁移映射](studio-plugin-migration.json)覆盖当前全部152项：32项纯计算迁移候选、58项等待Host接口、62项独立版整体未完成。重叠整合暂为0项，尚未确认Host已有同能力，避免推定。五项试点对应json/base64/sha256三项独立工具，全部pluginStatus仍not_delivered，acceptedCapabilities=0。脚本从tools.json同步工具版本/状态，CI执行 --check 防止漏项或陈旧映射。静态候选不代表整个界面/文件功能可直接迁移，需逐项抽核心和限定输入协议。

快照核验脚本：`python scripts/verify_studio_contract.py`，只依赖本仓库。`.gitattributes`禁用该快照的换行转换，以保存发布方原始摘要。16向量和5 capability数量已核对，初次核对时尚未构建实际WASI、缺少wasm32-wasip1且未发现wasmtime CLI；后续已复用共享工具链补齐target并构建实际模块，见下方更新。

## 2026-10-07：文本核心与实际模块

wasm32-wasip1现已装入既有工具链，仍复用D盘Cargo缓存和唯一共享target。实现crates/zi-text-core与crates/zi-text-wasi；根工作区default-members保持独立程序。独立JSON/Base64/SHA-256默认走同一算法，另有显式插件兼容模式执行相同有界请求。严格校验缺失/重复/额外字段、唯一处理器、UTF-8文本/请求/结果字节、JSON重复键/64层/安全整数和Base64 padding。

已构建实际WASI模块并通过36组原生/WASI逐字节对照，包括16条契约向量和接近48KiB的结果；已检查模块声明64MiB最大内存。Node只用于本项目功能验证，不是生产沙箱，也不验收fuel。stdout为单一结果，正常proc_exit行为记录在包外verification中，Host仍需正确处理成功退出与并发管道排空。

View位于plugins/text/views/main.html，自包含，无外部CSS/JS/图片。公开SDK测试替身连接实际WASI，桌面1100px浅色/390px窄窗深色通过；空输入、超限、错误、重复点击、Ctrl+Enter、结果重用、输入变化后忽略旧结果及无Host禁用通过。实际Studio桥、安装、撤权、升级/回滚、场景及Pi Broker未验收。无授权正式私钥，交付unsigned目录，不生成伪签名容器。

JSON/Base64/SHA-256源工具版本1.1.0，五项插件候选0.1.0；迁移表三项pluginStatus更新为candidate，acceptedCapabilities仍为0。独立版默认宽整数、重复键旧策略与解码外层空白保留；插件模式用契约严格规则。后续资源/服务/桌面profile仍等待Host接口。

实际最终模块220439字节，SHA-256 abfd0dbe45c064c4f8e40a5945c004694eb356cdf13688f8d7adf6536e64f20a；36组对照最大响应47191字节。当前模块在Node中_start正常返回，exitCode=0，未触发proc_exit；不据此宣称Studio的proc_exit(0)缺口已修复。候选目录计划release/text-plugin-0.1.0-dev57，包外text-plugin-0.1.0-dev57-verification.json，不生成额外ZIP。

## 候选交付完成（独立版源码6b23a71）

候选目录已生成：release/text-plugin-0.1.0-dev57，13件文件，总250054字节；外部release/text-plugin-0.1.0-dev57-verification.json，逐件摘要/大小、源码修订与组件摘要、契约摘要、36条WASI/原生结果和View QA齐全。packageSha256=null，目录摘要不冒充签名包摘要。所有capability/catalog/Pi绑定及源工具1.1.0核对通过。未提供或借用正式私钥；不发布signed容器。

独立GUI/MCP完整运行目录release/stage-82-text-plugin-6b23a71，程序0.82.0-dev.57；673/0/34、严格默认/全特性、8m29s优化构建、实际新EXE隔离服务启停、两主题原生Ctrl+Enter检查通过。Host真实安装、最大输出并发排空/fuel/权限、实际桥/Pi/scenes仍由Studio接收验收。当前模块正常_start返回，未调用proc_exit；不能用本候选推定其他插件的proc_exit(0)处理已修复。

## dev.58 文本便捷性（验证中）

当前源工具版本 JSON/Base64/SHA-256 1.1.1，目录版本 0.82.0-dev.58。原生操作前置、小窗口高度调整、兼容字节反馈与序列化前超限拒绝；View 增加大结果不可整段重用的说明，并实际检查六秒超时恢复。View 使用既有 WASI + SDK 测试替身通过两主题验证；原生最终检查仍在运行。

上一轮 release/text-plugin-0.1.0-dev57 和外部验证文件保留且未覆盖，仍对应源工具1.1.0与固定源码6b23a71。迁移清单 sourceVersion 表示当前源工具版本，candidate 不表示该新版本已重新打包或被 Host 接收；acceptedCapabilities 仍为0。源工具修订、旧候选交付与真实Host验收是分别记录的进展。

## dev.59 空文本结果（验证中）

当前目录0.82.0-dev.59。通用文本结果区增加完成状态，区分未运行/失败与成功空文本；随工具草稿切换保存，失败、清空、兼容模式及启动工具重置。28项共用结果界面的独立版本已同步；JSON/Base64/SHA-256现1.1.2。

原生两主题合成Ctrl+Enter及Base64空输入反馈已检查；模拟点击实际复制按钮并检查CopyText空字符串后拦截，测试不读写真实剪贴板。空结果仍不通过现有文本接力发送；此轮不声明接力/Host空文本支持。旧unsigned候选仍为dev57交付，未覆盖；acceptedCapabilities保持0。
