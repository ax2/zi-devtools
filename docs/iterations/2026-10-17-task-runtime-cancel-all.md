# 任务中心：确认后批量取消

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`task-runtime 0.1.0`（进行中）

## 目标

任务数增加后，用户可以在任务中心一次请求停止多个本机后台任务，同时看清目标范围和取消边界。继续保留工具已有的任务 generation 校验，不让陈旧任务行作用到新一轮工作。

## 修改

- `Center::cancellable_running` 只返回 `Running` 且任务自身声明可取消的记录；已进入 `Cancelling`、已结束和不可取消任务会排除。
- 任务中心进一步检查已注册取消路由。未实现取消分派的 HTTP 请求与网络诊断行不显示误导性的“取消”操作。
- “取消全部可取消任务”先展示数量和最多六条任务标签；用户确认后重新读取目标集合，再按每条任务的 key、instance 和 generation 发停止请求。
- 单项与批量取消复用同一取消分派。确认区说明已写入数据和已发送请求无法撤回，远端服务可能继续处理已接收请求；“返回，不取消”立即关闭确认。
- 文件校验取消改为校验 generation 和接收器仍活动。过期任务记录不会停止新一轮校验。
- 工具目录将 `task-runtime` 设为进行中 `0.1.0`。这不是统一任务运行时完成标记；任务持久化、跨页面/关闭隔离和统一结果接力未实现。

## 验证

- 单元测试验证批量候选只含运行中可取消记录；取消中及不可取消记录均被排除。
- 取消路由测试验证仅声明实际分派入口的任务类型显示取消操作。
- 文件校验测试先构造旧、新 generation，再确认旧 generation 被拒绝且取消令牌不变，新 generation 才能进入取消状态。
- `cargo run --offline --locked --features ui-preview --example capture_ui -- target/ui-preview-task-cancel-all 552 single` 与场景 `553`：真实 eframe 渲染 1600×1475 深/浅主题确认界面；截图已检查，目标列表、取消影响说明和返回按钮可读。截图归档见 [暗色](../../screenshots/task-center-cancel-all-20261017/tasks-cancel-all-dark.png) / [浅色](../../screenshots/task-center-cancel-all-20261017/tasks-cancel-all-light.png)。
- `cargo test --offline --locked --lib cancellation_tests`、`bulk_cancel_candidates_include_only_running_cancellable_rows`、`file_task_cancellation_rejects_stale_generation`：通过。
- `cargo test --offline --locked`：1,050 passed，0 failed，32 ignored。
- `cargo clippy --offline --locked --all-targets -- -D warnings` 与 `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`：均通过。
- `cargo fmt --all -- --check`、`python scripts/sync_tools.py --check` 与 `python scripts/sync_studio_plugin_map.py --check`：通过。
- `cargo build --release --offline --locked`：通过，耗时 8m 50s。生成 `target/release/ZiDevTools.exe`，41,028,608 字节，SHA-256 `57E5458BA491A8172ACADAC5FFD700D31764AF1242CF6B6EEC47B6D71740F9B7`。
- `git diff --check`：通过；Git 仅提示当前工作树多处换行符归一化警告。

## 遗留

- 确认只发出取消请求；某些任务已完成的磁盘写入或网络副作用不会自动回滚。
- 任务中心仍按本次运行内最近 50 项保留历史，不支持跨重启恢复、统一暂停、提交给其他工具或统一持久化结果。
- 取消任务类型和具体处理器目前仍需在应用内维护映射。后续统一任务运行时应把能力和取消执行器一同注册，并为跨页面/窗口的取消与结果接力提供稳定 RunId。
