# 更新预览中的工具版本分组摘要

## 目标

让用户在检查程序更新时看清各独立工具的版本目录变化，避免更新包版本掩盖工具自身的新增、升级、状态调整或移除。

## 修改

- `src/updates/signed.rs` 将平面字符串差异改为新增、版本/状态变化、移除三组；对照使用稳定工具 ID，未变化项继续隐藏。
- 下载预览、便携升级预览和 MSI 升级预览共用同一分组渲染，保持原来的签名校验、升级确认、取消和恢复流程。
- 在工具版本规则与桌面产品路线图中记录该能力；程序开发版本没有递增。

## 验证

- `cargo fmt --check`：通过。
- `cargo test updates::signed::tests::tool_changes_are_grouped_by_add_update_and_removal`：通过；验证同一目标同时包含新增、升级和移除时分类、内容与数量均正确。
- `cargo test`：通过，898 passed / 27 ignored；忽略项需要本机 Ollama、官方 MCP SDK fixture 或公网 HTTPS 等外部条件。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`：通过。
- `cargo check --all-targets --features ui-preview`：通过。
- `cargo build --release`：通过，未递增 `0.82.0-dev.105`。
- eframe 合成 UI 预览截图经人工检查：浅色主题，1600×1125；新增、版本/状态变化、移除三组均清晰显示。截图归档于 `C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-update-tool-summary\update-signed-light.png`，SHA-256 `F39471FBDFEC68B8DEB4976E5CD0A7C6FD0A71AEF08F107182E13750A239F1FD`。
- 首轮测试编译输出窗口到期后，确认同一 Cargo 测试进程仍在运行并等待其结束；随后重跑测试取得明确通过结果。没有中断 Studio 项目的并行 Cargo 检查。

## 边界与后续

- 本轮验证差异分类逻辑并人工检查下载预览；便携和 MSI 更新界面通过共享组件及合成夹具编译检查，尚未在真实发布包中逐项人工验收。
- 版本筛选、按工具分组的完整发布说明（包含用户可读变更文案而不只是版本/状态差异）仍待实现；签名差异只证明目录元数据变化，不证明每个工具均已回归。
