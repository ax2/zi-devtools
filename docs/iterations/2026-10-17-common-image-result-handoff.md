# 图片结果进入统一工具接力

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）

## 目标

将图片流程的当前图像结果接入全局“发送结果到工具”审核入口，使类型化材料有一致的预览、目标兼容性和明确确认路径。

## 修改

- 当图片页当前没有文本结果时，统一结果栏可为当前图片流程输出创建 `Material::Image` 接力；已有 OCR 或图片报告仍优先作为文本，不改变既有输出选择语义。
- 图片材料只能选择文本操作流程目标。确认后创建独立工作实例，携带像素与图片来源链、加入 `image-workflow` 当前工具版本；像素只留在会话内，不转成文本、不保存进快照，也不自动运行 `image.info`。
- 文本流程工具提升至 `1.12.0`，图片流程提升至 `0.12.0`；程序版本不变。

## 验证

- 两项定向单测覆盖统一目标、内存预览、目标类型限制、新实例接收但不运行，以及选中图片输出的来源链和工具版本；默认与 `ui-preview` 特性下均通过。
- `cargo test --offline --locked`：1,045 项通过、32 项因本机服务/交互设备/外部 SDK 忽略、0 失败；集成测试和文档测试通过。
- `cargo test --offline --locked --features ui-preview`：全套 UI-preview 库、集成和文档测试通过。
- 默认和 `ui-preview` 特性下的 `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 和 `git diff --check` 通过。
- `cargo build --release --offline --locked` 通过。程序版本仍为 `0.82.0-dev.105`；`target/release/ZiDevTools.exe` 为 40,991,232 字节，SHA-256 `C28AECB45769E8B16EBC16519F068B4AE94BBBAAA5444D6602F9A0E688A789E2`。
- 新增 `common_image_result_handoff_ui` 原生 eframe 验收夹具；真实 Rust UI 在 1100×760 窗口中以合成指针按下/释放点击结果栏和确认按钮。暗色、浅色各运行通过，验证了审核弹窗的图片类型、来源版本和限制说明，以及确认后新建未运行流程实例。界面截图在暗/浅主题下经目视检查，并保存为[审核弹窗](../../screenshots/common-image-result-handoff-20261017/handoff-review-dark.png) / [接收结果](../../screenshots/common-image-result-handoff-20261017/handoff-received-dark.png)，对应浅色截图也保存在相同目录。该夹具不是物理鼠标验收。
- 未发布安装包；也未因本次工作清理构建缓存。
