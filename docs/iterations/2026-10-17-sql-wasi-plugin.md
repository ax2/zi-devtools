# 2026-10-17 SQL 格式化 WASI 插件候选

## 目标

复用独立 SQL 格式化器的纯 Rust 核心，为文本插件增加可跨应用调用的 SQL 格式化能力，同时明确桌面 UI 完整能力与插件候选子集的差异。

## 修改

- 将 `sqlformat` 算法与方言字面量保护逻辑移入 `zi-text-core::sql`；桌面模块保留薄 API 包装。
- 增加 `devtools.text.sql.format`，限定通用方言和默认布局。
- View 加入 SQL 操作、示例和能力桥接；候选包、fixture、迁移表和文档版本同步至 0.3.0。
- 候选仍为 unsigned；Studio Host 安装、Pi、权限、生命周期和 fuel 未验收。

## 验证

`cargo fmt --check`、`cargo test`（954通过、27忽略）、`cargo clippy --all-targets -- -D warnings`、`cargo build --release` 通过（5分50秒）。native/WASI 43条逐字节对照通过，最大结果47,191字节；两主题浏览器View通过；候选15个文件757,289字节，逐项源文件/包文件摘要及SQL manifest/catalog绑定通过。`sync_tools.py --check` 与 `sync_studio_plugin_map.py --check` 通过，154项迁移映射保留0项Host接受。

## 遗留

插件不包含桌面端 PostgreSQL/MySQL/SQLite/SQL Server 方言选择、缩进/关键字选项；不做 SQL AST/语义校验或数据库执行。Node WASI 仅作功能适配验证，不能代表 Studio Host 验收。
