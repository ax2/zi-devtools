# 备忘资料库批量导出

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
独立工具版本：备忘资料库互通 `0.3.0`（开发中）

## 目标与实现

完成备忘资料库多文件交换中的导出部分。导出范围是当前备忘列表经过关键词与置顶筛选后的结果；不包括日程、回收站、未保存草稿或其他工具数据。每批最多 32 条，正文总量不超过 1 MiB。

点击“批量导出当前结果…”后逐条预览标题、文件名和字节数。用户选择一个父目录才开始写入；程序在其中创建唯一子目录，每条备忘正文写成 UTF-8 Markdown 文件。文件名净化 Windows 非法字符，并附加记录标识；同批文件名冲突会在写入前拒绝。源记录不会改变，也不会覆盖已有文件。

所有文件先写入新的暂存子目录，全部成功后再将目录改为正式名称。I/O 错误时保留暂存目录并报告位置，方便用户检查，而不静默删除可能有用的部分结果。导出不包含标签、日程、提醒或内部记录标识的正文内容。

## 验证

- 定向测试覆盖搜索筛选、文件名净化、UTF-8 与原始换行精确保留、导出后数据库记录不变。
- eframe 批量导出预览截图已检查，保存在 [memo-bulk-export.png](../../screenshots/memo-bulk-export-20261017/memo-bulk-export.png)。
- 全量 `cargo test --offline --locked --features ui-preview --quiet`：1001 通过、0 失败、36 忽略（其中 lib 951 通过 / 27 忽略，集成目标 50 通过 / 9 忽略；外部模型和网络测试按设计忽略）。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`cargo build --offline --locked --release`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过。
- `git diff --check` 通过；Git 仅提示既有工作区文件的 LF/CRLF 转换警告。

## 遗留

备忘到本机知识库的显式接力仍未实现。程序版本未更改，也没有发布安装包。
