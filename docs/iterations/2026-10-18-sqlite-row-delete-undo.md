# SQLite 单行删除与会话内撤销

日期：2026-10-18

## 目标

在现有单元格更新、逐次确认和会话内撤销/重做基础上，增加选中单行的安全删除，并保持撤销/重做模型统一。

## 实现

- 选中当前页某行后可请求删除预览。预览在 `IMMEDIATE` 事务中核对数据库与整行数据、执行删除后回滚；确认后再次核对文件身份、schema、主键和行内容，再提交单行删除。
- 将单格与行变更统一放入最多20步会话内撤销/重做历史。撤销会在主键未被占用时恢复完整行，重做再次删除；每次均需要用户确认。
- 对普通 rowid 表同时快照并验证原始内部 rowid；`WITHOUT ROWID` 表不访问隐藏 rowid，按主键恢复。所有 rowid 别名都被实际列遮蔽时拒绝删除。
- 仅允许可精确恢复的普通列和值；生成列/隐藏列、BLOB、截断或其他不可表示数据、触发器、外键、虚拟表及无主键表拒绝操作。
- 行快照仅暂存在应用当前进程，不落盘、不上传；退出应用后无法恢复。该功能不是数据库备份。
- SQLite 浏览器版本升至 `1.4.0`；授权写入与变更预览版本升至 `0.3.0`。开发版程序版本未变。
- 复核单元格写入和整行删除的提交后错误路径：事务已提交但逆向快照刷新失败时，返回独立的“已提交、历史不可用”状态，界面不会误报“未写入”；正常情况下仍刷新文件身份以支持撤销。

## 验证

- `cargo test --offline --locked sqlite_browser::`：22 项 SQLite 浏览/转移/写入测试通过，覆盖整行预览回滚、确认删除、撤销恢复、重做删除、外部变更拒绝、重用主键拒绝、rowid 保留、WITHOUT ROWID、不可精确恢复的值，以及单元格和行删除在模拟“提交后快照刷新失败”时仍明确报告已提交。
- `cargo test --offline --locked --quiet`：1,074 项通过、32 项因本机服务/模型/交互条件忽略、0 失败。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets -- -D warnings`、`python scripts/sync_tools.py --check` 与 `git diff --check` 通过。
- 故障注入辅助函数抽取后的 `cargo build --offline --locked --release` 通过，耗时 12m48s；最终 `target/release/ZiDevTools.exe` 为 41,295,360 bytes。构建后 E 盘可用空间为 8,009,482,240 bytes（约 7.46 GiB），没有清理缓存。程序开发版仍是 `0.82.0-dev.105`，未创建正式发布包。
- `capture_ui` 真实 eframe 截图 light/dark 各 1600×1125；目视检查工具栏中的删除按钮可见且未裁切，行选择可辨认。
- 未清理 Cargo 增量缓存、源码、发布目录或任何项目外依赖。
- 截图证据保存在 `screenshots/sqlite-row-delete-sqlite-browser-light.png` 与 `screenshots/sqlite-row-delete-sqlite-browser-dark.png`。

## 遗留限制

依赖 file material 身份复核；受支持的值、schema 和数据库限制有意保守。跨重启恢复、批量删除、持久化备份及一般数据库恢复流程均不在本轮范围。
