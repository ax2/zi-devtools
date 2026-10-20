# 表格管线按需检查中间结果（0.18.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：可组合数据管线 `0.18.0`（开发中）

## 修改

- 可从每个步骤报告发起“查看此步完整结果”。在后台从原始输入重放到选定步骤，只保留当前检查快照，不为所有步骤累积复制表格。
- 使用现有只读表格检查器分页、筛选和复制单元格；关闭后返回未应用的最终预览。
- 检查任务进入后台任务中心，可从工作台或任务中心取消；任务活动时阻止编辑、应用、关闭实例和保存快照。
- 结果展示前复核原始表和流程定义；来源或定义变化时不打开旧结果。测试确认输入与最终预览均不被检查动作替换。

## 验证

- `workflow::tests::intermediate_preview_replays_only_through_the_selected_step`：覆盖选定步骤的结果、后续步骤故障隔离、无效步骤边界和来源保留。
- `workflow_ui::tests::selected_step_result_is_recomputed_async_without_replacing_final_preview`：覆盖后台执行、任务中心记录、结果检查、最终预览不变和输入不变。
- `workflow_ui::tests::cancelling_step_result_preview_keeps_the_full_preview_and_input`：覆盖代次取消和旧预览/输入保留。
- `workflow_ui::tests::workspace_publishes_and_cancels_step_inspection_with_its_instance_id`：覆盖任务中心实例归属、取消路由和取消后的结果保留。
- 暗/浅色 eframe 真实渲染各 1600×1125，检查模态窗、行列分页区和返回流程按钮。截图见[暗色](../../screenshots/workflow-step-inspection-20261017/dark.png)与[浅色](../../screenshots/workflow-step-inspection-20261017/light.png)。
- `cargo test --offline --locked`：全工作区测试命令成功退出；依赖本机模型或外网的既有测试按配置忽略。
- `cargo clippy --offline --all-targets --features ui-preview --locked -- -D warnings`、`cargo fmt --check`、工具目录生成/检查、Studio 插件映射生成/检查均通过。
- `git diff --check` 通过。应用版本仍为 `0.82.0-dev.105`，未发布。

## 边界

中间表按需重新计算而非缓存全部快照；检查不会自动应用或写入文件。物理鼠标/键盘验收，以及窄窗口和非标准 DPI 验收未覆盖。表格流程仍不执行文件写步骤，也未统一接入图片、插件、MCP 或 Agent。
