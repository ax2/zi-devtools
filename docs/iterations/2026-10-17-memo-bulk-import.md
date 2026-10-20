# 备忘资料库批量导入

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
独立工具版本：备忘资料库互通 `0.2.0`（开发中）

## 目标与实现

让用户把多个本地 Markdown / UTF-8 文本文件一次导入为备忘，同时在写入前核对所有内容。批量选择最多 32 个 `.md`、`.markdown` 或 `.txt` 文件；每份正文最多 128 KiB，批次正文最多 1 MiB。读取不修改源文件，也不执行 Markdown、链接或 front matter。

所有导入内容在确认窗口中逐条展开预览。确认操作保存为独立新备忘；写入前以事务内快照比较本机记录，预览后如有任何记录变化则整批拒绝。数据库容量和记录验证在事务内统一执行，SQL 插入使用同一个 SQLite 事务，避免部分完成。

此迭代只完成批量导入。批量导出和主动接力到知识库仍在目录待办中；程序开发版本未更改。

## 验证

- 定向测试覆盖多文件 UTF-8 读取、标题推导、文件数限制、整批事务保存、过期快照拒绝且无部分插入。
- 工作流测试覆盖文件选择后的预览状态、确认前数据库未创建、明确确认后两条记录同时保存、原文件保持不变。
- 全量 `cargo test --offline --locked --features ui-preview --quiet`：合计 1000 通过、0 失败、36 忽略；其中 lib 单元测试 950 通过 / 27 忽略，集成目标 50 通过 / 9 忽略（需本机模型或外部网络的用例按设计忽略）。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`cargo build --offline --locked --release`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过。
- eframe 真实窗口截图已检查，显示多文件内容预览、容量边界说明和单次确认入口，保存在 [memo-bulk-import.png](../../screenshots/memo-bulk-import-20261017/memo-bulk-import.png)。
- `git diff --check` 通过；Git 提示工作区既有 LF/CRLF 转换警告，没有空白错误。

## 遗留

批量导出、目录写入失败恢复，以及把备忘显式发送到本机知识库仍待实现。尚未发布安装包。
