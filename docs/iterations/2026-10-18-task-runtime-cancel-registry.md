# 任务取消能力与处理器统一注册

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`task-runtime 0.3.0`（进行中）

## 目标

任务行的“可取消”声明、界面入口与实际取消分派分散在不同判断中，新增后台任务时可能出现界面提供取消按钮、实际却没有处理器的情况。将取消能力、实例要求和处理器收敛到同一注册表，并让任务行、单项取消和批量取消复用该来源。

## 修改

- 新增静态取消注册表，将任务 key、是否必须提供实例 ID、处理函数放在同一条记录中。
- 框架工具通过框架工具目录动态匹配一个通用注册项，避免逐个复制工具清单；HTTP 请求和网络诊断等不支持取消的任务不会因此获得取消入口。
- 单项取消在派发前校验必需的实例 ID；缺少 ID 时提示并停止派发。
- 任务列表按钮、批量取消候选和确认后的实时复核共用行级注册表校验。仍同时要求任务报告 `cancellable` 且状态为运行中。
- 为各类任务提取显式取消处理函数；继续由任务自身的 generation/实例校验拒绝过期请求。
- `docs/tools.json`、`docs/tool-versions.md` 与产品路线图登记 `task-runtime 0.3.0`。统一 RunId、持久化作业恢复、跨页面隔离、统一暂停与结果接力仍未完成。

## 验证

- `cargo test --offline --locked --lib cancellation_tests`：4 项通过，其中 2 项覆盖取消注册表及任务行路由；另外 2 项为同名 MCP OAuth 取消测试。
- `cargo test --offline --locked`：单元测试 1,055 项通过、32 项忽略；集成测试 52 项通过、9 项忽略；0 失败。
- `cargo fmt --all -- --check`、默认与 `ui-preview` 两套 `cargo clippy --offline --locked --all-targets -- -D warnings` 均通过。
- `python scripts/sync_tools.py` 重新生成工具目录与 Studio 插件迁移映射；随后两个 `--check` 均通过，目录为 92 项已实现、62 项规划/进行中，插件映射包含 154 项目录工具且 0 项宣称 Host 已接受。
- `git diff --check` 通过；仅报告工作区既有 CRLF 行尾转换提示。
- `cargo build --release --offline --locked`：通过，耗时 10m 24s。生成 `target/release/ZiDevTools.exe`，41,071,616 字节，SHA-256 `5040182F6F35E52B15E14770EA3ACE187907F6EBECE32F6D996BC5F0A5644891`。

## 限制

- 注册表集中路由元数据，但不把各工具内部取消状态统一成可恢复作业协议；取消仍由各工具自己的 generation 或实例状态保证。
- 此轮不改变用户可见功能和程序版本，不生成或发布安装包。
