# 后台任务中心搜索与筛选（1.11.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：后台任务中心 `1.11.0`（开发中）

## 修改

- 增加本次运行内名称搜索，支持多个空格分隔词同时匹配任务标题和实例名；搜索长度限制为256字符，搜索状态仅留在当前应用内存。
- 增加“全部、运行中、失败、已结束”筛选及匹配数量。运行任务继续置顶；筛选不会清理或改变后台任务。
- 搜索只匹配任务标题和实例名称，不检查任务摘要、参数、正文或凭据，避免搜索路径进入其他索引。

## 验证

- `tasks::tests::filters_and_search_match_task_labels_without_searching_summaries`：通过，覆盖实例搜索、多个状态筛选和敏感摘要不参与搜索。
- `cargo test --offline --locked`：全套测试成功退出；外部模型/网络条件测试按既有配置忽略。
- `cargo clippy --offline --all-targets --locked -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 和 `git diff --check` 通过。
- `capture_ui` 的暗色与浅色真实 eframe 渲染均为 1600×1125；已检查搜索栏、四种筛选、计数和任务卡片层级。截图见 [暗色](../../screenshots/task-center-search-20261017/tasks-dark.png) / [浅色](../../screenshots/task-center-search-20261017/tasks-light.png)。这是渲染检查，不代表物理鼠标操作验收。
