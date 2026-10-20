# 知识源扫描后直达索引

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
独立工具版本：本地知识源管理 `1.1.0`

## 目标与实现

知识源成功扫描并将快照保存到本机配置后，来源页显示“下一步：打开增量知识索引…”。点击只切换页面，不调用索引同步。目标按钮明确提示，用户进入索引页后仍需点击“同步全部已扫描来源”；索引正文写入本机数据库仍由用户发起。

如果扫描失败或新一轮扫描开始，后续入口会隐藏，避免把旧快照误认为刚完成的扫描。扫描结果未能持久化时也不开放索引后续入口。

## 验证

- 定向临时目录测试验证成功扫描并持久化后开放后续请求，且请求需单独触发和消费。
- eframe 截图：[`knowledge-index-followup.png`](../../screenshots/knowledge-index-followup-20261017/knowledge-index-followup.png)，1600×1125 浅色主题；扫描状态、下一步按钮和“仍需明确同步”说明均可见。
- 定向临时目录测试通过；全量 `cargo test --offline --locked --features ui-preview --quiet`：1004 通过、0 失败、36 忽略。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo build --offline --locked --release`、工具目录及 Studio 插件映射同步检查、`git diff --check` 均通过。Git 对既有工作区文件提示 LF/CRLF 转换警告。

## 遗留

扫描完成不会自动更新全文或向量索引；新入口只减少页面导航步骤。
