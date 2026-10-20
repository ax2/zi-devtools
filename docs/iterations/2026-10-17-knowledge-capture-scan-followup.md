# 知识采集保存后的显式来源扫描

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
独立工具版本：网页与对话收集 `1.2.0`

## 目标与实现

保存或去重成功后，知识采集页提供“扫描整个知识源…”按钮。只有用户主动点击时，程序才跳转到本地知识源页、选中刚才使用的目录，并开始一次扫描。扫描遵守现有排除规则和 500 文档、128 MiB 限制；只更新文件版本清单，不建立正文索引。索引同步仍需用户另行进入“增量知识索引”手动发起。

知识采集输入发生变化或用户新建草稿时，清除保存成功的后续操作状态，避免对过期正文发起误导性扫描。扫描目标以已选知识源的稳定 ID 查找，不依赖旧页面字符串或任意路径。

## 验证

- 定向测试将指定目录知识源选中并启动扫描，验证只对该 ID 的目录进行快照更新。
- eframe 截图：[`knowledge-capture-followup.png`](../../screenshots/knowledge-capture-followup-20261017/knowledge-capture-followup.png)，1600×1125 深色主题；按钮、目标目录与“不自动索引”状态可读。
- 完整 `cargo test --offline --locked --features ui-preview --quiet`：1004 通过、0 失败、36 忽略；忽略项依赖外部模型或网络环境。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo build --offline --locked --release`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 和 `git diff --check` 均通过。Git 对既有工作区文件显示 LF/CRLF 转换提示。

## 遗留

本轮只简化“保存 → 来源扫描”路径；扫描后仍需用户在索引页手动同步，未自动读取模型或写入向量索引。
