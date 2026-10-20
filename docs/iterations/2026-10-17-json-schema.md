# JSON Schema 校验（0.2.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：JSON Schema 校验 `0.2.0`（开发中）

## 本轮交付

- 将规划中的 JSON Schema 校验接入“编码工具”，支持标准草稿自动识别、Schema 元模式校验、实例校验、JSON Pointer 数据路径、脱敏错误和内嵌 `#` 引用。
- 依赖 `jsonschema 0.59.0`，关闭默认 HTTP/文件 resolver；同时启用离线模式，拒绝 `$ref`、`$dynamicRef`、`$recursiveRef` 中任何非 `#` URI。正则格式采用线性时间 regex 引擎。
- 引用限制只遍历 Schema 定义子 Schema 的标准关键字，`const`、`default`、`enum` 等实例值中的同名数据键不误判。
- `0.2.0` 增加明确文件选择和根目录受限引用：读取根文件与被 `$ref` 指向的本机 JSON 文件；路径规范化后必须位于所选根文件目录内。最多 32 个外部文件、每个 64 KiB、累计 1 MiB；只按 Schema 显式引用读取，不枚举目录。
- Schema 声明 `$id` 时，远端 URI 的相对引用含义不映射成本机路径；此类歧义相对引用拒绝，用户可用仍受根目录约束的绝对 `file://` URI。
- 限制 Schema 为 64 KiB、待校验 JSON 为 1 MiB、结果显示最多 100 条错误。工具不改写输入、不自动修复，也不访问外部资源。
- 工具目录记录版本 `0.2.0` 和 `in-progress` 状态，程序版本仍固定为 `0.82.0-dev.105`。

## 验证

- `cargo test --offline --locked json_schema::tests -- --nocapture`：14 项通过，覆盖 Draft 2020-12 字段定位/值脱敏、内嵌 `$defs`、本机相对引用、根目录外路径拒绝、根级及嵌套网络引用拒绝、含 `$id` 的 URI 歧义、根文件和输入体积上限、最多32个引用文件限制、100 条错误截断，以及包含 `$ref` 键的普通 `const` 数据。
- `cargo test --offline --locked selected_schema_file_and_editor_contents_follow_their_tool_draft -- --nocapture`：通过，验证选定文件内容和路径随 JSON Schema 工具草稿切换保存并恢复。
- `cargo test --offline --locked --features ui-preview --quiet`：主库 980 项通过、0 失败、27 忽略；可运行集成测试通过，外部 Ollama/网络等依赖测试按环境忽略。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check` 均通过。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\ui-preview-json-schema 540 single` 与场景 541 均通过；深浅主题各渲染为 1600×1125，分别保存于 `screenshots/json-schema-ui-20261017/`。已检查两种主题中 Schema、实例、动作和结果的层级与可读性。
- `cargo build --offline --locked --release` 通过，耗时 6 分 01 秒。EXE 大小 40,392,704 字节，SHA-256 `53AAA0BE28150F072CF5B2FDA8D745178C54750F7B126C1C861DEC65548FD73B`；文件版本仍为 `0.82.0-dev.105`。未打包或发布。

## 遗留与边界

本地文件选择器的物理鼠标/键盘交互、窄屏布局和粘贴场景尚未验收。当前支持用户明确选择根文件后按需读取受限目录中的本地外部 Schema 引用；拒绝网络引用、目录外路径和含 `$id` 时有 URI 基址歧义的相对引用。不支持自动修复或 WASI/Studio 插件版。Schema 校验器完整支持范围取决于依赖 crate 的标准草稿实现；此处不把校验结果当作业务语义验证。
