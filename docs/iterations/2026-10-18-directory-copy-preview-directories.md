# 目录差异复制预览列出新建目录（1.2.7）

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`file-compare 1.2.7`

## 目标

目录复制此前会在用户确认后自动创建缺少的目标子目录，但复制计划只列出文件。现在预览阶段也应列明目录创建范围，让用户在确认前看清全部文件系统写入。

## 修改

- 复制计划收集目标缺失的每层子目录，以相对路径排序并去重；现存文件仍显示为冲突并跳过，链接、非目录父路径仍拒绝计划或执行。
- 确认区显示将创建的目录列表、总数和长列表截断提示；无须创建目录时明确显示这一点。
- 临时目录测试验证两个嵌套文件共享的缺失目录只列一次，确认执行后按预览路径创建目录并复制正确字节。
- 真实 eframe 鼠标冒烟改用嵌套来源路径，覆盖预览和实际创建目标目录的交互。

## 验证

- `cargo test --offline --locked directory_compare::tests -- --nocapture`：13 项通过、0 失败。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target/ui-preview-directory-copy-preview-dirs 165 directory-copy-smoke`：隔离临时目录 eframe 鼠标冒烟通过；选择差异、只读预览、明确执行，机器证明确认预览先于写入、新建目标目录、复制字节匹配所选来源且来源保留。
- 完成态截图：`target/ui-preview-directory-copy-preview-dirs/directory-copy-smoke.png`（1600×1125 浅色）。已检查目标子目录列表位于确认按钮之前、嵌套相对路径可读、执行后有明确结果；测试路径只在被忽略的 `target/` 夹具中。
- UI 冒烟首次运行因测试脚本仍按旧平面文件名查询状态而超时；同步改为查询嵌套相对路径后通过。另一次新输出目录的合成窗口交互超时，复用已初始化的隔离输出夹具后重跑通过；原因未能进一步确定，物理输入和窗口激活状态仍不是本用例的验收范围。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、工具目录同步检查（92 已实现 / 62 规划或开发中，154 插件映射、0 accepted）及 `git diff --check`：通过。

## 边界

复制仍按文件分别原子提交，不是整批事务；取消后已提交文件和已经创建的空目录可能保留。现存目标不覆盖，预览不执行任何写操作。操作系统层面的并发文件系统竞态仍不能完全消除。
