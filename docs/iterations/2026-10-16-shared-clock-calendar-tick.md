# 2026-10-16 时钟与日程统一时间快照

状态：开发版架构迭代完成；全功能时钟独立工具版本 `0.5.1`，万年历与日程独立工具版本 `1.4.1`；程序版本保持 `0.82.0-dev.105`，未发布。

## 目标与实现

时钟工作台和万年历/日程过去在主循环中分别读取 `Instant`、UTC 或本地时间。若更新发生在日历秒边界附近，各自采样可能错开。本轮增加应用级 `TickSnapshot`，每个 UI 更新循环只采样一次单调时钟和 UTC；本地日程时间从该 UTC 快照转换得出。

时钟检查点保存和时钟提醒共用该单调/UTC快照；日程后台提醒轮询新增 `poll_at`，接收同一单调时刻与派生本地时间。原有 `poll(ctx)` 保留给既有调用和测试。使用饱和单调时长比较，避免更早快照导致 panic；异步读写完成后重置时钟时有安全回退。

本轮统一的是**应用循环时间采样**，不是完整提醒调度或数据模型。Clock alarm 与 calendar event 仍各自保存、展开重复、记录稍后/知晓和显示提醒，退出程序后的系统提醒也未实现。

## 验证

- 新增 `TickSnapshot` 测试，验证派生的本地时间与该 UTC 快照代表同一时刻。
- 新增日程定向测试，固定注入单调时钟和本地时间，验证提醒按主循环提供的同一时间判定到期。
- `cargo test --lib app::scheduler::tests`：1 项通过。
- `cargo test --lib planner::tests`：37 项通过。
- `cargo test --lib clock::windows::tests`：4 项通过。
- 全量 `cargo test --features ui-preview`：库测试 913 项通过、27 项按需忽略；全部集成测试通过。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 和 `git diff --check` 通过。
- Windows 原生 `clock-window-smoke` 重跑通过：验证共享应用循环下计时器继续运行，原生小窗的置顶、全屏、回到工作台、关闭后重开与尺寸恢复仍正常。首次运行发现系统布局可能对 720×480 请求产生小于 1 逻辑点的浮点取整差异，已将测试夹具调整为 1 点容差并复验。

## 后续

下一阶段再评估统一的 occurrence ID、提醒队列、知晓/稍后状态与本机数据存储迁移。必须保留现有日程数据库备份和多窗口提醒 lease 行为；在迁移规则、恢复行为及应用完全退出时的策略完成前，不能宣称两类提醒已统一。
