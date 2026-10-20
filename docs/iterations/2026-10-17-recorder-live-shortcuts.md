# 教程录屏实时组合键显示（工具 0.9.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（不变）\
工具：教程录屏辅助，`0.8.0` → `0.9.0`。功能仍处于开发中。

## 目标与实现

在用户明确开启后，将录制过程中实际按下的快捷键组合绘制进录制帧，作为手动填写组合提示的补充。默认关闭，只有 Ctrl、Alt 或 Win 与一个白名单键同时按下时才显示；支持字母、数字、F1–F12 和常见导航键。普通文字、Shift 单独输入、多键组合及任何额外的未识别按键均不会显示。用户可选择四角位置。

采样只在开启设置后的录制帧回调中进行，暂停时帧回调提前退出，因此暂停不采样。实现不安装键盘钩子、不写键盘日志、不持久化输入。采样随帧率运行（约每秒最多 30 次），很短的组合可能漏显；用户应避免在录制敏感窗口时启用。原有手动组合提示仍独立可用。

## 验证

- `cargo fmt --all --check`：通过。
- `cargo test --offline --locked`：通过，1,026 项通过、31 项忽略、0 失败；忽略项依赖 Ollama 或显式外部服务。
- `cargo clippy --offline --locked --all-targets -- -D warnings`：通过。
- `cargo test --offline --locked --features ui-preview live_chord -- --nocapture`：2 项实时输入过滤与渲染测试通过，包括无修饰文字、多个已知按键、额外未识别按键、默认关闭和合成显示。
- `python scripts/sync_tools.py --check`：通过，清单共 154 项，92 项已实现，62 项规划或开发中。
- `capture_ui` 场景 352 / 353 使用真实 eframe 渲染深浅主题，均为 1600×1125；确认开关默认关闭，提示文字可见。画面仅为合成录屏区域，不是实际桌面捕获。截图：
  - [深色主题](../../screenshots/recorder-live-shortcuts-20261017/recorder-tutorial-dark.png)，SHA-256 `13429712D51CC606E6859CE8AA4E9FB8047CA3BF63935541A3EE7102826E8476`
  - [浅色主题](../../screenshots/recorder-live-shortcuts-20261017/recorder-tutorial-light.png)，SHA-256 `7116F07C7EE1A68B689914969EFA686C22F00843FFEF52671DA01A51E1B7ECA6`
- `cargo build --offline --locked --release`：通过，优化构建完成（7分15秒）。

## 遗留

未执行真实键盘与桌面录制验收；物理按键组合可能因帧率采样而遗漏。混合 DPI、高分辨率输出、真实桌面画面以及教程录屏完整验收继续待办。未发布新程序版本。
