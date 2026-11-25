# SQLite 浏览器

在 Studio 工具中心搜索「SQLite 浏览器」，选择并确认一份本机 SQLite 文件。留空表名并执行可查看目录，填写目录中的普通表名可查看数据页。每页最多 16 项或 16 行，页码 1–16。可勾选「生成此页 CSV」，执行后按 Studio 的确认流程保存新的 CSV 文件。源数据库保持原样。

支持最多 128 KiB、UTF-8 编码、可独立打开的静态 SQLite 文件。最多检查 64 个表和视图定义，普通表最多 16 列。视图、虚拟表、含生成列的表以及依赖 WAL 的文件不开放数据读取。目录也会排除以该库虚拟表名称加下划线开头的辅助表，同前缀命名的普通表同样受限。对长文本和 BLOB 仅展示有界预览，文字受每个单元格 512 字节和每页共享 6 KiB 上限约束。无行、含截断、二进制或无法完整表示的页面不生成 CSV。正常页面的 CSV 沿用原生工具的行序、字段内容和电子表格公式前缀保护。

插件仅申请 `resource.read`，用内存只读连接处理当次确认的快照。正式范围限于目录、普通表分页和当前页 CSV。数据库备份、编辑、撤销重做、任意 SQL 和外部扩展由其他独立功能处理。

## 构建和验证

用同一锁定的 `libsqlite3-sys 0.30.1` 及 Windows 便携 Zig 0.15.2。构建方法与表格另存 SQLite 共用。调用 `python scripts/build_table_sqlite_wasi.py --package zi-sqlite-browser-wasi --zig <zig.exe> --sqlite-source <sqlite3.c> --work <可写缓存目录>`。脚本检查 C 源摘要并复用已有 Cargo `wasm32-wasip1` 缓存及预算。没有编译器运行时依赖。

`scripts/verify_sqlite_browser_plugin.py` 通过实际 Windows Worker 和原生适配器比较合成 SQLite 的目录、页和 CSV 逐字节结果；`scripts/package_sqlite_browser_plugin.py` 将完整已通过用例收入冻结包和外部校验收据。
