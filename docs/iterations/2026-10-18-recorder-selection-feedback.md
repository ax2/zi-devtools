# 2026-10-18 框选错误在浮层内反馈

## 目标

在主窗口隐藏期间，让用户能直接看到框选失败原因，并可立即重试。

## 修改

- 框选区域小于 32 × 32 时，主窗口状态之外，选择浮层上方也显示高对比错误消息和深色底板。
- 开始下一次拖选立即清除旧错误，显示当前选择状态；成功选择与取消的原有流程不变。
- 教程录屏辅助工具版本升至 `0.11.1`，同步工具目录、版本历史、工具说明和产品路线图；程序开发版本不变。

## 验证

`cargo fmt --check`、`python scripts/sync_tools.py`、`python scripts/sync_tools.py --check` 与 `git diff --check` 通过；目录为 92 项已实现、62 项规划/开发中。`./scripts/dev.ps1 -UseUserCache dt-test` 主程序 1,085 项通过、0 失败、33 项忽略，所有运行的集成测试通过。`./scripts/dev.ps1 -UseUserCache dt-lint` 严格 lint 通过。`./scripts/dev.ps1 -UseUserCache build --release --locked` 成功，release 构建用时 22 分 11 秒；仅生成 C: 用户 Cargo 缓存下的 41,430,528 字节 EXE，SHA-256 为 `ABAF77C905691966708E24BDAC58CBD1433A5EFEBF2DDE7D39F3942E31FD7543`，未生成安装包或发布版本。

## 未覆盖

自动化 UI 测试未驱动真实桌面鼠标，浮层位置、多显示器/DPI 和主窗口恢复端到端操作仍需实际桌面验收。
