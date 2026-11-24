# 两表合并（独立格式）

两张表分别选择 CSV、TSV 或 JSON，每张最多 8 KiB。支持左连接、内连接、按同名列追加和按行号拼列。JSON 支持对象数组与 `{ "headers": [], "rows": [] }` 列式结构。列式输入保持显式列序，数组按现有工作台规则推断列。CSV 和 TSV 字段保持为文本，JSON 保留有效的 64 位整数，其他数字沿用原工作台的 JSON 数值语义。连接按值和类型匹配，CSV 中的 `1` 与 JSON 数字 `1` 不匹配；要关联该文本，请在 JSON 中写 `"1"`。不选连接列时使用对应表的首列。两侧都先按输入名称精确匹配，找不到时再去掉首尾空白。

结果包含合并报告、列式表格和 CSV 字段。插件无文件、网络或模型权限。Studio 在用户选择文件后提供草稿，在用户主动执行后显示结果。选址和确认另存由 Studio 负责。原有 Compute 包可继续安装，仍按既有同格式表格合同运行。本候选保留原 48 KiB 传输与 Worker 限额。

## 构建和固定向量

```powershell
cargo build --locked --profile plugin --target wasm32-wasip1 -p zi-table-merge-wasi
cargo build --locked -p zi-table-merge-wasi
```

用独立原生适配器、WASI 模块和已安装发行物中的 Windows 生产 Worker 运行 `scripts/verify_table_merge_plugin.py`。将实际验证生成的收据与相同模块交给 `scripts/package_table_merge_plugin.py`，生成新的未签名候选目录。Host 和应用验收在 Studio #28 单独记录。
