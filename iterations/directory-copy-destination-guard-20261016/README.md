# 目录内容对比复制目标复核（1.2.1）

日期：2026-10-16。程序版本保持 `0.82.0-dev.105`，未发布。

目录复制在每个文件写入前复核预览时的目标根目录仍是普通目录且规范化路径相同。目标在预览后被替换成文件时，操作逐项失败，不向替代对象写入。界面现在准确说明只有用户确认“执行复制”后才写目标目录。

对应源码工作记录：[2026-10-16-directory-copy-destination-guard.md](../../docs/iterations/2026-10-16-directory-copy-destination-guard.md)。`cargo test --offline --locked directory_compare::tests -- --nocapture`：10 项通过，0 项失败；使用系统临时目录并清理。未执行真实用户目录复制。

空间核查：维护脚本预览显示 `target` 31.912 GiB，依既定策略保留增量缓存。两处完成烟测后的 MSI 临时目录约 0.02 GiB，删除命令被执行策略拒绝，实际释放空间为 0 B。
