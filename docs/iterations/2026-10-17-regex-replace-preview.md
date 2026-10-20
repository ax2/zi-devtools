# 正则替换预览工具（0.1.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：正则替换 `0.1.0`（开发中）

## 功能范围

- 使用 Rust `regex` 引擎按原文顺序做非重叠全量替换预览；支持 `$1`、`${name}`、`$$`，保留 Unicode 文本和空匹配的引擎语义。
- 输出显示本次匹配数，并明确提示“仅预览，输入未修改”。预览仅保存在当前工具状态中，不写入源文件；用户可检查、复制或交换文本。
- 输入/输出上限各2 MiB，正则表达式最多4096 UTF-8字节，替换模板最多64 KiB，最多1000次匹配。超限拒绝整次预览，不返回部分替换结果。
- ToolKind、独立草稿、启动器目录、搜索别名、工具版本目录均已接入。工作流/插件适配器没有替换模板字段，故不允许在缺少模板时隐式执行（避免意外删除匹配内容）。

## 验证

- `cargo test --offline --locked -p zi-regex-core`：11项通过，含捕获引用与 regex crate 展开结果的对照、Unicode、空匹配、0/1000/1001匹配、精确2 MiB结果及超限拒绝。
- `cargo test --offline --locked --features ui-preview`：957通过、0失败、27忽略；工具目录路由、常用搜索和工具草稿保存/恢复均有覆盖。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、`scripts/sync_tools.py --check`、`scripts/sync_studio_plugin_map.py --check`通过。
- `cargo build --offline --locked --release`通过，5分40秒。EXE `target/release/ZiDevTools.exe` 为35,524,096字节，SHA-256 `F6A64BE8FCD273EAB6D3F5F3B040E3B6E1429A4C080D6A66EB32DAE51A30542D`，程序文件版本仍为 `0.82.0-dev.105`。

没有做桌面界面的人工视觉检查，因此目录状态维持 `in-progress`；没有生成安装包或发布。

## 已知限制

不写回文件、不保留跨进程草稿、不做上下文差异着色；不属于Studio已接受插件能力。工具仍是开发中版本，不能据此宣称发布完成。
