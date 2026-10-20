# SQL 格式化响应式输入输出布局（0.3.1）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变）\
工具版本：SQL 格式化 `0.3.1`

## 修改

首次真实窗口渲染发现，SQL 格式化结果出现在输入框下方，在默认窗口高度下完整对照输入和结果需要滚动。宽窗口现在并排显示输入与结果，窄窗口继续纵向排列；不改变方言、格式化规则、复制结果和交换输入/输出行为。工具目录与说明同步到 `0.3.1`，程序版本不变。

## 界面检查

- 使用真实 eframe `capture_ui` 预览场景 536/537，加载 PostgreSQL 查询示例并验证格式化结果包含 `LEFT JOIN`。
- 浅色和深色截图均为 1600×1125；输入、方言、缩进、关键字大小写、格式化按钮、完整输出及复制/交换入口均在同一视口内可见。
- 截图：[浅色](../../screenshots/sql-formatter-ui-20261017/sql-formatter-light.png)、[深色](../../screenshots/sql-formatter-ui-20261017/sql-formatter-dark.png)。这是合成预览窗口的渲染检查，不等同物理鼠标验收。

## 验证

- `cargo fmt --check`。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\\ui-preview-sql-formatter-20261017 536 single` 与场景 537 均通过。
- `cargo test`：961 项通过、0 失败、27 项环境相关测试忽略；全部集成测试通过，包括本机服务生命周期 13 项。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 与 `python scripts/sync_studio_plugin_map.py --check` 通过。
- `cargo build --release` 通过，耗时 5 分 41 秒；复用现有 Cargo 缓存，程序版本仍为 `0.82.0-dev.105`。

## 范围限制

窄窗口保留纵向滚动；SQL 方言词法能力及非完整 AST/语义校验的限制不变。格式化不会连接数据库或执行语句。
