# 截图选区撤销与重做快捷键（0.4.9）

日期：2026-10-17\
程序开发版本：保持 `0.82.0-dev.105`\
工具版本：截图与透明套索 `0.4.9`

## 目标与实现

为上轮新增的选区撤销/重做补充快捷访问方式，同时避免在其他界面或无可撤销历史时拦截常用按键。`Ctrl+Z`撤销，`Ctrl+Y`与`Ctrl+Shift+Z`重做；只有对应历史非空且工作台空闲时才消费按键。工具版本升至 `0.4.9`，程序开发版本保持不变。

## 验证

- 新增 egui 原生键盘事件回归，依次覆盖 `Ctrl+Z`、`Ctrl+Shift+Z`、`Ctrl+Y` 并校验重算后的像素输出。
- `cargo test --offline --locked --features ui-preview --quiet`：988 项通过、27 项按环境要求忽略、0 失败；可运行集成测试通过。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、工具目录同步及 `--check`、Studio 插件映射同步及 `--check`、`git diff --check`：通过。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`：通过。
- `cargo fmt --all -- --check`、工具目录同步及 `--check`、Studio 插件映射同步及 `--check`、`git diff --check`：通过。
- `cargo build --offline --locked --release`：通过，耗时 7 分 45 秒；生成 `target/release/ZiDevTools.exe`（40,496,640 字节）。未发布版本。
- 复用 0.4.8 的界面截图：[screenshot-lasso-light.png](../../screenshots/screenshot-history-20261017/screenshot-lasso-light.png)；按钮位置未变，本轮新增鼠标悬停快捷键提示。

## 范围与遗留

历史仅覆盖选区轮廓和矩形/自由选区模式，不保留多份完整输出图像。验证截图来自受控 UI 预览，不代表已完成物理鼠标、多显示器、混合 DPI 或真实全屏捕获验收。工具继续处于开发中；除上述撤销/重做外的图片编辑、标注、打码和长截图等能力仍按工具目录规划推进。
