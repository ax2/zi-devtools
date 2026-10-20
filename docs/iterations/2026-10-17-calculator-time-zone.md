# 计算器时区换算接入（0.15.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变）\
工具版本：全能计算器 `0.15.0`（开发中）

## 目标

让时区换算可从全能计算器的工作区标签直接访问，同时继续提供工具库中的独立时区工作台；两种入口共用一个本机运行态，避免复制转换逻辑和分叉输入。

## 修改

- 在计算器标签栏增加“时区换算”，嵌入现有 IANA 时区工作台 UI；时区工作台页面仍使用同一个 `App::time_zone` 状态。
- 进入该标签后隐藏 RAD/DEG 控件与计算器工作表管理，避免把无关设置混入时区操作。
- 时区草稿不纳入计算器工作表 schema；改变当前工作区不使计算工作表变脏，确认恢复计算工作表时返回其已保存工作区。
- 计算器工具版本升为0.15.0，目录、生成的工具清单、版本说明、路线和 README 同步。

## 验证

- `calculator::ui::tests::time_zone_workspace_stays_outside_calculation_sheet_snapshots`：通过，检查时区标签不改变计算表快照，恢复计算表后重置工作区选择。
- `cargo test --offline --locked calculator::ui::tests`：2项通过。
- `capture_ui` 实际 eframe 渲染暗色/浅色各1600×1125；能看到计算器选中态、来源本地时间、来源时区、两个目的时区、UTC换算及复制操作。截图：
  - `../../screenshots/calculator-time-zone-20261017/calculator-time-zone-dark.png`，SHA-256 `CBCB8B591BC7F95487CFA95D902A4E0E732194889B66C8B0CA7D48ADE01C4A7D`
  - `../../screenshots/calculator-time-zone-20261017/calculator-time-zone-light.png`，SHA-256 `F6C2F58F477321877C22D8E5D7B657D226522C265849EE9FD8512079A3D59585`
- `cargo test --offline --locked`：主库1,028通过、31忽略、0失败；集成测试通过，环境依赖项按配置忽略。
- `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check`、`git diff --check` 均通过；`cargo build --offline --locked --release` 成功，`target/release/ZiDevTools.exe` 为40,777,216字节。
- Release 构建本轮运行约7分钟；未更改程序版本，未清理或删除构建缓存。

## 边界

- 转换能力及夏令时边界由既有时区工作台和时区模块测试覆盖；本次只接入已有功能，没有扩展 IANA 规则范围。
- 仍不写入计算表、用户配置或外部文件；关闭程序后草稿消失。
- 数据库为随应用提供的 IANA tzdb；离线可用，但不保证涵盖数据库发布后发生的政策更改。
