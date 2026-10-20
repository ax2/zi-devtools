# 后台任务身份增加 Job owner UUID

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`task-runtime 0.5.0`（进行中）

## 目标

`task-runtime 0.4.0` 的复合 `RunId` 使用工具 ID、实例 ID 和由各自 `Job` 计数的 generation。两个不同 Job 若恰好使用相同工具/实例且都处于同一 generation，会得到相同身份，导致任务中心把其中一条覆盖，或一条清理墓碑屏蔽另一条。为每个 Job owner 加入随机 UUID，保留现有 generation 供工具取消逻辑校验。

## 修改

- 每个 `Job::default()` 分配新的 UUID owner；同一 Job 的后续 generation 继续共享 owner。
- `RunId` 与清理槽位都包含 owner UUID、工具 ID 和可选实例 ID；`RunId` 另包含 generation。
- 任务中心因此能区分同时运行的两个同类型任务，即使它们的实例 ID 和本地 generation 相同。
- 完成记录仍只保存显式允许的非敏感字段；owner UUID 不写入任务历史数据库。
- 工具目录提升到 `task-runtime 0.5.0`。owner UUID 仅在 Job 生命周期内有效，不宣称可跨进程恢复。

## 验证

- 同 generation、同工具、同实例的两个 Job 得到不同 RunId；任务中心同时保留二者，清理后两者的迟到快照都被各自墓碑拒绝：针对性测试通过。
- `cargo test --offline --locked`：单元测试 1,057 项通过、32 项忽略；集成测试 52 项通过、9 项忽略；0 失败。
- 默认与 `ui-preview` 两套 `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check` 均通过。
- `python scripts/sync_tools.py` 与 `python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；目录共 92 项已实现、62 项规划/进行中，插件迁移映射包含 154 项，accepted capability 为 0。
- `git diff --check` 通过。
- `cargo build --release --offline --locked`：通过，耗时 10m 39s。生成 `target/release/ZiDevTools.exe`，41,085,440 字节，SHA-256 `145A41B6E5C40B2B8017B7CD79A6B81BB8877B56AEA6F5E7B18A9C12F43E46A5`。

## 遗留

- UUID 不会在 Job 销毁后自动恢复，也没有持久化队列、崩溃恢复或重启恢复协议。
- 页面关闭隔离、统一暂停和结果接力仍待实现。
