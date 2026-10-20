# 统一后台任务运行身份

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`task-runtime 0.4.0`（进行中）

## 目标

任务中心之前在若干位置重复组合工具 ID、实例 ID 与 generation，形成一次运行的身份。将其收敛为单一 `RunId`，让任务行更新/清理和取消请求传递相同的身份值，为后续任务跨页归属与持久化作业协议建立明确边界。

## 修改

- `RunId` 由稳定工具 ID、可选实例 ID 和运行 generation 组成；`Row::run_id()` 是从任务行读取其身份的唯一入口。
- 任务中心去重比较统一 `RunId`，清理墓碑改为使用 `RunId` 的任务槽位与最高 generation，避免迟到的旧快照让已清理运行复活。
- 打开任务结果以及单项和批量取消都传递同一个 `RunId`。取消处理器从该身份读取工具、实例和 generation，并继续调用各工作台现有的代次校验。
- 新增测试验证工具 ID、实例 ID 或 generation 任一不同都会得到不同运行 ID。
- 工具目录与版本说明提升到 `task-runtime 0.4.0`。`RunId` 只在当前进程内有效；持久化作业需要另行设计随机/稳定作业标识，不能复用本运行期 ID。

## 验证

- `cargo test --offline --locked tasks::tests::run_id_compares_tool_instance_and_generation_as_one_identity`：通过。
- `cargo test --offline --locked`（含最终打开路径调整）：单元测试 1,056 项通过、32 项忽略；集成测试 52 项通过、9 项忽略；0 失败。
- 默认与 `ui-preview` 两套严格 Clippy、`cargo fmt --all -- --check` 均通过。
- `python scripts/sync_tools.py` 与 `python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；工具目录为 92 项已实现、62 项规划/进行中，插件迁移映射包含 154 项，accepted capability 为 0。
- `git diff --check` 通过。
- `cargo build --release --offline --locked`（含最终打开路径调整）：通过，耗时 10m 49s。生成 `target/release/ZiDevTools.exe`，41,075,200 字节，SHA-256 `E02A0F934DDE810BB2BB9C707A2561A3704FA2E8E4282C69545CD359701C892D`。

## 遗留

- 当前 `RunId` 只表示进程内的任务运行身份，不保证跨重启唯一，也不支持崩溃恢复。
- 页面关闭隔离、统一暂停、持久化作业恢复和结果接力仍未完成；本轮不声称这些能力已实现。
