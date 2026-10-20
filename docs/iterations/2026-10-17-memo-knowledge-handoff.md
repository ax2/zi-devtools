# 已保存备忘接力到本机知识采集

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
工具版本：备忘录 `1.5.0`；备忘资料库互通 `0.4.0`（本轮范围）

## 目标与实现

在备忘录固定操作区增加“送到知识采集”。仅对已保存、未修改、未删除的普通备忘开放；执行时再次复核记录状态。标题、来源和正文进入已有知识采集表单的内存草稿，用户随后仍需准备正文预览、选择本机知识源并明确保存。此接力不直接写入文件、不扫描目录、不触发索引，也不改动原备忘。

复用现有知识采集的验证、正文上限和保存流程，没有增加第二套存储或索引。知识源扫描/索引自动化仍未完成。

## 验证

- 定向单测覆盖已保存普通备忘可接力、未保存编辑及日程拒绝，以及知识采集保留标题且仅生成未预览内存草稿。
- eframe 截图：[`memo-knowledge-handoff.png`](../../screenshots/memo-knowledge-handoff-20261017/memo-knowledge-handoff.png)，1600×1125 浅色主题；已检查固定操作区入口可见，未遮挡编辑器。
- 完整 `cargo test --offline --locked --features ui-preview --quiet`：1003 通过、0 失败、36 忽略；忽略项依赖外部模型或网络环境。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo build --offline --locked --release`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过。
- `git diff --check` 通过；工作区有既有修改，Git 对大量文件提示 LF/CRLF 转换警告。

## 遗留

知识源扫描和索引更新仍由现有工作流显式操作；尚未接入自动扫描/索引。没有修改程序版本或发布安装包。
