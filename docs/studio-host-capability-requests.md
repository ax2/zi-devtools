# Studio Host 能力需求提案：Agent、超级剪贴板与图片资源执行

日期：2026-10-14（图片资源补充：2026-10-15）\
状态：DevTools 提供方提案；不是双方冻结契约，也不代表插件包已交付或 Studio 已接受。

本文对应 Studio 接收契约 `zicode.devtools-plugin/1.0.0-rc.1` 的后续 profile 设计要求。现有 `compute.wasi.v1` 是无文件、网络、进程、持久状态的一次性调用；不能用 capability 声明替代 Host 回调。Studio 的 `host-api.md` 已列出 `zicode.pi.sendPrompt` 等通用接口名称，权限目录也列出 `clipboard.read/write`，但这些描述没有冻结 Agent 所需的模型选择/MCP执行/运行取消边界，也没有提供格式化剪贴板与 OCR 的 typed desktop 回调。图片资源 WASI 虽有现存资源输入/输出 framing，当前 fuel profile 仍无法完成实测图片变换。以下缺口应由 Studio 在版本化协议中实现并提供 conformance fixture，然后 DevTools 才能独立构建相应插件候选。

## 当前目录证据

`docs/tools.json` 中 `agent-runner` 1.5.0 为 implemented、`super-clipboard` 0.20.2 为 in-progress。`docs/studio-plugin-migration.json` 中两项均为 `await_host` / `not_delivered`，没有 candidate capability。Studio 的交接说明指出尚无后台模型/MCP 服务接口，也无 `desktop.v1` 剪贴板/系统 OCR 回调。不能将独立桌面已实现描述成插件已接入。

`plugin-market`、`plugin-updates` 属于宿主自身安装与更新生命周期；不应交付为能更新 Studio 的插件。`explorer-menu` 只能作为宿主注册的文件动作声明，不能让插件直接修改 Windows 注册表或创建每工具菜单。它们不在本文的 Agent/剪贴板插件候选范围。

## 通用异步服务约定（service.v1 提案）

服务操作由 View SDK 的 Host 注入 API 发起，WASI 计算模块仍只执行纯计算。不得暴露任意 URL、凭据、shell、进程或本机路径。所有请求都有 Host 产生的请求 ID、插件/场景绑定、绝对超时、取消和稳定错误类型。权限撤销、场景关闭或调用方卸载会使请求失效；迟到结果不可进入新一代 UI 状态。

机器可读的 request/response/event 草案和正反样例见 [`contracts/studio-host-capabilities/draft-20261014/`](../contracts/studio-host-capabilities/draft-20261014/)。其中的 `service.v1`、`desktop.v1` 仅为 DevTools 提供方提案，尚未冻结或实现。建议请求外壳：

```json
{
  "protocol": "zicode.service.v1",
  "requestId": "host-issued-opaque-id",
  "pluginId": "com.zicode.devtools.agent",
  "sceneId": "agent.run",
  "operation": "model.generate",
  "payload": {}
}
```

成功、失败、取消、超时和权限撤销必须是不同终态。可选流式模式只能发送有界 `started`、`delta`、`completed`、`failed`、`cancelled` 事件；事件包含相同 requestId 和序号，Host 对总字节数、事件数与时长设上限。重连不得重放已发生副作用的 MCP 调用。模型生成允许取消，Host 应停止读取并关闭本次请求；不得宣称远端模型在网络已送达后一定停止计算。

## 图片资源 WASI profile（image.resize.v1 提案）

DevTools 提供方已实现 `zi-image-core` 原生/WASI 共用算法和 `zi-image-wasi` adapter。Adapter 只处理 Host 已授权的一项 PNG/JPEG/WebP 资源，输出缩放后的 PNG/JPEG/WebP，不读路径、不联网；输入和输出分别最多 32 MiB，图片像素上限为核心解码器的 16 MP。当前实现依赖 `resource.wasi.input.draft2` 与 `resource.wasi.output.draft1` framing；这里只请求版本化执行预算和生命周期控制，不要求 Host 改变这些既有 framing。

Studio Host 源码现包含图片专用 preview profile，明确 opt-in 后为 256 MiB Store memory、30,000,000,000 fuel、10 秒 deadline、恰好 1 项输入/最多 1 项输出、全局/单插件 2/1 Worker slots；profile admission 静态检查通过。Studio Worker CLI 使用该 profile 对 4 MP JPEG、4096×3906 JPEG、2 MP 透明 PNG 及 PNG/JPEG/WebP 的 3×3 输入输出矩阵均成功执行。三张主要图像分别消耗 5,638,357,787、23,264,663,800、2,124,882,293 fuel，最大观察到 197,525,504 bytes guest linear memory；近 16 MP Worker 完整调用约 7.23 秒。矩阵中七组原生/WASI 解码像素完全相同，WebP 输入的 PNG 和 WebP 两组各差一个像素的一个通道值 1；九组 Studio Worker 输出均与直接 WASI 结果逐字节一致。32 MiB+1 输出拒绝探针通过。完整输入、profile、Worker/module SHA 与输出证据见 [`studio-image-workflow-handoff.md`](studio-image-workflow-handoff.md)。这些是 Host Worker mode 的提供方验收证据，不等于工具中心安装、权限授权、资源登记/预览/接力/保存、取消或 lease 回收已由 Studio 接受。

只读审查还发现负向结果尚无可用的零资源错误终态：binary Worker 正常退出后进入 `plugin_resources/worker/output.rs::register`，图片输出检查先要求恰好一个输出资源，再校验结果 schema；而 adapter 的业务失败会返回 `ok=false` 且不生成图片资源。因此，Worker CLI 返回 `INVALID_PARAMETERS`、`INVALID_IMAGE` 等结果，并不能证明 Host API 会保留这些错误码；按当前注册逻辑，该帧将被归为 `invalid_output`。Studio 需要明确支持受限错误结果与零输出资源的组合（并禁止发布资源），或定义等价的稳定错误帧，再由 Host 入口做负向 conformance。该要求尚未实现或验收，不改变本节正向 Worker 结果，也不表示 Studio 已接受插件。

只读检查到 Studio 工作区的候选文件 `E:\zicode-studio-desktop\docs\plugins\resource-image-resize-preview.md`，其版本为 `resource.wasi.image.resize.v1@1.0.0-preview.1`，当时尚未跟踪，且明示 Host 尚未实现或接受。以下镜像该候选边界用于提供方准备，不表示双方已冻结契约或实现了 Host：单图 PNG/JPEG/WebP，输入/输出各 32 MiB，4096 单边/16 MP，guest memory 256 MiB、fuel 30B、deadline 10 秒、全局并发 2、每插件 1、每活动调用预留 384 MiB、profile 总预留 768 MiB；lease 须在进程退出和输入/输出缓冲释放后归还。沿用 input draft2/output draft1，capability 与 `requiredProfiles` 显式协商。30B 相对候选原先采用的 22.53B 样例约为 1.33 倍，但最新 4096×3906 fixture 消耗 23.265B，余量约为 1.29 倍，仍须由 Host 对冷启动、全部格式及边界 fixture 验证。

候选首版不提供进度事件。Host 需能强制终止同步图片 Worker；取消/超时后不登记迟到输出，并在进程回收与缓冲释放后才归还 lease。deadline、fuel exhaustion、资源超限、无效图片和用户取消应彼此区分。DevTools 没有修改 Studio，也没有发起安装、接受或签名操作。

Host conformance 至少覆盖：1–16 MP、4096 边长、三种输入与输出格式、透明度、损坏/截断/错误 MIME、尺寸预检、32 MiB 输出边界、fuel/deadline、立即/处理中/完成前取消、撤权、Worker 崩溃、取消后无迟到输出，以及并发 1/2/3、lease 回收和 768 MiB 总预留约束。跨平台编码应由 Host owner 预先规定字节一致或语义容差，目前两组 WebP 输入每图只有一个像素、单个通道差 1，候选契约尚未规定该容差。任何一项不通过前，迁移目录均应保持 `not_delivered`。

## Agent 插件需要的能力

### 会话资料与信任来源

插件界面允许用户提交一个 UTF-8 编码后不超过 2 KiB 的任务目标，并在本次会话加入最多 32 项、UTF-8 编码后合计不超过 32 KiB 的有界资料。JSON Schema 的 `maxLength` 按字符而非 UTF-8 字节计数，因此 Host 必须在派发前独立计算每个字段/合计字节数；`é` 等多字节字符的超限样例列在 `vectors.json`。资料项至少带稳定的本次运行 ID、显示名称、来源类型、信任标记、是否由用户明确选中及正文长度。来源类型可以区分用户手输、工具接力、粘贴文本和经系统文件选择器导入的文本；路径不得传给插件或显示在插件协议中。

文件导入由 Host 原生选择器完成，只向当前场景授予用户刚选中的有界 UTF-8 快照。拒绝二进制、超限、无效 UTF-8 与总预算溢出时保留原会话内容。数据仅在当前会话中；Host 页面关闭或显式清空后释放。此范围不要求插件任意浏览目录、保留绝对路径或自动恢复以前会话。

提案输入：

```json
{
  "goal": "2 KiB bounded text",
  "materials": [
    {
      "sourceId": "run-local-id",
      "displayName": "用户给定的文件名",
      "sourceKind": "explicit_file_selection",
      "trust": "untrusted_user_material",
      "selected": true,
      "text": "bounded UTF-8 snapshot"
    }
  ],
  "historySchema": 4
}
```

`trust` 只描述导入来源，不给资料内指令更高权限。Host/插件必须把资料作为不可信内容处理，不能把它转换为系统提示或自动执行代码。手选文件名也须限制长度并清除路径组件。

### 模型发现与生成

`model.generate` 的 `consentId` 必须引用 Studio 从可信用户交互签发的一次性授权，绑定插件 ID、场景 ID、操作和当前权限代次。`clipboard.subscribe` 的 `userGestureId` 也必须由 Studio 可信交互产生，并按插件/场景/操作/权限代次绑定和一次性消费。字段在 provider 请求体中出现不代表用户授权；插件自行选择的任意字符串、重放的令牌、绑定不匹配或权限代次过期时，Host 必须拒绝。相关有效、伪造、重放和错绑向量见 `contracts/studio-host-capabilities/draft-20261014/vectors.json`。这些 provider 侧向量不代替 Studio Host conformance。

Host 管理模型连接、凭据、允许的模型和网络策略；插件收到的是临时 opaque `modelRef`，不是端点、密钥、Cookie 或连接字符串。Studio 可在后续实现中把现有 Pi Broker 能力映射到该抽象，但必须明确是否支持选择模型、发现模型和独立一轮请求。模型列表由用户点击刷新；生成必须由用户点击开始，清晰展示将发送的目标与所选资料来源。未选择资料不得偷偷把剪贴板、目录或历史会话加入提示词。

生成请求至少包括：用户目标、明确选中资料快照、用户选择的模型引用、最大输出 token/字节、截止时间及逐条来源 ID。Host 返回的用量可以包含输入/输出 token 和截断标志，但不得将凭据、整个消息历史或原始网络错误写入后台任务摘要。模型错误、拒绝、超时、取消与响应超限须区分；失败或迟到响应不得覆盖较新的运行。

### MCP 白名单和运行状态

MCP 服务器配置由 Host 管理。插件只能列举当前用户明确连接且允许本插件/场景使用的服务器引用，再查看已发现工具的名称、说明和 schema。每次实际工具调用都要求用户明确批准；本轮最多四次，且只允许被用户确认的只读工具。Host 对插件/场景/权限代次、运行、服务器、工具、工具 schema 摘要和实际参数摘要绑定一次性授权；参数摘要建议按 [RFC 8785 JCS](https://www.rfc-editor.org/rfc/rfc8785) 对参数生成确定性 UTF-8 表示后计算 SHA-256，避免对象字段顺序或数值格式产生不同哈希。任一绑定值改变即拒绝旧授权。插件不得接收任意 MCP endpoint，不得自行连接 stdio 子进程、HTTP 服务，也不得把“工具名称里有 read”当只读证明。

每轮状态通过 service.v1 任务 ID 报告 queued/running/completed/failed/cancelled/timed_out/revoked。任务中心只获得固定阶段、调用计数、耗时和受限错误码；不记录目标、资料正文、模型输入/输出、MCP 参数或返回正文。已取消或撤权的运行无法重新提交迟到结果。

### 运行记录和引用

交付必须保留 Agent 运行记录 schema v4：仅当用户明确选择导出时才携带目标/资料正文；默认记录只允许安全元数据。引用必须指向本次实际传给模型的来源 ID，拒绝未知或未发送的引用。导出/导入校验包含重复 ID、缺失来源、正文超限、伪造引用、未授权 MCP、超过四次调用、取消后迟到结果、token/输出超限和 schema 旧版本向量。记录只描述本地客户端观察到的流程，不能证明模型正确性或服务器端没有继续处理请求。

## 超级剪贴板插件需要的能力（desktop.v1 提案）

Windows 是首个候选平台。macOS 与 Linux 在各自 Host 回调及真实系统测试完成前必须显示 unsupported；不能从 Windows 测试推断支持。

### 按格式最小授权

权限按格式独立询问和撤销：纯文本、图片、文件引用、HTML、RTF、OCR。插件启动时默认全部关闭，纯计算 View 可在不授权系统剪贴板时打开。Host 回调至少包含一次性 session ID、剪贴板 revision、格式、来源应用的经用户允许显示的友好名称、实际字节大小与剪贴板回流标记；不向插件返回无关格式或机器进程路径。

建议一次性快照的概念模型：

```json
{
  "protocol": "zicode.desktop.v1",
  "sessionId": "host-issued-opaque-id",
  "platform": "windows",
  "clipboardRevision": "host-issued-revision",
  "formats": [{"kind": "text", "content": "explicitly granted text"}],
  "origin": {"displayName": "经允许的应用名", "isPluginWrite": false}
}
```

系统历史与收藏由 Host 所有的用户数据存储管理，使用版本化 schema；迁移兼容已发行的 schema 1–7。插件升级不得清空或重建用户数据。截断、损坏、撤销权限或数据库写入失败时保留最后完整内容并显示错误，绝不因读取失败覆盖历史。

### 读写、回流与取消

读取只包含已授权格式并受每格式字节上限约束。剪贴板变更通知有去抖和队列上限；插件写回的内容须携带回流标记，避免再次捕获造成递归。写回只由明确用户动作触发，保留用户选择的富文本格式组合；失败不得静默丢失原纯文本。标签和组合输出留在用户可见的本机工作台中，修改/清空/保存有清晰结果。

图片 OCR 由 Host 原生系统 OCR 服务执行，不允许插件调用 WinRT 或读取任意图像路径。调用输入为当前授权的图片句柄、用户选定语言标签和可选像素矩形；Host 验证矩形在图片界内，返回有界 UTF-8 文本和稳定错误（语言未安装、图片格式不支持、超限、取消、权限撤销）。识别文本不自动保存、发送模型或复制回剪贴板。建议单结果最多 1 MiB，队列至多一个待处理请求，页面关闭或用户取消后丢弃迟到结果。

### 平台矩阵和自动化边界

每个格式与 OCR 分开报告 supported/unsupported/permission_denied/not_installed；权限撤销即时阻断后续读取/写入。文件引用只能由 Host 转换成绑定 revision、用途和有效期的 opaque 资源意图；文件内容读取另走 resource.v1 用户确认流程。插件不得获任意绝对路径，不得自动上传或保存文件、图片、OCR 文本、HTML/RTF 或历史内容。

## Host 自身职责（不打包为 DevTools 插件）

- 插件市场与更新：由 Studio 管理签名信任、包获取、差异预览、权限重审、事务升级、回滚、撤权和卸载。插件不得更改 Studio 插件、Host 或其签名密钥。
- 资源管理器菜单：插件最多声明扩展名/资源类型、动作 ID 和允许的选择数量。Windows Host 只注册一个统一入口，点击后再解析所选项和确认；其它平台逐项声明 unsupported。插件不直接注册 Shell 菜单或访问任意选中路径。

## 接收方提供的最小 conformance 矩阵

1. 正常模型发现/用户确认生成/流式完成与结果超限；同会话两轮并发、旧 generation 返回和用户取消。
2. MCP 未授权、撤权中调用、服务器/工具/schema 变化、只读声明缺失、第五次调用和参数超限均拒绝；任务摘要中无原始目标/参数/正文。
3. 资料 32 项/32 KiB 精确边界、UTF-8 多字节、32 KiB+1、二进制、失效引用及文件选择取消；路径从文件选择结果到模型 payload 全链路不可见。
4. 剪贴板每格式单独授权/撤权、Windows真实 Unicode 与多格式内容、写回回流抑制、标签/组合，文件选择与路径隔离，OCR语言未安装/边界矩形/取消/迟到结果；无权限时系统调用计数为零。
5. Windows之外的每项系统格式/OCR均有显式 unsupported 测试；数据 schema 1–7迁移、不兼容输入和损坏存储保留数据。
6. 首次安装、升级、回滚、撤权、卸载保留用户数据由 Studio 的正式包管理器测试；这些不能由 DevTools 的插件 View 自测冒充。

## 当前交付判断

在 Studio 发布并验证 `service.v1` 与 `desktop.v1` 之前，`agent-runner` 和 `super-clipboard` 的本地插件状态维持 `await_host` / `not_delivered`，没有可如实生成的对应 WASI 模块、`.zicode-plugin` 或接收证据。已有 compute 候选不会因此扩大这两项工具的覆盖范围。正式签名还要求经授权的提供方密钥；不使用 Studio 私钥、不生成伪造签名，也不把独立版 EXE 当作插件模块。
