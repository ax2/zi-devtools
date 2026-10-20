# 流程中间结果另存（可组合数据管线 0.19.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：可组合数据管线 `0.19.0`（开发中）

## 修改

- 步骤完整结果检查器新增“另存此步结果为 CSV / SQLite”。选择后关闭检查器并打开现有结果另存面板，保留“第 N 步结果”标签。
- 复用同一套目标选择、CSV 公式防护、SQLite 类型映射、32 MiB 上限、后台任务、取消和新文件无覆盖发布；确认框展示步骤标签、行列和样本。
- 审核绑定原始表、流程定义、精确步骤结果与目标。返回编辑不会落盘；来源或定义变化会使旧审核失效。另存不应用原表、不保存写入授权。
- `docs/tools.json` 中管线版本提升至 `0.19.0`；应用版本仍为 `0.82.0-dev.105`。

## 验证

- `workbench::workflow_output::tests::intermediate_step_result_can_be_saved_under_its_own_label_without_applying` 与 `intermediate_step_sqlite_output_uses_the_selected_snapshot`：验证步骤标签、行数和精确结果绑定；预览不创建 CSV/SQLite，确认后的 SQLite 内容只含选定步骤数据，原表保留。
- `workflow_step_output_ui` 原生 eframe 窗口：暗色与浅色各真实合成点击检查器另存入口、保存预览和返回按钮；断言步骤结果审核可见，返回后目标仍不存在。截图为 980×640 视口，见[暗色](../../screenshots/workflow-step-output-20261017/workflow-step-output-dark.png)与[浅色](../../screenshots/workflow-step-output-20261017/workflow-step-output-light.png)。这不是物理鼠标或触屏验收。
- `cargo test --offline --locked`：库测试 932 通过、27 忽略、0 失败；所有启用的集成测试通过。
- `cargo clippy --offline --all-targets --features ui-preview --locked -- -D warnings` 通过；格式、工具目录同步、Studio 插件映射同步和 `git diff --check` 通过。

## 遗留

只支持将单个步骤快照另存为 CSV 或 SQLite；尚无多输出材料端口，也不把中间结果自动送入其他工具。物理鼠标、触屏和极窄窗口未覆盖。
