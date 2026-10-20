# 2026-10-16 SQL 格式化迭代

状态：开发中，当前独立工具版本 0.3.0；程序版本保持 `0.82.0-dev.105`。

## 目标与实现

推进工具目录中规划的 SQL 格式化工具，支持通用 SQL、PostgreSQL、MySQL、SQLite、SQL Server / T-SQL 选项。本轮将首轮手写词法换行规则替换为 `sqlformat` 库的布局格式化，并增加方言特定字面量保护，不执行 SQL、不连接数据库。

实现位于 `src/sql_formatter.rs`，由通用工具注册、工具搜索目录及 SQL 方言 ComboBox 接入。SQL 工具草稿保留选择的方言。`docs/tools.json` 将其从规划更新为开发中 0.1.0，`docs/sql-formatter.md` 记录使用范围和限制；同步生成 README 与工具目录，不递增程序版本号。

## 首轮验证记录（0.1.0）

- SQL 定向测试：5 项通过，覆盖常见子句、JOIN 修饰词、PostgreSQL dollar quote、MySQL 反引号、SQL Server 方括号、注释/字符串保护、无效方言和未闭合引号。
- 工具选择草稿测试：切换工具后保留 MySQL 方言。
- 全量 `cargo test --quiet`：主库 894 项通过、27 项忽略；全部集成测试通过，包括服务生命周期 13 项。
- `cargo clippy --all-targets -- -D warnings`（最终测试调整后复跑）、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check` 和 `git diff --check` 通过。
- Debug 桌面程序以隔离配置和 `--tool sql-format --no-restore` 成功启动并结束。当前 CUA 会话未枚举出原生应用窗口，未取得 UI 截图验收。
- `cargo build --release` 通过；产物 `target/release/ZiDevTools.exe`，大小 33,943,552 bytes，构建时间 2026-10-16 09:12 +08:00。

## 当前限制与后续

当前实现是空白与布局格式化，不是 SQL AST 或语义校验器。复杂嵌套查询、各数据库方言的完整差异及存储过程仍可能需要针对性验收；格式化成功不能说明 SQL 语义有效。`sqlformat` 0.5.0 为纯 Rust 库，包内源码约 210 kB，运行依赖为 `unicode_categories` 和 `winnow`；不启用可选调试依赖。其方言 API 只有通用、PostgreSQL、SQL Server 三种模式，因此 MySQL/SQLite UI 选择映射到通用模式。

## 2026-10-16 二次迭代（0.2.0）

用 `sqlformat = 0.5.0` 替换自维护的 SQL tokenizer 和关键词/换行规则，减少本地边界处理错误，并补上 CTE、JOIN、CASE、窗口函数和参数占位符保留用例。应用传入 `QueryParams::None`，确保问号、冒号、美元符号和 at-sign 占位符不被插值。采用 4 空格缩进、保留关键字大小写以外的文本、关键字大写、控制顶层 JOIN 布局。

该库只格式化文本，不校验语法或语义。版本历史和工具说明更新到 0.2.0；程序版本保持 `0.82.0-dev.105`。后续验证结果追加在本节末尾。

### 二次迭代验证（0.2.0）

- `cargo test --lib sql_formatter`：7 项通过，覆盖 CTE/JOIN/CASE/窗口函数、注释与占位符、PostgreSQL dollar quote、SQL Server/SQLite 方括号标识符、非法方言及空输入。
- `cargo test --quiet`：主库 895 项通过、27 项忽略；全部集成测试通过，包括服务生命周期 13 项。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check` 和 `git diff --check` 通过。
- 未构建 release、未发布；本轮没有进行 UI 截图验收，方言差异与排版结果仍需后续在真实数据库语句集上持续扩展。

全量测试首次运行中，服务取消夹具因 2 秒接收窗口过短出现一次零请求失败；单独复跑通过。夹具改为等待取消标志或 10 秒上限，并将断言改为检查并发请求上限、部分结果及健康状态，不再要求 TCP 接收数与完成结果数相等。

## 三次迭代（0.3.0）

增加 2/4 空格与 Tab 缩进、关键字大写/小写/保留输入选项。默认保持 4 空格和关键字大写。偏好存放在当前工具草稿中，离开再返回 SQL 工具时保留；更改选项会清除旧格式化结果，避免将旧结果误认为使用了新设置。

### 三次迭代验证

- SQL 定向测试覆盖 2 空格、小写关键字、Tab、保留关键字大小写与字符串字面量不变。
- 工具状态测试覆盖用户缩进和关键字偏好在切换工具后保留，并按该偏好生成结果。
- 全量格式、工具目录同步、严格 Clippy 和测试结果待本轮实现后补录。

本轮尚未进行桌面 UI 截图验收；方言差异仍受 sqlformat 通用词法能力限制，需后续针对真实数据库语句集扩展。
