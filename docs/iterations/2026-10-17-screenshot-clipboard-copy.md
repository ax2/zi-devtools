# 截图透明结果复制到剪贴板（0.4.7）

日期：2026-10-17\
程序开发版本：保持 `0.82.0-dev.105`\
工具版本：截图与透明套索 `0.4.7`

## 目标与修改

透明套索结果此前只能预览和另存，新增“复制透明 PNG”便捷动作。点击后在后台将选区编码为 PNG，并同时写入 Windows PNG 与透明 DIBV5 剪贴板格式；不点击时不会访问或改写系统剪贴板。

复制沿用剪贴板图像容量边界：最多400万像素，PNG最多16MiB。超出像素上限时按钮禁用；PNG 编码超限或剪贴板写入失败时显示错误。普通 PNG 另存流程、透明结果和原始捕获保持不变。程序开发版本未变，截图工具目录升至 `0.4.7`。

## 验证

- `cargo test --offline --locked --features ui-preview screenshot_clipboard_preparation -- --nocapture`：透明像素往返与像素边界测试通过。
- 全工作区 `cargo test --offline --locked --features ui-preview --quiet`：985 项通过、27 项按环境要求忽略、0 失败；可运行集成测试通过。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、工具目录同步、Studio 插件映射同步及 `git diff --check` 通过。
- `cargo build --offline --locked --release`：通过，release 优化构建完成（7分36秒）；程序版本未修改、未发布。
- 实际 eframe 1600×1125 浅色截图显示“复制透明 PNG”与“另存透明 PNG”并列，预览层级正常，截图保存在 `screenshots/screenshot-copy-20261017/screenshot-lasso-light.png`。
- 更新了私有 Windows 窗口站剪贴板夹具，使其实际验证截图复制输出；本机尝试运行时系统拒绝创建隔离窗口站（错误5，需要提升权限）。该夹具未访问交互式用户剪贴板，因此实际 Windows 剪贴板 API 写入尚未在本机完成验证。

## 边界

复制是用户点击后才执行的系统剪贴板写入。实际剪贴板操作尚缺提升权限的隔离窗口站运行验证；没有用真实用户剪贴板替代隔离测试。多屏、混合 DPI、物理鼠标及截图编辑/标注能力仍按工具目录继续开发。
