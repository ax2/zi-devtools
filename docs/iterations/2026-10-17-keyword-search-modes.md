# 本机关键词检索：匹配模式

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`rag-keyword-search 1.1.0`

## 修改

- 在保留默认自动模式的基础上，增加完整短语、包含所有词、包含任一词三种用户可选匹配模式。词项查询仍使用 SQLite FTS5 参数化查询，中文完整短语使用索引内的字面包含检查；本次没有启用网络或模型检索。
- 英文多词检索可放宽为任一词命中，也可收紧为完整短语。词项模式拒绝只有标点符号的输入。
- 多词查询未作为连续短语出现时，结果预览围绕最早命中的查询词展示上下文；完整短语优先以整句定位。
- 更新 `docs/tools.json`、生成的 `docs/tools.md` 与 `docs/tool-versions.md`，工具版本从 `1.0.0` 升到 `1.1.0`。

## 验证

- `cargo test --offline --locked --features ui-preview knowledge_ -- --nocapture`：知识库相关测试通过，覆盖四种模式、来源过滤、结果快照变化、短语连续性、标点拒绝及预览上下文定位。
- `cargo test --offline --locked --features ui-preview --quiet`：库测试 1016 通过、27 忽略、0 失败；可运行集成测试通过。忽略项包含需要本机 Ollama 模型或直接访问公网的场景。
- `cargo fmt --all`、严格 all-target Clippy、`python scripts/sync_tools.py --check` 和 `python scripts/sync_studio_plugin_map.py --check` 通过。
- 通过真实 eframe 渲染器查看了 1600×1125 深色、浅色页面；下拉框和结果卡片布局清晰。截图保存在 `screenshots/knowledge-search-1.1.0-20261017/`。

## 边界

默认行为不变；完整短语以 SQLite 的 `lower()` 和 `instr()` 做大小写折叠后的包含匹配，并非自然语言分词。中文复杂问句仍没有分词或向量召回。没有重建 release，也没有发布程序或 Studio 插件包。
