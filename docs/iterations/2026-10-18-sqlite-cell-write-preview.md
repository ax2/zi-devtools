# SQLite 浏览器单格写入预览

日期：2026-10-18

## 目标

让 SQLite 浏览器具备用户明确授权后的本地数据修改能力，同时限定为当前页单格操作。不得执行任意 SQL；预览和提交都要绑定到同一普通表、列、主键行和旧值。

## 现状与实现

- SQLite 浏览器原先以 `SQLITE_OPEN_READ_ONLY` 和 `query_only` 打开文件，只能预览及导出当前页。数据工作台的 SQLite 导出只创建新文件，不修改源数据库。
- 新增单元格选择和“编辑选中单元格”对话框，输入仅支持 JSON 标量；写入入口明确呈现行列和当前值。
- 写入预览重查普通表 schema 和主键，在 `BEGIN` 事务里执行单条参数化 `UPDATE`，读取 SQLite 实际存储值后回滚。用户审阅旧值/新值并再次确认后，打开数据库、检查 schema version 和旧值，以事务执行同一更新并提交。
- 只允许更新非主键列；目标表必须有主键，不能是 view 或 virtual table。含触发器或数据库中任意外键的文件拒绝写入，以免预览遗漏级联或触发器副作用。BLOB、截断、非 UTF-8、非有限数字、对象/数组和超界整数不接受。
- 每条更新使用绑定值和转义标识符；数据冲突或 schema 变化时不覆盖，刷新当前页显示实际状态。事务失败通过 SQLite 回滚。

## 验证

- `cargo test --offline --locked sqlite_browser::write::tests --no-fail-fast`：4 项通过，覆盖预览不持久化、确认后仅改单格、并发旧值冲突、无主键/触发器/外键拒绝和输入标量边界。
- 当前页刷新由真实应用 UI 入口调用；渲染截图和人工桌面可用性尚待补验。

## 已知边界

当前只支持单个单元格，不能批量改值、编辑主键、添加/删除行、执行任意 SQL 或编辑关联数据库。事务预览会短暂取得写锁，并可能更新 WAL 辅助文件；超时预算与繁忙锁限制沿用 SQLite 配置。跨程序并发写入通过文件身份、旧值比较和 SQLite 锁检测；本机撤销历史不持久化，未提供数据库备份。

工具目录：SQLite 浏览器从 `1.1.0` 升至 `1.2.0`；“授权写入与变更预览”从规划 `0.0.0` 进入开发中 `0.1.0`。

补充验收：`cargo run --offline --locked --features ui-preview --example capture_ui -- target/sqlite-ui-capture 167` 通过，实际 eframe 渲染输出 1600×1125 的 SQLite 浏览器亮色与暗色页面截图；检查亮色截图确认单格选择和编辑入口均可见、无控件裁切。截图位于忽略的 `target/sqlite-ui-capture/`，没有写入发布资料。

补充构建记录：`cargo build --offline --locked --release` 通过，耗时 10m58s；生成 `target/release/ZiDevTools.exe`（41,213,952 bytes），程序开发版号仍为 `0.82.0-dev.105`。构建后 E 盘可用空间 7.811 GiB；没有清理 Cargo/Release 缓存或发布新版本。
