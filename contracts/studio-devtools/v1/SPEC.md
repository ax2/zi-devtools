# ZiDevTools 独立插件交付规范

日期：2026-10-07。契约标识 `zicode.devtools-plugin/1.0.0-rc.1`。
状态：首批实现契约已固定，双方实包验收待完成；不是“所有工具已接入”的声明。

## 1. 独立构建与职责

ZiDevTools 维护自己的纯 Rust 工具核心、独立 egui 程序和插件构建适配器。
同一核心可由独立程序和 WASI 插件调用，但 Studio 不引用 ZiDevTools 源码、Cargo crate、
EXE、绝对本机路径或配置目录；两仓库之间没有 path dependency、symlink 或共享可变数据库。
Studio 只消费版本化插件包与公开契约。提供方可把契约快照复制进自己的仓库，记录版本和摘要；
编译与 CI 必须在只有自己仓库的机器上完成，不得读取本机 Studio checkout。

Studio 负责通用安装、校验、隔离执行、场景权限、UI 消息桥、Pi Broker、更新/回滚与接收验收。
ZiDevTools 负责工具算法、输入输出、工具版本、独立 UI、插件 View 和生成包。
规范变更通过新版本及迁移说明交接；两边不能私自增加只适用于对方内部代码的快捷入口。

## 2. 能力分层与迁移范围

| Profile | v1 状态 | 范围和边界 |
| --- | --- | --- |
| compute.wasi.v1 | 首批目标，现有 Host 有执行基础 | 文本、编码、JSON 等有界纯计算；无文件、网络、进程、持久状态 |
| resource.v1 | 设计待实施，不可宣称支持 | 由 Host 授权资源句柄，流式读写、预览、冲突及恢复；数据/图片/目录工具 |
| service.v1 | 设计待实施，不可宣称支持 | Host 受控网络、任务、服务、数据库与模型访问；权限、取消和生命周期 |
| desktop.v1 | 设计待实施，不可宣称支持 | 录屏、剪贴板、快捷键等按 OS 单独声明与验证 |

每项 ZiDevTools 目录功能必须映射为可迁移、已有能力整合、等待宿主接口或未实现。
“独立版 implemented”不等于“插件 implemented”。Windows 专有功能不得标记全平台。
所有功能最终有映射，不为赶进度让插件启动独立 EXE、访问任意路径或另起无鉴权 HTTP 服务。
未来 profile 需先定义版本化请求/结果、权限、资源上限和取消契约，再由 Studio 实现通用接口。
当前 WASI stdin/stdout 是一次调用，无法通过声明权限凭空获得 Host 回调或双向 RPC。

## 3. 包、标识与交付物

沿用现有 `plugin.json` schemaVersion 1、Plugin API 1 和 `.zicode-plugin` 签名容器。
首包 ID 为 `com.zicode.devtools.text`，工具命名空间 `devtools.text.*`；独立工具 ID 保留在
交付清单的 sourceToolId，不能把全局原始短 ID（如 json）直接作为跨插件 Capability ID。
其他包按数据、诊断、知识、系统等边界拆分，避免每个工具增加主导航项。

提供方交付：

- 自包含目录：plugin.json、runtime/tools.wasm、views/main.html、LICENSE/NOTICE；
- catalog.json：按本规范的 delivery schema，记录工具 ID、版本、来源和支持状态；
- schemas/*.json 和 fixtures/*.json：输入/输出约束、正反例与预期结果；
- `.zicode-plugin` 正式容器、独立 SHA-256 文件；私钥位于源码和输出目录之外；
- 包外 verification.json：源码 revision、dirty 状态、契约版本/快照摘要、工具链、已做检查、
  目标平台及未验收项，并引用最终包 SHA-256。包内 provenance.json 只记录构建来源，
  不写自身包摘要，避免自引用哈希。开发候选允许 dirty=true，正式交付需固定源码与内容摘要；
- 合法的首次安装、升级、撤权、卸载保留数据说明。文档和 schema 应随签名覆盖。

未配置正式签名密钥时交付候选目录及明确的 unsigned 状态，不能借用 Studio 私钥或伪造正式签名。
`.zicode-plugin` 内部为 ZIP 是现有插件格式；独立程序的 Windows 阶段交付仍保留完整目录和 EXE。
包不得包含 node_modules、target、Pi、Bun、Ollama、模型权重、用户数据库、服务配置或凭据。
总展开内容最多 50 MiB、文件最多 2048、单文件最多 10 MiB；WASI 模块另限 2 MiB。
超过限额先拆包/裁剪依赖，不静默提高 Host 限额。

## 4. 首批 ABI：compute.wasi.v1

Rust 目标为 `wasm32-wasip1`，WASI Preview 1 command 模块导出 `_start`。
每次调用重新实例化，stdin 为一个 UTF-8 JSON 对象并以 EOF 结束，无长度头、无 JSON-RPC：

```json
{"pluginId":"com.zicode.devtools.text","sceneId":"coding","capabilityId":"devtools.text.base64.encode","commandId":null,"input":{"text":"hello"}}
```

由 Host 提供 pluginId、sceneId；capabilityId/commandId 恰好一个非 null。
提供方必须检查处理器是否允许，不能把它当任意代码/命令。首批不注册后台 Command，
UI 通过 Capability 调用；UI 活动入口由既有 Activity/View 提供。
stdout 只写一个 JSON 结果；不要输出启动标语、日志、多行事件或 Host 私有 WorkerResponse 包装：

```json
{"contractVersion":"1.0.0-rc.1","ok":true,"data":{"text":"aGVsbG8="},"error":null}
```

可预期输入错误输出 `ok:false,data:null,error:{code,message}`；code 从
INVALID_INPUT、INPUT_TOO_LARGE、INVALID_ENCODING、UNSUPPORTED_OPERATION、INTERNAL_ERROR 选择。
message 为可展示、不含原始输入/凭据/堆栈的短说明，最多 240 字符。
界面需分别处理插件业务失败与 Host 拒绝/超时导致的 Promise rejection。
未知协议主版本拒绝执行，新增可选字段需要双方兼容测试；破坏性输入/输出变化使用新主版本/Capability ID。

当前 Host 上限：模块 2 MiB，内存 64 MiB，栈 2 MiB，fuel 10,000,000，执行进程 5 秒，
Worker 请求 256 KiB，stdout 1 MiB，stderr 64 KiB；View 消息上限是 64K **字符**。
本 profile 收紧为输入 text UTF-8 最多 8192 字节、序列化请求和响应各最多 48 KiB，
提供方在解析/分配前后检查字节与结构上限。输出上限也适用于错误路径。
无预打开目录、环境变量、网络、宿主路径或跨调用状态；v1 不提供进度事件/长任务/持久化。

Host 当前有两个需由 Studio 验收的实现缺口：

- `_start` 正常返回可用；现有代码把所有 Wasmtime call error 都判失败。标准工具链如使用
  `proc_exit(0)`，Studio 应区分成功退出与真实失败，并验证已有完整 JSON；提供方报告实际行为。
- 外层 Worker 先等待退出再读取管道，较大结果有潜在管道阻塞风险。48 KiB 上限只是产品约束，
  不是修复；Studio 必须验证最大输出，并在必要时并发排空管道，不能仅缩小正例掩盖问题。

这两项列为接收验收门槛，不要求提供方绕过隔离或手写不可靠的退出逻辑。

## 5. 五个试点工具与确定性语义

| sourceToolId | Capability ID | 成功 data.text |
| --- | --- | --- |
| json | devtools.text.json.format | JSON 两空格缩进；无末尾换行；对象键按字典序递归排序 |
| json | devtools.text.json.minify | 无空白 JSON；对象键按字典序递归排序 |
| base64 | devtools.text.base64.encode | UTF-8 → 标准有 padding 的 Base64，无换行 |
| base64 | devtools.text.base64.decode | 严格标准 Base64 → 合法 UTF-8；非法编码返回 INVALID_ENCODING |
| sha256 | devtools.text.sha256 | 输入 UTF-8 字节的 64 位小写十六进制 SHA-256 |

输入均为严格 `{text:string}`，不接受额外字段。JSON 工具限制深度 64，仅接受有限数值，
整数范围限制在安全整数 ±9007199254740991，范围外明确失败；不能悄悄损失整数精度。
重复对象键处理需实现拒绝或在交接时提出契约修订，不能在不同适配器中采用不同结果。
默认 schema 的字符数限制不能替代 UTF-8 字节检查。契约中的 fixtures 是行为对照，不代替真实 Host 验收。

## 6. UI 与 AI 接入

View 为包内自包含 HTML（当前最稳妥方式为内联脚本/CSS）；不 import Studio React 模块，
不访问父 DOM、Tauri IPC、原始 Pi RPC、网络或 file URL。独立 egui UI 继续只服务独立程序。
使用现有注入 SDK：`await window.zicode.ready()`、`context.get()`、`setDirty(bool)`、
`capabilities.invoke(capabilityId,input)`、`ui.showInformation(message)`。
`@zicode/plugin-sdk` 的未来示例不是已发布 npm 依赖。输入、错误和输出以 textContent 渲染。
至少覆盖窄窗/桌面、明暗可读、键盘、运行中、空输入、错误、重复点击与失效调用反馈。
当前 SDK 未承诺主题/语言推送事件；需要的新增通用接口由 Studio 负责，双方先记录缺口。

首包可声明五个 Pi Tool，名称如 `devtools_text_base64_encode`，绑定对应 capability 和相同 schema；
`pi.tools.register` 只作为 optional 权限。未授权时 View 仍可使用纯计算工具，AI 入口不可见。
不得静默把剪贴板、用户文件或整个目录内容送入模型。先安装/启用，后显式授权 Pi Tool。

## 7. 后续 profile 必须保持的接口原则

资源必须使用 Host 颁发、绑定插件/场景/工作区/操作/有效期的 opaque handle，不能直接接受路径
获得权限。写入先预览，按版本提交，权限撤销和作用域变化需失效；用户数据迁移由显式导入触发。
异步任务需请求 ID、开始/进度/终态、取消/超时、幂等键及进程回收，不能承诺当前同步 WASI 已支持。
网络通过 Host 域名/方法策略；凭据只传引用；系统能力按目标 OS 协商，不支持时返回稳定错误。
插件数据按 plugin ID 和 schema version 隔离，升级需备份/迁移/回滚兼容说明，卸载默认保留。

## 8. 分工与完成标准

ZiDevTools 会话先交付：全目录映射清单、独立 core 边界、五个试点的双产物构建、插件 View、
fixtures 结果和尺寸摘要；现有独立程序继续满足其 AGENTS 验收，不把 Studio 作为构建依赖。
Studio 会话负责：核对该包、Host 边界修复、安装/信任/启停/撤权/回滚/卸载，真实 View→Host→WASI
以及实际内嵌 Pi→Broker 调用证据。双方测试缺口单独记录，不能互相推定已经通过。
Windows 首批接收后，分别补 macOS ARM64/x64 和 Linux x64 的真实 Host 运行证据。
达到首批验收只代表五个工具通过，不代表完整 ZiDevTools 目录已迁移。

可复制的机器契约与样例位于仓库 `contracts/devtools-plugin/v1/`；文档是当前版本的解释说明。
