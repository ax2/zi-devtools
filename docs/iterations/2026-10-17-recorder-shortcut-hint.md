# 教程录屏：手动按键组合提示（0.7.0）

日期：2026-10-17\
程序版本：0.82.0-dev.105（保持不变）

## 本轮变更

在录屏教程辅助中增加默认关闭的“手动按键组合”提示。用户可输入不超过24个ASCII英文字母、数字、空格及 `+ - /`，字母规范为大写；确认添加后，像素字形标签写入同一套预览和录制帧合成器，并可放到四角之一。关闭提示可单独移除，关闭所有效果也会清空。

按键内容由用户手动提供。没有全局键盘钩子、按键监听或真实输入捕获；UI 明确提示检查敏感信息。这是静态操作提示，不会跟随实际按键变化。

## 验证

- 定向测试覆盖默认关闭、角落定位、输入规范化、非法字符与长度拒绝。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\ui-preview-recorder-shortcut-hint 352 single` 与场景 353 实际渲染深/浅主题 1600×1125 窗口；截图已人工检查提示栏和导出预览内的组合标签。
- 深色截图 `screenshots/recorder-shortcut-hint-20261017/recorder-tutorial-dark.png`，SHA-256 `0967F74A938FC5D5889FED5EFE510CF6770C96DDDF7A324DF0ADB713AE2FE125`；浅色截图 `screenshots/recorder-shortcut-hint-20261017/recorder-tutorial-light.png`，SHA-256 `628E03700755B09151643C83C5B0778678D27FB7BBF5AFE1FCAF77745CA98BBA`。
- `python scripts/sync_tools.py --check` 通过。
- `cargo fmt --all --check`、`git diff --check`、`cargo test --offline --locked --features ui-preview --quiet`（1025通过、31忽略、0失败）、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings` 与 `cargo build --offline --locked --release` 均通过。
- 录屏效果仍采用同一 BGRA 合成输出，真实桌面、混合 DPI、高分辨率性能和音画同步仍待实机验收。
- 工具版本从 0.6.0 提升至 0.7.0；程序开发版本保持不变。
