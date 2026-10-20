# 资料库结构化表格互通（0.7.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：类型化资料与结果 `0.7.0`

## 目标与修改

流程检查器已能把类型化表格送往统一接力面板，但资料库此前把它标记为普通 JSON。保存后再检索或重开时，行列结构仍在正文中，却失去“结构化表格”的类型语义。

资料库新增 `table` 类型，接收表格材料时保留列顺序、单元格JSON值类型及来源工具版本。类型可在资料列表筛选和编辑器类型菜单中选择；搜索仍复用当前版本正文的后台搜索。保存前使用数据工作台相同的 Dataset 结构校验；无效JSON、列名、重复列、行宽和行列上限不符时拒绝保存。SQLite schema 不变，现有数据库和其它四类资料无需迁移。接收只建立未保存草稿，仍需用户明确保存。

随后审查发现回程路径曾把资料库中的表格当作普通文本发送。现改为从结构化资料提取经过验证的 Dataset 快照，通过 `Material::Table` 返回数据工作台或资料库；无效编辑在发送前拒绝，原保存版本与当前草稿均不因发送而写入。来源标记继续沿接力面板展示。

## 验证

- `cargo test --offline --locked structured_table_handoff -- --nocapture`：2 项通过，覆盖表格进入资料库草稿及保存/重开/搜索类型保持。
- `cargo test --offline --locked artifact_table_snapshot_returns_to_data_workbench_as_typed_table -- --nocapture` 与 `cargo test --offline --locked invalid_edited_table_cannot_be_exported_as_a_typed_handoff -- --nocapture`：各1项通过，覆盖回程接力和无效草稿拒绝。
- 全量 `ui-preview` 回归包含资料库全部13项测试和回程接力检查。
- `cargo test --offline --locked --features ui-preview`：989 项通过、0 失败、36 项因本机服务/交互设备/外部 SDK 等条件忽略。
- `cargo clippy --offline --locked --all-targets -- -D warnings`：通过。
- `rustfmt --edition 2024 --check src/artifact_hub.rs src/app/handoff.rs`：通过。
- `python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check`：通过；已同步工具目录生成文件。
- `git diff --check`：通过。Git 对工作区现有 LF 行结束符显示 CRLF 提示，无空白错误。

## 边界

结构化表格目前以 UTF-8 JSON Dataset 载荷保存，沿用单版16 MiB、库总内容256 MiB限制；不是流式大型表格存储。接收不会保存资料、导出文件、运行流程或自动接触其它工具。此开发变更未递增应用版本，也没有发布安装包。
