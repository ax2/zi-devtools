# 流程步骤结果类型化接力（可组合数据管线 0.20.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：可组合数据管线 `0.20.0`（开发中）

## 修改

- 最终流程预览与任一步完整结果检查器可通过统一“发送结果到工具”面板接力，目标保留类型限制，确认后创建独立数据实例。
- 接力材料同时携带已登记的 `pipeline` 工具版本元数据与人类可读来源标签；新实例名称保留来源版本和步骤标签，不再误称为文本流程表格。
- 检查、筛选、复制、发起接力和取消接力都不应用原表、不替换最终预览，也不保存实例；接收只导入表格，不运行目标操作。
- 可组合数据管线独立版本由 `0.19.0` 升至 `0.20.0`；程序版本维持 `0.82.0-dev.105`。

## 验证

- `app::handoff::tests::typed_table_handoff_preserves_cells_and_opens_a_new_data_instance`：类型、列序、原实例和来源标签均保留。
- `workbench::workflow_ui::tests::selected_step_can_be_queued_for_typed_handoff_without_applying_or_replacing_results`：步骤快照可一次性接力，原输入与最终预览不变。
- `workflow_step_handoff_ui` 原生 eframe 窗口暗/浅主题各通过合成点击：从步骤检查器发送、查看含工具版本的类型化预览、明确确认接收，再检查新实例内容与来源名称以及原流程保留。代表截图为 1365×900，见[暗色接力审核](../../screenshots/workflow-step-handoff-20261017/workflow-step-handoff-dark-1.png)、[浅色接力审核](../../screenshots/workflow-step-handoff-20261017/workflow-step-handoff-light-1.png)、[接收后的工作台](../../screenshots/workflow-step-handoff-20261017/workflow-step-handoff-light-2.png)。合成事件不等同于物理鼠标、键盘或触屏验收。
- `cargo test --offline --locked --features ui-preview`：962 项库测试中 935 通过、27 忽略、0 失败；所有可运行集成测试通过，依赖外部服务的用例按标记忽略。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check` 均通过。

## 边界

当前仅接力表格，并沿用共享面板现有目标能力。不会自动写入磁盘或自动运行目标。文件、图片、MCP 和 Agent 仍未成为可组合管线中的通用动作端口；多输出、分支流程和范围授权写入仍需单独设计与验收。
