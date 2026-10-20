# HTTP 请求接入后台任务中心（1.12.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：后台任务中心 `1.12.0`

## 目标与修改

HTTP 请求在后台线程执行，但此前任务中心无法查看其运行状态或从中心返回发起请求的标签。

- 每个请求标签维护任务代次并向任务中心提供状态；任务卡片能打开原请求标签，标签已关闭时回到 HTTP 工作台并显示提示。
- 摘要只包含 HTTP 状态或通用失败状态及耗时，不包含 URL、请求头、请求体、响应内容或原始错误。
- 请求运行时禁止关闭该标签，避免任务中心留下无法返回的任务。HTTP 请求不支持取消，因此任务中心不显示取消动作。

## 验证

- `dev_tools::tests::http_task_snapshots_exclude_request_content_and_busy_tabs_cannot_close` 覆盖运行与完成状态、标签不可关闭、按 ID 返回指定请求标签及敏感请求字段不进入任务快照。
- `cargo test --offline --locked --features ui-preview`：库测试 943 项通过、27 项忽略；集成测试 50 项通过、9 项按 Ollama/联网/官方 SDK 条件忽略；0 项失败。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、工具目录/Studio 映射同步检查与 `git diff --check` 通过。
- eframe 暗色与浅色任务中心截图均为 1600×1125；已检查 HTTP 运行状态、固定摘要和“打开请求标签”入口可见。截图见 [暗色](../../screenshots/task-center-http-20261017/tasks-dark.png) / [浅色](../../screenshots/task-center-http-20261017/tasks-light.png)。这是真实渲染检查；按钮的物理鼠标点击验收未执行。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- <系统临时目录> 76 task-center-http-open-smoke`：合成鼠标点击“打开请求标签”后断言已切到 HTTP 页面并选中标签 `#1`，输出 `PASS HTTP request task routing`。物理鼠标验收未执行。
- 截图输出目录位于系统临时目录。一次顺序截图调用额外生成了与本轮无关的后续 UI 图片；仅将任务中心两张图归档。尝试移除其余生成 PNG 时执行策略拒绝删除，未绕过限制，临时输出仍保留。

## 边界

任务中心仅保存本次应用运行内状态，HTTP 响应和请求细节仍留在 HTTP 工作台。网络客户端当前不支持中止已运行请求，因此任务中心只提供状态和返回入口；HTTP 超时及重定向策略继续由 HTTP 工作台现有实现负责。
