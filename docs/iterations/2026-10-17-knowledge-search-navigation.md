# 索引同步后直达关键词检索

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变，未发布）\
独立工具版本：增量知识索引 `1.1.0`

## 目标与实现

非空索引同步成功后，索引页显示“下一步：打开本机关键词检索…”。按钮只切换页面，不提交查询；查询框与来源筛选仍由用户控制，结果也不会自动发送到其他工具。索引为空、同步失败、新一轮同步正在进行、索引删除确认打开或来源配置锁定时不提供此快捷入口。

这样“保存材料 → 扫描来源 → 明确同步索引 → 手动检索”每个有数据影响的步骤仍独立确认，同时减少完成索引后寻找检索工具的导航成本。

## 验证

- 定向集成测试用临时文本目录执行实际索引同步，验证只在成功且存在索引文件后开放检索后续入口。
- eframe 截图：[`knowledge-search-followup.png`](../../screenshots/knowledge-search-followup-20261017/knowledge-search-followup.png)，1600×1125 深色主题；本机路径使用合成值，按钮和“不会自动查询”说明清楚可见。
- 定向同步测试通过；完整 `cargo test --offline --locked --features ui-preview --quiet`：1005 通过、0 失败、36 忽略。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo build --offline --locked --release`、工具目录及 Studio 插件映射同步检查、`git diff --check` 均通过。Git 对既有工作区文件提示 LF/CRLF 转换警告。

## 遗留

此按钮不执行关键词查询。英文 FTS5 与中文字面匹配的现有行为保持不变。
