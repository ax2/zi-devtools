# 任务中心：可选的跨次启动历史

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`task-runtime 0.2.0`（进行中）

## 目标

任务中心的终态记录原先只存在内存，程序重启后无法回看。增加用户主动选择的本机历史，同时限制保存字段和记录数量，避免将任务结果、路径或输入内容写入历史。

## 修改

- `Preferences.task_history_local_enabled` 默认关闭；旧版偏好文件缺少此字段时仍关闭。
- 用户在任务中心勾选“在本机保存任务历史”后才写本机 `task-history.sqlite3`；状态开关先持久化到偏好文件，保存失败时不启用记录。
- SQLite 只存固定工具 key/名称、终态、耗时和完成时间，不存任务实例名、摘要、文件路径、输入、输出、generation 或取消句柄。最多保留最新 1,000 条。
- 重启后显示本机历史；关闭开关只停止新增记录，已保存数据保留并继续可见；“清空本机历史”需再次确认，删除后执行 SQLite secure-delete 与 VACUUM。
- 数据库读取校验普通文件、1 MiB 上限和 schema 版本；事务中插入并裁剪，失败不影响实际任务结果。列表最多绘制 100 项，仍可按工具名称搜索。
- 工具目录把 `task-runtime` 更新到 `0.2.0`，程序开发版本不变。

## 验证

- `cargo test --offline --locked task_history`：3 项通过，覆盖 opt-in 文件仅含非敏感字段、不可写入运行态/不兼容 schema，以及 1,000 条上限。
- 最新源码完整 `cargo test --offline --locked`：1,054 项通过、32 项忽略、0 失败。
- `cargo fmt --all -- --check`、默认与 `ui-preview` 两套 `cargo clippy --offline --locked --all-targets -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check` 均通过。
- 深浅主题真实 eframe 渲染截图：1600×1475，已检查历史列表、状态颜色、时间/耗时和保存控制层级；截图见 [深色](../../screenshots/task-center-local-history-20261018/tasks-local-history-dark.png) / [浅色](../../screenshots/task-center-local-history-20261018/tasks-local-history-light.png)。
- 最终 `cargo build --release --offline --locked`：通过，耗时 8m 26s。生成 `target/release/ZiDevTools.exe`，41,072,640 字节，SHA-256 `312637979CB0F3F1033FB0ACF22C52B1B8AB246497EE82B6D719DD20F2C7591E`。

## 遗留

- 此功能只保存已结束任务的有限索引元数据，不恢复仍在运行的作业，也不提供跨工具统一暂停或结果接力。
- 数据库归当前 Windows 用户的应用数据目录；文件系统/管理员级取证不属于此功能的保护范围。
- 本轮没有生成或发布新安装包，程序全局版本号保持不变。
