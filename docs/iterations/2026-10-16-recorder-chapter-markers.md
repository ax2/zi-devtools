# 教程录屏章节标记（0.6.0）

日期：2026-10-16\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：教程录屏辅助 `0.6.0`（开发中）

## 目标与实现

- 录制时添加章节标记，时间取自 `elapsed_duration()`，因此会排除暂停时间；只允许录制状态添加，暂停、倒计时和保存期间按钮不可用。
- 标记最多256条，毫秒级时间严格递增；同一毫秒内的重复点击会被拒绝。标题默认“第N章”，录制后可编辑或删除。
- 保存结束后由用户选择 `.ffmeta` 路径。生成 UTF-8 `;FFMETADATA1`，按后续章节起点和最终有效录制时长计算区间；无效时长外的标记不导出。标题限制512字节并转义元数据分隔符、反斜杠和换行，拒绝 NUL 等控制字符。
- 写入复用本机原子无覆盖文件发布，只生成旁车元数据，不改动 MP4、不运行或捆绑 FFmpeg。

## 验证

- `cargo test --offline --locked recorder::chapters::tests -- --nocapture`：4项通过，覆盖毫秒递增、时间区间、标题注入转义、FFmpeg换行延续和非法输入拒绝。
- `cargo test --offline --locked chapter_metadata_export_writes_new_file_and_never_overwrites -- --nocapture`：1项通过；在系统临时目录验证实际写入、内容和冲突时不覆盖，并清理临时文件。
- `ffmpeg`/`ffprobe` 不在当前 PATH，未能用 FFmpeg 二进制解析旁车或实际 remux；格式按[官方 `ffmetadata` 规范](https://ffmpeg.org/ffmpeg-formats.html#Metadata-1)实现。不得据单测宣称实际媒体容器兼容验收完成。
- 全工作区验证通过：`cargo test --offline --locked`（退出码0；库测试925通过、27忽略），`cargo clippy --offline --all-targets --locked -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 与 `git diff --check`。FFmpeg二进制解析和实际 remux 不在本次验证范围。

## 后续

章节元数据仍需在可用 FFmpeg 环境实际解析并 remux 一段录屏，然后用 ffprobe 检查时间段与标题。提词器、临时画笔和真实录屏 UI 操作仍未完成。
