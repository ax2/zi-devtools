# 2026-10-18 录屏选区抓屏验证与外置构建缓存入口

## 目标

在保留 E: 项目 target 增量缓存的前提下，给本地开发提供明确选择的 C: 用户缓存入口；验证录屏选区快照能够读取主屏像素，并把验证边界写清楚。

## 修改

- `scripts/dev.ps1` 增加 `-UseUserCache`。只在本次 Cargo 子进程运行期间使用 `%LOCALAPPDATA%\ZiDevTools\zi-devtools`，检查现存路径祖先及缓存根不是重解析点，拒绝同项目并发构建，并在退出时恢复调用 Shell 的 `CARGO_TARGET_DIR`。默认构建路径不变；不移动或删除 E: 项目缓存。
- `docs/development.md` 记录显式切换方法、默认缓存策略及检查外置缓存的命令。
- 录屏选区流程在主窗最小化、等待桌面重绘后读取显示器快照，并在选择完成、取消或失败时请求恢复主窗口。
- `src/recorder_ui.rs` 增加 Windows 交互桌面的忽略式 GDI 抓屏冒烟测试，检查主屏尺寸、像素数和 alpha 通道。
- `docs/recorder-tutorial.md` 标明此测试不证明画面视觉正确，也不覆盖隐藏窗口到浮层选择、取消恢复的端到端操作。

## 验证

- `./scripts/dev.ps1 -UseUserCache dt-test`：主程序 1,085 项通过，0 失败，33 项忽略；服务生命周期、工作区及 WASI crate 测试通过。
- `./scripts/dev.ps1 -UseUserCache dt-lint`：通过，`-D warnings` 严格检查完成。
- `cargo fmt --check`：通过。
- `python scripts/sync_tools.py` 与 `python scripts/sync_tools.py --check`：均报告目录有效（92 项已实现，62 项规划/开发中）。
- `git diff --check`：通过。
- 交互桌面专项命令 `cargo test --locked recorder_ui::tests::captures_primary_display_snapshot_in_memory -- --ignored --exact`：1 项通过。像素只留在测试进程内存中。

## 结论与遗留

外置缓存入口已可用于后续明确选择的开发构建，完整项目缓存仍保留。屏幕快照 API 返回完整缓冲，但当前桌面测试不能核实捕获画面的实际内容，也未自动化验证主窗口最小化、框选浮层、多屏/混合 DPI、真实鼠标输入或恢复失败行为；这些仍需桌面端验收。没有更改程序、工具目录或插件版本，也未创建发布包。
