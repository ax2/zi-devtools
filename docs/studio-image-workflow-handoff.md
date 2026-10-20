# 图片流程进入 Studio 的交付边界

## 2026-10-18：当前图片0.2.0及目录b91d5f51

图片缩放插件0.2.0已改用Host支持的整数范围参数，增加分类、搜索别名和资源类型过滤。23项实际WASI（含9项原生字节对比）、65项脚本测试及全工作区1337项Rust测试通过；Clippy和release构建通过。当前目录b91d5f51包含10个未签名候选包，Host完整接受仍0，未发布。

当前图片候选为 `release/plugin-candidates/image-resize-0.2.0-candidate-01`，新WASM与当前image-workflow 0.12.0核心构建。0.1.2的pattern/not输入声明不兼容Host，保留为失败中间证据；不可安装，也不作为当前交付。下文0.1.1及其他目录为历史记录。详见[本轮记录](iterations/2026-10-18-image-resize-input-contract.md)。


审计日期：2026-10-15\
提供方仓库：`E:\zi-devtools`\
基线提交：`65f9aa63c831a4540bdd5731aa4da3de76dbd4b8`\
源码状态：工作区有大量未提交改动；本轮交互验收代码和文档也未提交，不能把基线提交误认为包含这些改动。\
Studio checkout：只读检查了现有 Host Worker 与资源预算；没有修改 Studio checkout。

## 交付状态

本轮已产出图片 resize 单能力的未签名提供方候选目录和包外 SHA-256 验证收据。由于没有授权的发布者 Ed25519 私钥，没有生成 `.zicode-plugin` 签名容器，候选目录不能通过 Studio 的签名包安装器。Studio 专项会话随后通过真实 HTTP Host 执行了该未签名开发目录候选的一条 image resize 路径；这不等于签名安装器、正式签名容器或全目录接入验收。

| 字段 | 当前图片流程状态 |
| --- | --- |
| 插件包 / plugin ID | 未签名候选目录：`release/plugin-candidates/image-resize-0.1.1-candidate-02/`；`com.zicode.devtools.image` |
| 插件版本 / `plugin.json` schema | `0.1.1` / schema v2；精确要求图片 resize、output schema 与 resource output types 三个预览 profile；质量字段注明仅 JPEG 生效 |
| 工具版本 | `image-workflow` `0.11.0`，状态 `in-progress` |
| 独立图片工作台版本 | `image-tools` `1.7.0`，状态 `implemented` |
| 图片配方格式 | `kind=image-workflow`，`schema_version=1` |
| 动作版本 | `image.crop`、`image.resize`、`image.encode`、`image.info`、`image.ocr` 当前各为 v1 |
| 报告 schema | `image.info` 生成 schemaVersion 2 JSON |
| DevTools manifest profiles / permissions / output types | Studio 专项会话报告真实 HTTP Host 已发现开发目录 manifest/catalog 并完成单一 resize 能力；全目录仍 0/154，`studioReceiptVerified` 由 Studio 端独立记录 |
| 模块构建 / fixtures / 摘要 | 资源 WASI 模块、固定 fixtures、当前 Studio Worker 结果与 package tree 清单已验证；模块 SHA-256 `a38e4743475f33f8c04aed87e064c942067956d4d0a366a4a12e36371df8f77a`，当前候选 tree SHA-256 `ef8cfd90cb11e6685f0e32e89e55f10ec53af564468f0bfde6cae025baace257` |
| 签名 | 未签名目录；未生成 `.zicode-plugin`；需授权发布者密钥后才能形成正式签名容器 |

本轮同时增加了原生/WASI 共用的纯 Rust 图片算法核心及受限资源 WASI adapter。Studio 迁移目录仍将 `image-workflow` 标记为 `standalone_incomplete`、`not_delivered`，`candidateCapabilities=[]`、`pilotCapabilities=[]`；单一 resize 路径的真实 HTTP Host 通过不会改变这些全目录状态。本轮截图、算法一致性和预算测试不构成 `image-workflow` 全范围验收。

## 当前提供方能力与证据

ZiDevTools 原生图片流程已有独立图片工作实例、步骤定义和执行、裁剪/缩放/编码/信息报告、Windows 本机 OCR、会话图像内存预算、流程库和任务状态。实现位于 `src/image_tools/workflow/` 等应用模块；图像材料以进程内 `Arc<DynamicImage>` 传递，不序列化进文本流程快照。该能力运行于桌面应用，不是 Studio 插件接口。

新增接力路径从图片工具显式选择“文本操作流程”，确认后在数据工作台新增独立实例，以文本流程格式 v4 和 `image.info` v1 为起点。它生成 schemaVersion 2 的 JSON 报告，允许继续串接现有 JSON/文本动作。确认不自动执行、编码、保存或联网；内存像素不会进入快照，应用重启后需要重新接力。

本轮验证范围和结果：

- UI 输入夹具源码：`E:\zi-devtools\examples\image_text_flow_relay_ui.rs`；合成来源通过 `E:\zi-devtools\src\image_tools\relay.rs` 中的 `preview_relay_fixture` 和 `E:\zi-devtools\src\image_tools.rs` 中的 `preview_fixture` 在内存创建，没有外部图片文件。
- 动作链式报告单测：`E:\zi-devtools\src\text_flow.rs:1565`，命令 `cargo test --lib image_material_produces_versioned_report_and_chains_into_text_actions`：1 项通过，891 项过滤。
- `cargo check --example image_text_flow_relay_ui --features ui-preview`：通过。
- `cargo run --example image_text_flow_relay_ui --features ui-preview -- 'E:\zi-devtools\target\ui-preview\image-text-flow-dark'`：通过，验证暗色 1100×900 窗口的目标选择、确认、新实例接收、未自动运行及来源保留。
- `& '.\target\debug\examples\image_text_flow_relay_ui.exe' 'E:\zi-devtools\target\ui-preview\image-text-flow-light' light`：通过，同范围浅色验收。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`：通过。
- `python scripts/sync_tools.py --check` 与 `git diff --check`：通过。
- 截图完整路径：`E:\zi-devtools\target\ui-preview\image-text-flow-dark\image-flow-0.png`、`E:\zi-devtools\target\ui-preview\image-text-flow-dark\image-flow-1.png`、`E:\zi-devtools\target\ui-preview\image-text-flow-light\image-flow-0.png`、`E:\zi-devtools\target\ui-preview\image-text-flow-light\image-flow-1.png`。它们是本机、可重建的 UI 验收证据，不是插件包素材或发布证明。

该受控输入夹具验证原生窗口里的布局与按钮事件，不等同于物理鼠标、多屏、混合 DPI 或小窗口验收。先前全量 Rust 测试通过的结果未因本轮仅增加的 UI-preview 夹具而重跑；本轮 Clippy 编译覆盖了带 `ui-preview` 的所有 targets。本段只记录本轮范围，不扩大为图片工作流完整接受。

## 可移植核心与资源 WASI 实测

纯算法已提取到 `crates/zi-image-core/`，包括先校验尺寸再解码、裁剪边界计算、缩放尺寸、缩放及 PNG/JPEG/WebP 编码。桌面流程调用同一核心，`crates/zi-image-wasi/` 仅提供 `devtools.image.resize` 资源输入/输出 adapter；它只读宿主提供的一个 PNG/JPEG/WebP 资源，不读取路径或网络。范围为单图缩放并输出 PNG/JPEG/WebP，最大输入/输出各 32 MiB，宽度 1..4096。它不等于完整 `image-workflow` 插件。

验证命令与结果：

- `cargo test --locked -p zi-image-core`：6 项通过，覆盖三种格式、裁剪、尺寸及解码边界；新增 4096 边长与 16 MP profile 上限测试。
- `cargo check --offline -p zi-image-wasi --target wasm32-wasip1`：通过。
- `cargo check --locked -p zi-devtools --lib`：通过。
- 图片资源构建入口：`scripts/dev.ps1 dt-image-plugin --offline`；模块 ceiling 只在 `zi-image-wasi/build.rs` 设为 268,435,456 bytes，公共 WASI compute 默认仍为 64 MiB。候选模块长度 1,028,463 bytes，SHA-256 `a38e4743475f33f8c04aed87e064c942067956d4d0a366a4a12e36371df8f77a`。
- 重现与输入 fixture：`python scripts/benchmark_studio_image_resource.py`；完整机器结果为 `target/image-resource-benchmark/results.json`，当前 SHA-256 `aac8936362b94b6e498afd74629010a933c811ba742a673043bb3aba0a9cc335`。本轮因 Studio Worker 二进制更新而重跑；当前 Worker SHA-256 为 `0b3ae21ed1bb9852aafb1407d84aaaccbdb53e04d02850811e9f6d260546845f`。脚本复用仓库已有 Wasmtime 36 Python/C API 缓存，并以 `resource.wasi.image.resize.v1` 的 30B/10 秒 profile 调用 Studio Worker CLI；没有安装新的运行时，Studio checkout 保持只读。
- 候选目录构建与复核：`python scripts/package_image_resize_plugin.py`、`python scripts/verify_image_resize_plugin.py`；外部收据 `release/plugin-candidates/image-resize-0.1.1-candidate-02-verification.json` 独立于候选目录，记录 payload 清单、各文件摘要、WASM/Worker SHA、source tool scope、fixtures 与明确的未接受状态。0.1.0 candidate-01 保留为历史候选。

| fixture | 操作 | 成功执行的 WASI fuel | guest linear memory | WASI/native 输出 | 当前 Studio Worker |
| --- | --- | ---: | ---: | --- | --- |
| 4 MP JPEG | 缩至 1000px WebP | 5,638,357,787 | 51,838,976 bytes | 2,731,564 bytes；WASI/native 字节与解码像素一致 | 当前 Worker 成功，约 3.52 秒 |
| 15.999 MP JPEG（4096×3906） | 缩至 2000px WebP | 23,264,663,800 | 197,525,504 bytes（调用后 Wasm memory 高水位） | 10,332,476 bytes；WASI/native 解码图仅 2/3,814,000 像素不同，最大通道差 1 | 当前 Worker 成功，约 7.77 秒，低于 10 秒 profile deadline |
| 2 MP RGBA PNG | 缩至 1000px PNG | 2,124,882,293 | 29,229,056 bytes | 11,060 bytes，SHA-256 一致；alpha 73 保留 | 当前 Worker 成功，约 2.84 秒 |
| 越界 JPEG（5000×2000） | 拒绝解码 | 145,968 | 2,359,296 bytes | 在像素解码前返回 `IMAGE_DIMENSION_LIMIT`，无输出 | 真实 Worker 稳定拒绝且无输出 |

另跑 1 MP 的 3×3 输入/输出格式矩阵（PNG/JPEG/WebP 全组合），9 组都由真实 Studio Worker 与直接 Wasmtime 成功输出。WASI 与原生结果中 7 组解码后的像素完全一致；WebP 输入转 PNG 与 WebP 两组各有 1/640,000 个像素的单通道差 1。九组的 Worker 输出字节均与直接 WASI 输出逐字节一致。另有损 JPEG/WebP 编码与不同平台的字节一致性仍按解码像素比较。实际 Worker 矩阵请求包含明确的 `executionProfile`，使用 30B fuel、10 秒截止、单输入/输出配置。另通过 Worker 验证非法宽度返回 `INVALID_PARAMETERS`、MIME 与文件格式不符返回 `IMAGE_OPERATION_FAILED`，两种失败都没有生成输出。

为排查两种 WebP 输入差异，曾在临时构建中把 Lanczos3 换为 Catmull-Rom 做对照；它未消除差异，且扩大了多种格式的像素偏差，因此没有保留该改动，交付核心仍使用 Lanczos3。无缩放、只解码后重新编码的 WebP→PNG 对照在两端字节一致，证据将差异定位到缩放阶段，但尚未定位到可保证跨目标逐位一致的实现。

当前 Studio image resize profile 实测为 256 MiB Store memory、30,000,000,000 fuel、10 秒 deadline、恰好一项输入和最多一项输出（各最多 32 MiB）、全局 2 个 profile Worker slot、每插件 1 个 slot。4 MP、近 16 MP、透明 PNG 与九种格式组合均由真实 Worker 成功；越界维度和损坏图像分别返回 `IMAGE_DIMENSION_LIMIT` 与 `IMAGE_OPERATION_FAILED`，无输出资源。Host 拒绝 32 MiB+1 输出创建探针通过。本轮 Worker CLI 无在途取消控制，未实测取消、权限撤销、资源登记/预览/接力/保存及 lease 回收。

结论：候选 adapter 在 Studio 预览 profile 的 256 MiB/30B/10 秒预算内完成了全部 12 项正向 Worker 调用，包括最大近 16 MP fixture；模块 ceiling 本身不扩大 Host 预算。候选 30B 预算对实测 23.265B 最大样例约留 1.29 倍 fuel 余量，近 16 MP 的 7.23 秒墙钟运行也应在较慢设备上复测。两种 WebP 输入仍各存在一个像素、一个通道值的原生/WASI差异；候选不宣称逐字节确定性。以上不证明 OCR、裁剪/信息步骤、取消或完整工作流已迁移，也不代表 Host 已接受该能力。

## 与 Studio 图片 profile 候选对齐

只读检查发现 Studio 工作区新增 `E:\zicode-studio-desktop\docs\plugins\resource-image-resize-preview.md`，候选版本 `resource.wasi.image.resize.v1@1.0.0-preview.1`。该文件在检查时为未跟踪状态，明确写明 Host 尚未实现或接受；本仓库不因此声明 manifest 兼容，不据此创建插件包，也没有修改 Studio。

候选边界要求 capability-scoped `executionProfile` 与 `requiredProfiles` 协商，沿用 input draft2/output draft1；单图 PNG/JPEG/WebP、单输入/输出各 32 MiB、4096 单边/16 MP，guest 256 MiB、fuel 30B、10 秒硬截止，全局并发 2/每插件 1，每次调用预留 384 MiB、profile 总预留 768 MiB。预留须在复制输入或模块前取得，Worker 与子进程退出、缓冲释放后才归还。首版不含进度事件；取消靠 Host 强制结束同步 Worker，不能仅靠 guest 轮询。

30B 在候选文档原先采用的 22.53B 样例上约有 1.33 倍预算；本轮 4096×3906 边界 fixture 消耗 23.265B，预算倍数约为 1.29。Studio 仍需在真实 Host 下复测冷启动、所有输入/输出格式、16 MP/4096 边界、32 MiB 输出上限、10 秒截止、取消时机、同插件/跨插件并发及 lease 回收；这些是候选验收门，不是当前结果。直接 Wasmtime 的 3×3 格式矩阵已完成，Studio Host conformance 仍未完成。

### Worker 负向结果与 Host 注册的差异

历史只读审查根据 Studio Host 源码发现，成功退出的 binary Worker 会进入 `plugin_resources/worker/output.rs::register`；当时 `validate_image_resize_outputs` 在结果 schema 校验前要求恰好一个图片输出资源，因此推断无输出错误帧会变成 `invalid_output`，遮蔽 adapter 的错误码。该发现描述的是修复前状态。

Studio 专项会话随后通过真实 HTTP Host 验证零输出错误映射已修复：坏 PNG 返回 `INVALID_IMAGE`、不登记输出，Host 使用固定错误文案，不暴露 provider message。该会话同时验证了成功 PNG 输入、WASI resize、派生资源登记与读回 SHA/解码结果。仍未逐项验证非法参数、MIME 不匹配、尺寸超限、fuel/deadline 失败等全部负向边界；不能把一个坏图 fixture 的成功当作所有 Worker 失败类型均已验收。

## 进入独立插件前缺少的提供方工作

| 缺口 | ZiDevTools 提供方需要交付 |
| --- | --- |
| 可复用算法核心 | 已完成第一步：`zi-image-core` 是独立纯 Rust crate，原生图像流程已调用它。当前可移植 API 只覆盖安全解码、裁剪几何、缩放与编码；信息报告及 OCR 尚未收敛到可移植核心。 |
| 明确的材料 ABI | resize adapter 使用 Host 资源 framing，校验单一输入资源的 MIME、长度、content version 和操作参数，并输出 MIME、长度、尺寸 JSON；仍缺经 Host owner 冻结的公共 schema、端口声明和跨版本兼容承诺。 |
| WASI 适配与预算 | 已有 `wasm32-wasip1` resize adapter、32 MiB I/O 边界和 16 MP fixture。Studio Host 当前源码含图片专用预览 profile，显式 opt in 时使用 30B fuel、10 秒 deadline、256 MiB memory、1 输入/1 输出和 2/1 并发；Worker mode 正向矩阵已通过。桌面工具中心的安装/授权/资源登记/预览/接力/保存、取消及 lease 回收仍待 Host 端到端验收。 |
| 插件清单与 View | 形成有版本的 plugin manifest、动作 schema、目录条目及自包含 View；profiles、permissions、输入/输出类型要等待 Studio 公共 Host 合约核对，不能由提供方自行宣布 Host 能力。 |
| 一致性证明和打包 | 用固定 fixtures 做 native/WASI 逐字节或语义一致性、错误边界与取消测试；验证 View、包内容和外部 SHA-256。获得经授权的签名流程前，只能交付注明 unsigned 的候选目录，不得伪造签名或声称已安装。 |
| OCR 边界 | 当前 `image.ocr` 使用 Windows 本机 WinRT OCR，不能进入通用 WASI 包。需要另有跨平台 Host OCR 能力及权限契约并得到 Studio 接受，或将 OCR 明确留在 ZiDevTools 原生版。 |

建议首个 Studio 候选严格只覆盖一个 `devtools.image.resize` capability：对一项受授权图片资源做缩放与输出编码。不包含裁剪、独立编码/信息报告动作、OCR、文件路径读取、持久材料库、后台任务中心或跨实例 UI。其它动作分别评估 schema、资源和预算后再迁移。即使做出候选包，也只表示提供方按候选契约构建完成，须由 Studio Host 实现并验证真实安装和执行生命周期后才能标为接入。

## 双方责任与应用边界

ZiDevTools 负责纯 Rust 核心、WASI adapter、自包含 View、fixtures、验证器、unsigned 候选包和包外摘要；Studio 负责冻结并公布可用 Host SDK/能力契约、确认 manifest profile/permissions/端口类型、验证真实安装/运行/更新/回滚/卸载与授权。Studio 未确认前，候选不能变成 `accepted capability`。

原生 ZiDevTools 保持独立 EXE 和原有图像工作流。插件化时以同一纯 Rust 核心供原生 adapter 与 WASI adapter 调用；不把 Studio 运行时设为原生版依赖，不委托独立 EXE，不使用 sibling checkout、共享用户数据库或本地 HTTP 绕行。

## 2026-10-15 Studio 专项会话：真实 HTTP Host 验收补记

以下结果来自 Studio 专项会话的完成回报；本次 DevTools 会话没有重跑 Studio checkout、Host 或真实 HTTP 验收。candidate payload 与其外部 verification receipt 均保持原样。

- Studio 当前 Debug EXE 的真实 HTTP Host 路径已通过 `actual_http_devtools_image_resize_candidate_host_flow`：安装 unsigned 开发目录，发现 manifest 与 catalog；用户授权 PNG 输入后执行 WASI resize，登记派生输出并读回核对 SHA 与解码结果。
- 错误输入 PNG 返回 `INVALID_IMAGE`、没有输出，审计结果标为失败。Studio Host 已修正零输出错误的映射，并以固定错误文案隐藏 provider message。
- `studioReceiptVerified` 仍由 Studio 端独立记录。本次单工具 HTTP 通过不等于 `image-workflow` 全面接入：原生窗口 UI、预览与接力保存尚未验收，全目录仍为 0/154。

这项新验收更新了前文“Host 对零输出错误返回 `invalid_output`”的当前状态；保留旧段落作为问题发现记录，当前修复与范围以上述补记为准。
