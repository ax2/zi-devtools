# 批量文件重命名预览（0.1.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：批量文件重命名 `0.1.0`（开发中）

## 当前功能

- 用户通过文件选择器明确挑选最多 500 个普通文件；不递归扫描目录、不跟随文件符号链接。
- 规则仅作用于文件名主体：按字面文本查找/替换、添加前缀和后缀，扩展名保持不变。支持区分大小写开关；不支持正则、序号、目录名和大小写单独变更。
- 预览列出每一项的新旧文件名、字节数和状态。检查 Windows 保留名称/非法字符、255 UTF-16 单元上限、目标已存在、重复目标，以及预览后来源和目标变化；任何冲突都会禁用整批执行。
- 用户查看预览后明确点击“确认并执行重命名”。Windows 使用不覆盖的 `MoveFileW`；执行前复核来源身份、大小、修改时间和目标占用。出现单项移动错误时，尽力逆序恢复已完成项并保留无法恢复项目的撤销凭据。
- 最近一次成功批次可在本次运行内撤销。撤销会重新检查文件身份、大小、修改时间和原名称占用；检测到这些属性变化时拒绝恢复。关闭程序后不保留撤销记录。

文件不会在选择或生成预览时改变。当前工具不保留长期操作历史，不读写文件内容，不覆盖、删除或复制用户文件。

## 验证

- `cargo test --offline --locked --features ui-preview`：主库 965 项通过、0 失败、27 忽略；全部可运行集成测试通过。新文件重命名专项覆盖 Unicode 字面匹配边界、扩展名保留、冲突和重复目标、无效/保留名称、500 项限制、来源变更拒绝、迟到目标冲突、无覆盖执行和撤销保护。
- `cargo test --offline --locked common_task_queries_find_expected_tools_in_top_three -- --nocapture`：通过；“批量重命名”“文件名替换”能搜索到工具入口。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\\ui-preview-file-rename 538 single` 与场景 539 均通过，实际渲染深浅主题截图（1600×1125）。截图仅展示隔离临时文件名与预览；临时夹具在项目 `target` 下。
- 截图：[浅色主题](../../screenshots/file-rename-ui-20261017/file-rename-light.png)、[深色主题](../../screenshots/file-rename-ui-20261017/file-rename-dark.png)。人工检查了输入、预览行、冲突摘要和执行按钮的层级与可读性。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check` 均通过。
- `cargo build --offline --locked --release` 通过，耗时 5 分 26 秒。EXE `target/release/ZiDevTools.exe` 为 35,587,584 字节，SHA-256 `E13E2D918916C332042AAE178EC0485A8F225DA34A16C7F6B5F3E4139DAB0048`；程序文件版本保持 `0.82.0-dev.105`。
- Follow-up UI execution smoke: `cargo run --offline --locked --features ui-preview --example capture_ui -- target/file-rename-execution-smoke-final 538 file-rename-execution-smoke` passed. It clicked the actual confirmation and undo buttons, verified two renamed files retained their bytes, and verified undo restored all three original filenames and contents. Synthetic fixtures now use a UUID directory and exclusive file creation so repeated previews cannot overwrite existing files.
- Follow-up verification: `cargo test --offline --locked --features ui-preview`, `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`, default `cargo clippy --offline --locked --all-targets -- -D warnings`, `cargo fmt --all --check`, `git diff --check`, `python scripts/sync_tools.py --check`, and `python scripts/verify_tool_directory.py` passed.
- Follow-up `cargo build --release --offline --locked` passed in 12m05s. `target/release/ZiDevTools.exe` is 40,796,160 bytes, timestamp `2026-10-17 20:19:42`, SHA-256 `0CBE60DD14DB9C7479E7CC01413243804CB99D45018D5F750C17622650AB7B4E`; the program and tool versions did not change.

目录仍标为 `in-progress`：真实文件选择器交互、物理鼠标/高 DPI 和文件系统并发/ACL 条件尚未人工验收；未生成安装包或发布。

## 遗留与边界

撤销只在程序本次运行中可用；没有崩溃恢复日志。批次不是操作系统级事务，异常时会尝试回滚并报告不能恢复的文件。测试覆盖合成临时目录，不包含真实资源管理器多选、物理鼠标、高 DPI 和 ACL/杀毒软件并发占用验收。工具尚未作为 Studio 插件交付。
