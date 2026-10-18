# Studio 插件提供方交接

## 2026-10-18：全部包收据索引规范化

在上一份不可变目录修订基础上，为此前缺少 Studio 所需结构化索引编码的四个插件收据新增 revision-01，包目录与其逐文件内容均未改动；旧收据和旧目录保留。Inspect 0.1.2 已有的 revision-01 收据经 Studio 校验器复核后直接复用。迁移映射现为 6/6 个包收据目录索引全部 `matched`。

最终不可变目录修订为 `release/plugin-candidates/devtools-catalog-2026-10-18-d7d61b707a89a366/`，外部收据 `release/plugin-candidates/devtools-catalog-2026-10-18-d7d61b707a89a366-verification.json`；修订 SHA-256 `d7d61b707a89a366a5382c9dca8f253043242eac8c8a873e6931b71ee4207689`，目录索引 SHA-256 `dee6a98d21de3c98d37db81c84928ffc6adec28b4859dbdbea5bf2bc5f1fe8ca`，共 11 文件、2,105,306 字节。它包含更新后的154工具迁移表及六份外部包收据副本；源目录 `tools.json` 仍为 `37353db759bad2df4320990a55b6c5d32b81058e75f3e98fc39646d1afd0499d`，迁移表 SHA-256 为 `613bddd0db2aef56ffd388e7e67bcdaed1085e8c24cc88b66a836922dc3d568d`。Studio 的目录快照 verifier 与六个包 verifier 均退出 0，文件和索引匹配。初次重打包时 Text 工具操作级映射缺收据路径，生成器错误回退到旧收据；现已从工具级候选包映射解析该收据，最终快照内六包状态全为 `matched`。迁移表生成器改为写入 UTF-8 LF 字节，避免 Windows 换行转换给提交差异检查带来伪尾随空格并保持可复制摘要稳定。

四个新增/复用收据：diagnostics SHA-256 `a2989a110f1736fb431b468dc3a376e2fc9344e3de4332dab344cbf2a4a9a8fb`、inspect `29afb75e81cea5e7606ad238708b6bef796809eb1df8ee9ab58e66b1b011c202`、text `f9911bcb4a1e56e627b25605d7ee460e1ce32f082e1758928131256723bd8f02`、transforms `a8d76696f037e0c932952e68e1b620144d8a24ae739889a820859be9c796975f`。修订只增加可验证的收据元数据，不是插件包新版本、签名或运行验收；所有包仍 unsigned，`acceptedCapabilities=0`，Host/Pi/生命周期仍待 Studio 接收验收。完整过程见 [本轮记录](iterations/2026-10-18-studio-receipt-normalization.md)。

## 2026-10-18：不可变目录修订与 JSON Object 收据

不可变目录修订为 `release/plugin-candidates/devtools-catalog-2026-10-18-3c7c5aa8ad679a14/`，外部收据为同名 `-verification.json`。修订 ID 由源目录内容派生，不复用应用开发版本 `0.82.0-dev.105`；包内冻结两个源 JSON、Studio 对比结果、154 项逐工具包/操作/能力/范围映射，以及六个包的副本收据。源目录 SHA-256 分别为 `tools.json` `37353db759bad2df4320990a55b6c5d32b81058e75f3e98fc39646d1afd0499d` 和迁移映射 `8f67d99638646519e846d9391fd0fc7a4336460752e8cb3c4d7192c2e68d7a68`。修订 SHA-256 为 `3c7c5aa8ad679a141b1268654bc9ef085530d33b1e123b40e08c26df80042ea1`，索引 SHA-256 为 `ef23b73d4310d066ca73753830afeed404843d066de9e374f6363552e69fe2c3`；共 11 个文件、2,108,664 字节。

Studio 的 dev.105 对比报告核对到 154 项，新增/删除均为 0，版本或状态变化 76 项、范围变化 70 项、发现信息变化 76 项；当前状态数为 implemented 92、in-progress 35、planned 27。六个包映射均保留各自 scope 与收据。JSON Object 0.1.0 的 candidate-07 原目录、原收据和 37 个包文件均未改动；新增规范化目录索引收据 `release/plugin-candidates/json-object-0.1.0-candidate-07-verification-revision-01.json`，其 SHA-256 为 `ff9909ae8b73d569e02eeb9b5c3c5d28730d9d6e8e46a636712a83ef24816b85`。新索引 SHA-256 为 `9479917a473acd40e1ef1b9f826c286de53ce2adcabbdc15b3bb72718fdcb3c1`；37 个文件、328,768 字节。迁移映射现在指向新收据。

Studio 的通用只读 verifier 分别核对目录修订和 JSON Object 收据，均退出码 0，`directoryIndex.status=matched`。这只证明收据和文件字节匹配；目录仍 unsigned，`acceptedCapabilities=0`，不代表安装、运行时、Pi 或完整工具验收。快照来源工作树当时有未提交改动；本轮不对大范围既有改动做提交或回滚。完整验证与产物摘要见 [本轮档案](iterations/2026-10-18-studio-catalog-revision.md)。

## 当前候选：JSON object result 插件 0.1.0

`release/plugin-candidates/json-object-0.1.0-candidate-07` 声明 vendored `output.schema.v1@1.0.0-rc.1` 和 `result.json.v1@1.0.0-preview.1`，提供 JSON 对象摘要和对象接收能力。两个能力都声明 whole-output JSON Schema 并显式启用 `application/json` capture；Schema 覆盖冻结的完整 `contractVersion/ok/data/error` 输出，receiver 的 `value` 字段接受 object 类型。只有摘要能力注册为受 `pi.tools.register` 用户授权保护的 Pi 工具；接收能力不注册给 agent，只供 Host 在用户确认接力后调用。源 JSON 工具版本 `1.3.0` 增加对象摘要，独立程序版本保持 `0.82.0-dev.105`。

候选共 37 文件、328,768 字节；原收据目录索引 SHA-256 `51e2e1c9892ef6aff9ef848735007d0bf1cefd7fbb5ada2e9020952458246f17`。原目录、原收据和包内容保留。因 Studio verifier 规范化索引采用 `path,size,sha256` 顺序，现另附修订收据 `release/plugin-candidates/json-object-0.1.0-candidate-07-verification-revision-01.json`，规范化索引 SHA-256 `9479917a473acd40e1ef1b9f826c286de53ce2adcabbdc15b3bb72718fdcb3c1`；新收据 SHA-256 `ff9909ae8b73d569e02eeb9b5c3c5d28730d9d6e8e46a636712a83ef24816b85`。10 条 actual native/WASI 原始请求逐字节一致，重复键、非对象、额外字段和输入边界失败用例通过；成功/失败均保留完整四字段外壳。Wasmtime 36.0.2 下 10/10 燃料用例通过，最高 563,526/10,000,000；10/10 全新进程冷启动通过，最慢 475.680 ms。已从候选目录调用所附 replay script，10 条 WASI 请求全部与预期字节匹配。

候选未签名、未获 Studio Host/Pi 接受。结果捕获、对象接力的搜索/预览/确认、JSON 保存/回读、权限撤销和真实 Host 生命周期仍需接收方验收；`acceptedCapabilities` 保持 0。Studio Host 当前对 receiver input schema 的 `maxProperties` 关键字不兼容，Studio 正在补通用 schema 支持；保留提供方的有界限制，待 Host 支持后重跑验收。Candidate-01 是修复 vendored source metadata 时留下的无收据不完整打包尝试，不应安装；Candidate-02 至 Candidate-06 是较早完整包。本次交接使用 Candidate-07。

## 当前候选：文本插件 0.4.1（目录扩展与单次解析）

`release/text-plugin-0.4.1-candidate-02` 要求 `tool.directory.v1@1.0.0-preview.1`，七个稳定 capability ID 逐项声明分类、真实用途和中英文检索别名；计算输入/输出契约没有变化。包内包含 `verification/tool-directory-snapshot.json` 和 `MIGRATION.md`，包外收据 `release/text-plugin-0.4.1-candidate-02-verification.json` 列出逐文件 SHA-256、大小、目录索引摘要及 profile/schema/metadata 摘要。

0.4.1 把插件兼容 JSON 解析从 RawValue 多次递归扫描改为单次 Visitor 遍历；重复键、64层深度及安全整数规则保持。模块 745,814 字节，SHA-256 `7cfcde63aaaa68353067ca1f7a99c8c6e4a6926166f541a6de03c7c814803767`。43 条 native/WASI 字节对照、View 两主题七能力检查、官方 Wasmtime 36.0.2 的 43 条10M fuel/64MiB预算和43个独立进程冷启动全部通过；最大 fuel 4,099,756，最慢冷启动681.396ms，所有子进程已关闭。包共112文件、1,032,175字节，目录索引 SHA-256 `45e2ec33d6c93a82bec8a97295c9adf191577da78f27d95e3996c58ca10aa358`。旧 0.4.0、0.3.0及其收据保留，不覆盖。

包为 unsigned 目录，不是 `.zicode-plugin`，未由 Studio 安装或验收。Wasmtime测量不是Studio生产Worker、管道排空或Host计时验收。无获准签名密钥；Studio Host 的筛选、执行、Pi 授权、更新/撤权/卸载和真实安装截图仍待验收，`acceptedCapabilities` 不增加。

Studio 的 profile/schema 快照和17项能力目录映射位于 `contracts/studio-tool-directory/1.0.0-preview.1/`。生成器拒绝缺项、额外项、非法分类、控制字符、超出 UTF-8 字节预算及 NFKC/小写/去空白后的别名碰撞。Inspect `0.1.1-candidate-02` 与 Trace `0.1.1-candidate-01` 已重新冻结该目录 profile；旧候选保留，实际 Studio Host/Pi/安装验收仍未完成。

## 当前交付：格式与检查插件 0.1.1

候选目录 `release/plugin-candidates/inspect-0.1.1-candidate-02`，包外收据 `release/plugin-candidates/inspect-0.1.1-candidate-02-verification.json`。共 612 个文件、2,343,823 字节；模块 623,155 字节，SHA-256 `1db3fc37df01cd3a8371320d4f897a7fdf750c87e6202b736ed6870f046b8f29`；目录索引 SHA-256 `62de924883f6f0b246ee74fb34eea530b540e6ad7ccd40e5f1018ad830b8d391`。包内包含目录 profile 快照及 schema/别名来源。

覆盖颜色、YAML/JSON、IPv4 CIDR、JWT 内容和 Unicode 共 5 个工具、8 个既有操作，输入上限 8192 UTF-8 字节、序列化结果 48 KiB。121 条独立预期用例的 native/WASI 字节对照、Wasmtime 36.0.2 的 10M fuel/64MiB 内存预算、全新进程 5 秒冷启动和 JSON Schema 检查均通过；最大燃料 6,918,538，最大冷启动 442.297ms。两主题浏览器 View 实际调用 WASI 并验证全部 8 个操作及接力预览/取消；截图已检查。首次预算验证漏传 `--text-results` 时，通用报告解析器把文本结果当 JSON 解析而中断；按工具原有边界模式重跑后 121 项全部通过。

候选为 unsigned 提供方目录，不是 `.zicode-plugin` 安装包。Studio Host 安装、生产 Worker 隔离、Pi 授权、更新/撤权/卸载仍待接收方验收；全目录 `acceptedCapabilities=0`。

## 当前交付：格式与检查插件 0.1.2（tool.open.v1）

新版候选目录 `release/plugin-candidates/inspect-0.1.2-candidate-01/`，包外逐文件收据为同名 `-verification.json`。该包在 schemaVersion 2 清单中精确要求 `tool.open.v1@=1.0.0-preview.1`，保留 `tool.directory.v1@=1.0.0-preview.1`，提供单一 `location: main` View 和原有 8 个能力。旧 `inspect-0.1.1-candidate-02` 保持原样。新版的 WASM 与 0.1.1 字节一致；本候选只改包清单/元数据、主 View 和验证资料，不重复改算法模块。

`window.zicode.navigation.onOpenTool(({capabilityId}) => …)` 用精确 capability ID 映射对应操作选择框，并把焦点放回输入；不调用 `capabilities.invoke`、不改权限、不读写文件或启动服务。有效 ID 返回 `opened`，未知 ID 返回 `not_found`，内部 UI 不可用返回 `unsupported`。包外 verifier 在实际浏览器执行三类定位样例，并在亮/暗主题截图。慢回调样例用 4.5 秒延迟验证 4 秒 View 预算返回 `unsupported`，迟到 ACK 不覆盖已超时的状态；该延迟只在验收 fixture 中，不在产品 View 内。

能力到界面操作采用精确映射，不根据标题或关键词猜测：

| Capability ID | 主 View 操作 | 定位结果 |
| --- | --- | --- |
| `devtools.inspect.color.convert` | `color.convert` | 颜色转换 |
| `devtools.inspect.yaml.to_json` | `yaml.to_json` | YAML 转 JSON |
| `devtools.inspect.yaml.from_json` | `yaml.from_json` | JSON 转 YAML |
| `devtools.inspect.cidr.inspect` | `cidr.inspect` | CIDR / IP 网段 |
| `devtools.inspect.jwt.inspect` | `jwt.inspect` | JWT 检查 |
| `devtools.inspect.unicode.inspect` | `unicode.inspect` | Unicode 检查 |
| `devtools.inspect.unicode.nfc` | `unicode.nfc` | Unicode NFC 规范化 |
| `devtools.inspect.unicode.nfkc` | `unicode.nfkc` | Unicode NFKC 规范化 |

每个样例都断言 `capabilityInvocations=0`；定位只选择界面并聚焦输入，用户仍需检查输入并主动运行操作。

本地 provider receipt 通过并不等于 Studio 接受。Studio 仍需验证 Host 当前通道/请求 ID、4/5 秒预算、迟到回执、View 销毁、撤权、卸载、跨工作区/场景切换和原生 UI；目录完整接受数保持 0。

## 当前交付：Java / Django 堆栈插件 0.1.1

候选目录 `release/plugin-candidates/trace-0.1.1-candidate-01`，包外收据同名 `-verification.json`。共 352 个文件、1,428,569 字节；模块 SHA-256 `bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e`；目录索引 SHA-256 `2e8b35249384757bc1c809bf971e2feccfc9396211a4efde7ca4f29fda9820c0`。本修订将 `tool.directory.v1@1.0.0-preview.1` 与逐能力分类/中英文别名纳入清单，重新验证 59 条结果 Schema；运行模块、View 和既有 59 条预算/冷启动证据均与收据绑定，fuel 上限使用 7,932,681/10,000,000，冷启动 59/59 通过。旧 `trace-0.1.0-candidate-01` 保持不变。此候选未签名、未获 Host/Pi 接受。

## 当前交付：文本插件 0.3.0（SQL 部分覆盖）

候选目录 `release/text-plugin-0.3.0-candidate-03`，包含 JSON 格式/压缩/校验、Base64 编解码、SHA-256 和 SQL 通用格式化。SQL 复用共享 `sqlformat` Rust 核心，但只暴露通用方言默认格式，不含桌面版方言、缩进和关键字大小写选项。43 条 native/WASI 字节对照及两主题 View 对 JSON 格式/压缩/校验、SQL、Base64 编解码和 SHA-256 七个操作逐项检查通过。候选未签名、Studio Host/Pi 未验收。

## 历史交付：JSON完整范围候选 0.2.0

JSON独立工具版本1.2.0包含格式化、压缩、校验三操作，映射到独立能力 `devtools.text.json.format`、`devtools.text.json.minify` 与 `devtools.text.json.validate`。候选使用已冻结的 `compute.wasi.v1@=1.0.0-rc.1`，最多8192 UTF-8字节，结果最多48 KiB；不声明任务生命周期、文件、网络或资源profile。插件兼容核心拒绝重复键、超过64层和不安全整数，Standalone默认行为不变。

候选目录为 `release/text-plugin-0.2.0-candidate-04`，包ID `com.zicode.devtools.text`。旧 `0.1.0` 与早期 `0.2.0-candidate-01/02/03` 均保留且不覆盖。无签名私钥时仅生成unsigned目录及包外收据，记录文件清单/大小/SHA-256和验证结果。Studio安装、授权、PI注册、更新/回滚/撤权/卸载仍需接收方验收，`acceptedCapabilities` 不自动增加。

## 当前开发：JSON完整共享核心与查询多字段提案

[JSON核心说明](json-plugin-design.md)：完整JSONPath子集和有序/无序差异提取zi-json-core，独立版复用；原1MiB输入、4096字节查询、128步、10000项、8MiB输出及重复键/宽整数/Pointer语义保留。修正usize造成的大索引跨平台差异，实际native64/WASI32固定fixture10条独立预期一致，5组核心兼容边界测试通过；这是算法fixture，不是通用WASI适配或Host协议执行。完整JSON核心回归840通过，优化构建11m09s和隔离服务2177ms通过；随后三项JSON实验适配的19条真实native/WASI字节/Schema、独立预算和冷启动通过，工作区844通过。[实验说明](../plugins/fields/README.md)明确无View/包/Host协商，不是五操作提案全实现。根dev105，两工具1.0.3，仍not_delivered。

[多字段提案0.1.1](../proposals/studio-fields/v0.1.1/README.md)新增json.path(text/query)，连同前四项共五操作，明确1.1.0-proposal.2；98请求/17结果结构与UTF-8预算验证通过，前版68/17仍通过，旧目录与冻结rc.1不变。Host尚未协商接受，regex/text.diff完整运行及500ms计时接口问题仍待解决；不把结构验证推广为运行/Host/Pi通过。

## 当前交付：Java / Django 堆栈候选

[完整交付说明](trace-plugin-handoff.md)：两项完整文本堆栈操作共用 zi-trace-core，源工具 1.0.3，根 dev105 保持。当前 unsigned 目录 `release/plugin-candidates/trace-0.1.1-candidate-01` 为 352 文件、1,428,569 字节，包含目录搜索 profile；旧 `release/trace-0.1.0-candidate-01` 保留。59 条实际字节对照、Wasmtime 36.0.2 预算及全新进程冷启动全部通过，最慢 504.374ms，所有验证子进程关闭。最终 Schema、两主题公开 SDK + 实际 WASI View 检查通过；完整工作区测试 829 通过、0 失败、34 忽略，严格 Clippy 和原生优化构建/隔离服务启停通过。两项映射更新为提供方 candidate，实际 Host 安装/生命周期/Worker/Pi 未验收，完整接受仍为 0。

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
- 当前 [完整映射](studio-plugin-migration.json)包含 154 工具：41 纯计算候选、51 等 Host 接口、62 独立版未完成；重叠整合仍为 0，`acceptedCapabilities=0`。32 个工具条目已映射到 6 个具体候选目录及其外部收据；生产 Worker 冷启动/管道隔离、安装/更新/回滚/撤权/卸载和 Pi 授权需接收方实际验证。此计数随 2026-10-18 JSON Object 收据修订更新。

YAML/JSON、IPv4 CIDR、JWT、Unicode 与文本转换的既有候选均已落目录并纳入映射。下一步优先补足多字段 JSON/文本差异适配的完整预算和真实Host协议协商；当前固定结果仍有冷启动超时或参数协议未协商，不能先标记候选交付。图片/文件/桌面/服务能力保持明确 Host 接口需求，不以独立 EXE、共享数据库或本地 HTTP 绕过。以下为历史记录，数量和状态以本节及生成映射为准。

2026-10-07：接收到 Studio 会话的插件交付契约和优先接续要求。流程结果检查器检查点已完成；本轮已推进五项文本能力实际候选与完整目录迁移映射。

契约：`zicode.devtools-plugin/1.0.0-rc.1`。已一次性复制并逐项核对10件文件，连同索引共11件，位于 `contracts/studio-devtools/v1/`。索引SHA-256：`dd0688c009fa2d0600ab2d5af038a19efb5e1e26910619c1aede84aa668873f9`。此后构建/CI只使用本仓库快照，不读取Studio仓库，不引入跨仓path依赖、EXE启动或共享可变用户数据库。

试点：`com.zicode.devtools.text`，JSON格式化/压缩、Base64编解码、SHA-256五项。提取同一纯Rust核心用于独立egui和WASI command适配；View自包含HTML，按协议输入/结果和16条正反向量验证。compute.wasi.v1：2MiB模块、64MiB内存/10M fuel/5秒限制，文本8192 UTF-8字节，请求/结果48KiB。不把限额或静态样例视为实际Host验收。

交付状态：尚无插件产物或签名。无正式私钥时只交unsigned候选，不借用Studio密钥，不擅自发布。将输出包外verification.json、目录/模块大小和摘要及规范对照；包内不含自身包摘要。Host真实安装/UI/Pi Broker和proc_exit(0)/管道排空由接收方验收，提供方记录实际遇到的问题。resource/service/desktop profile待Host接口，不以路径直读/本地HTTP绕过。

全部 tools.json 条目都将记录可迁移、重叠能力整合、等Host接口或独立版未完成；不把独立版implemented当成插件完成。保持独立EXE及阶段档案，复用现有Cargo缓存，不增设重复target，不生成额外ZIP。

## 全目录首轮映射

[迁移映射](studio-plugin-migration.json)覆盖当前全部152项：32项纯计算迁移候选、58项等待Host接口、62项独立版整体未完成。重叠整合暂为0项，尚未确认Host已有同能力，避免推定。五项试点对应json/base64/sha256三项独立工具，全部pluginStatus仍not_delivered，acceptedCapabilities=0。脚本从tools.json同步工具版本/状态，CI执行 --check 防止漏项或陈旧映射。静态候选不代表整个界面/文件功能可直接迁移，需逐项抽核心和限定输入协议。

快照核验脚本：`python scripts/verify_studio_contract.py`，只依赖本仓库。`.gitattributes`禁用该快照的换行转换，以保存发布方原始摘要。16向量和5 capability数量已核对，初次核对时尚未构建实际WASI、缺少wasm32-wasip1且未发现wasmtime CLI；后续已复用共享工具链补齐target并构建实际模块，见下方更新。

## 2026-10-17：文本插件候选 0.3.0（SQL 部分覆盖）

当前文本插件候选新增 `devtools.text.sql.format`，复用桌面版的纯 Rust `sqlformat` 核心，View 可切换 SQL/JSON 并通过本地 SDK 形状桥接调用实际 WASI。仅支持通用方言默认布局；桌面端 PostgreSQL/MySQL/SQLite/SQL Server 选择、缩进和关键字大小写设置不在候选范围，因此迁移表将 SQL 标记为部分覆盖。模块验证需固定为 43 条 native/WASI 字节一致用例；两主题 View 覆盖 SQL 格式化及 JSON 正反例。

候选计划路径 `release/text-plugin-0.3.0-candidate-03`，包外收据同名 `-verification.json`。候选未签名，Studio Host/Pi/安装生命周期未验收，`acceptedCapabilities` 保持 0；Node 运行只证明功能一致，不证明 Host fuel 或沙箱安全。上一段记录的 0.2.0 JSON 候选保留为历史交付，不被覆盖。

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
