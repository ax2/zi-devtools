# ZiDevTools Text 0.4.2 unsigned candidate

该包包含 JSON 格式化、压缩和校验、SQL 格式化、Base64 编解码及 SHA-256。Rust 计算核心与独立 ZiDevTools 共用。包自带 HTML View 使用通用 SQL 默认值；Studio 工具目录可根据 SQL 能力输入声明提供额外选项。

SQL 仍接收最多 8192 UTF-8 字节，结果仍是 `{text}`。可选字段 `dialect` 支持 `generic`、`postgresql`、`mysql`、`sqlite`、`sqlserver`，`indent` 支持 `twoSpaces`、`fourSpaces`、`tabs`，`keywordCase` 支持 `uppercase`、`lowercase`、`preserve`。只提供 `text` 时保留通用方言、四空格、大写关键字。请求和结果上限 48 KiB，结构密集的 SQL 可能达到原有的 10M fuel 上限。格式化无需连接或执行数据库。其他六项能力继续只接收文本。

候选保留原来的七个 capability ID 和 `tool.directory.v1@1.0.0-preview.1`，0.4.1 冻结包保留。候选目录无签名；首次安装、升级、回滚、撤权、Native View 和 Pi 权限由接收方验收。在 Studio 授权 Pi 注册为可选操作。独立应用配置与插件数据隔离。

复用现有 Cargo target 构建 `zi-text-wasi --profile plugin --target wasm32-wasip1 --locked` 后，使用 `scripts/verify_text_wasi.py` 验证原生/WASI 对照，使用 `scripts/verify_diagnostics_fuel.py` 和 `scripts/verify_diagnostics_cold.py` 检查原有预算，使用 `scripts/verify_text_plugin_view.py` 验证通用 Text View。最后由 `scripts/package_text_plugin.py` 严格检查回执并生成目录清单。候选内 `verification/README.md` 说明如何重放相同输入。
