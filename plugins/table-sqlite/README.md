# 表格另存 SQLite

在 Studio 工具中心搜索「表格另存 SQLite」。选择文件，确认本次提供，再选择 CSV、TSV 或 JSON 格式。可填一个表名，留空时为 `data`。执行后检查行列数，确认另存新的 `.sqlite` 文件。源文件保持原样。

支持带可选 BOM 的 UTF-8。CSV 以逗号分隔，TSV 以制表符分隔，都以首行为表头。JSON 支持对象数组和 `headers`、`rows` 组成的列结构；缺失字段为 NULL。CSV 和 TSV 保留字符串；JSON 的 NULL、`i64` 整数、浮点和文本保留，布尔值转为 0 或 1，超出有符号范围的 `u64` 写为十进制文本，数组或对象写为 JSON 文本。空表头、重名或仅大小写不同的列名明确拒绝。

每次转换一份最多 16 KiB、128 行和 16 列的表格，生成一个最多 128 KiB 的 SQLite 单表文件。表名和列名不超过 512 个 UTF-8 字节。输出前执行事务、行数比对和 SQLite 完整性检查。使用既有 Worker 限额，仅申请 `resource.read`。原生工作台中的筛选或选区、工作实例持久化以及大表范围继续单列。

## 可复现构建

使用项目已锁定的 `libsqlite3-sys 0.30.1` 中的 `sqlite3.c` 和 Windows x64 官方 Zig 0.15.2。便携编译器 ZIP 的 SHA-256 为 `3a0ed1e8799a2f8ce2a6e6290a9ff22e6906f8227865911fb7ddedc3cc14cb0c`。运行 `python scripts/build_table_sqlite_wasi.py --zig <zig.exe> --sqlite-source <sqlite3.c> --work <可写构建目录>`，复用已有 Cargo 缓存及 `wasm32-wasip1` 目标。编译器仅用于构建，插件自带静态 SQLite。SQLite 在 Worker 里使用内存数据库，禁用扩展加载、临时文件和内存映射。

检查脚本 `scripts/verify_table_sqlite_plugin.py` 用原生适配器、生产 Windows Worker、独立 SQLite 读取器验证文件完整性、列名和数值类型。验证通过后可用 `scripts/package_table_sqlite_plugin.py` 冻结包和外部目录校验收据。
