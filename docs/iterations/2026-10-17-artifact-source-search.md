# 资料库按来源检索（0.11.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：类型化资料与结果 `0.11.0`

## 修改

- 资料库后台搜索在名称和当前版正文之外，增加已保存来源标记匹配；来源含工具 ID 与独立版本时，可以直接搜索例如 `agent-runner@1.9.0`。
- 搜索结果每条直接显示来源，便于确认同名资料的来历；界面说明同步列出三类搜索字段。手动修改后来源会按既有规则变为“本机编辑”。
- 不搜索或打开历史版本正文；原有当前版内容校验、正文摘要预览、类型筛选、回收站隔离及异步搜索行为保持不变。

## 验证

- 定向资料库测试覆盖工具 ID/版本大小写不敏感匹配及来源命中不生成正文摘录，1 项通过。
- `cargo test --offline --locked --features ui-preview --quiet`：984 通过、27 忽略、0 失败；资料库13项回归通过。
- `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check`：通过。
- `cargo build --release --offline --locked`：通过，Release 构建耗时 6 分 49 秒。
- 深色桌面版本差异界面实际渲染检查通过：来源出现在资料列表，搜索框提示包含来源字段；既有版本差异与资料操作布局仍可读。截图位于 `target/artifact-source-search-preview/artifact-version-diff.png`。

## 限制

来源自由文本仅保留现有允许的本机编辑/接力标记；不新增按时间、版本历史、标签或 MIME 类型检索，也不读取文档正文以外的附件。
