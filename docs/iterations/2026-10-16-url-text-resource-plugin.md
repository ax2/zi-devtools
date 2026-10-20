# URL 文本资源接收插件候选

日期：2026-10-16\
状态：提供方实现与 WASI 验证完成；Studio Host 验收待办。应用版本保持 `0.82.0-dev.105`，独立工具 `url` 版本保持 `1.0.2`。

## 目标

让 Studio 可将 OCR 等工具生成的 `text/plain` 资源，在用户明确授权后交给已有的 `devtools.transforms.url.encode`。保留 ZiDevTools 独立运行能力，不改 Studio 源码或其冻结测试夹具。

## 实现

- 新增 `zi-text-resource-wasi`，复用 `zi-text-core::transforms::url_encode`，不分叉算法。
- 候选清单只声明一项 `text/plain` capability，输入 schema 不接收额外字段，输出 schema 固定返回 `encodedText` 与 `sourceByteCount`。
- guest 上限为 8192 源字节；拒绝空文本、无效 UTF-8、超限文本、错误 MIME、错误资源描述和非预期 profile。仅有 Host 授权的单项资源经 `resource.wasi.input.draft2` 请求传入，guest 通过受限 resource input ABI 读取。
- 不访问文件路径、环境变量、网络或 Pi，不保存、不自动调用后续工具。WASM 导入仅包含 `wasi_snapshot_preview1` 与 `zicode_resource_input_draft1` 的 `size`/`read`。
- 插件包版本 `0.1.0`，工具 id 与版本精确保持 `devtools.transforms.url.encode@1.0.2`。候选包为未签名目录，不能称为 Studio 已接受或可安装发行版。

## 验证

- `cargo test -p zi-text-resource-wasi --offline --locked`：4 项通过。
- `cargo clippy -p zi-text-resource-wasi --all-targets --offline --locked -- -D warnings`：通过。
- `cargo build --offline --locked --target wasm32-wasip1 --profile plugin -p zi-text-resource-wasi`：通过。
- `scripts/verify_url_resource_plugin.py`：在 Wasmtime 36.0.0 中实际调用模块，7 项通过，包括 `ZiCode OCR test 2026`、UTF-8/保留字符、8192 字节边界、错误 MIME、无效 UTF-8、空文本与8193 字节超限；同时枚举 WASM 导入并确认没有文件路径或网络 API。WASM 标准启动存在 `environ_get`/`environ_sizes_get` 导入；适配器源码不读取环境值。
- 模块 225,023 字节，SHA-256 `ff233c72dcec2199aee7b559d6be9c1405e9d92ed72baaeb20b9468344dfe607`。
- `scripts/verify_url_resource_package.py`：通过，候选包 92 个文件，合计 602,144 字节，逐文件长度与 SHA-256 收据一致。

## 待 Studio 验收

清单搜索发现、按需加载、OCR 文本显式授权、可见编码结果、未单独确认时不保存，以及撤销权限、取消、stale transfer grant 拒绝。上述 Host 状态无法由提供方单独制造；当前收据将其标为未验证。还未验证 Studio 生产 Worker 的 fuel/deadline、macOS/Linux Host 行为、签名安装、升级和回滚。

## 产物

- 源码：`crates/zi-text-resource-wasi/`、`plugins/url-text-resource/`
- 构建：`target/wasm32-wasip1/plugin/zi-text-resource-wasi.wasm`
- 候选：`release/plugin-candidates/url-text-resource-0.1.0-candidate-02/`
- 收据：`release/plugin-candidates/url-text-resource-0.1.0-candidate-02-verification.json`
- 主机端负例：`plugins/url-text-resource/fixtures/host-negative-cases.json`
