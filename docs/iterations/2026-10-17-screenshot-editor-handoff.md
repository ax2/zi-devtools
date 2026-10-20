# 截图选区快捷进入标注与遮挡（0.7.0）

状态：开发版迭代完成，独立工具版本 `0.7.0`；程序开发版本保持不变，未发布。

## 目标与实现

截图选区的箭头、文字、遮挡能力原先要用户先找到图片接力，再在通用对话框里选目标。截图工作台现在提供“箭头、文字、打码…”快捷入口，自动准备内存图片接力并预选现有图片编辑器。接力仍需预览并明确确认；目标编辑器有未保存工作时仍要求明确允许替换，取消时保留源截图与目标工作。流程不创建临时图片、不自动保存。

复用 `image-crop-annotate` 编辑器现有的箭头、文字和黑色遮挡操作，没有复制实现第二套图片编辑逻辑。接力带截图工作台来源工具版本链，目标在确认之前不会改变。截图与透明套索版本更新到 `0.7.0`，程序版本未变。

## 验证

- 回归测试使用合成截图发起快捷接力，核对目标预选为图片编辑器、透明 alpha 保留、源工作仍存在、编辑器工作区在确认前没有被填充。
- `cargo test --offline --locked --features ui-preview --quiet`：完整测试套件通过。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、工具目录同步检查和 `git diff --check`：通过。
- 渲染并目视检查浅色合成界面；截图是合成夹具，不是桌面捕获：`screenshots/screenshot-annotation-entry-20261017/screenshot-annotation-entry-light.png`，1600×1125，324548 字节。
- `cargo build --offline --locked --release`：通过，9分52秒；生成 `target/release/ZiDevTools.exe`，40,523,264 字节（2026-10-17 10:24），SHA-256 `A49959F7D1BA373A967207A7F1422CD2D5CF57C068820F3BA61A268BE9CDECEC`。本机验证构建，未发布。

## 遗留

该入口提供编辑器现有黑色遮挡、箭头和文字功能；截图工作台内直接标注、像素化/模糊遮挡、置顶截图仍未实现。截图取消后不会更改或清理目标图片编辑器的既有工作。多屏/混合 DPI 和真实桌面验收仍需专项检查。
