# 教程录屏章节标记（0.6.0）

程序版本保持 `0.82.0-dev.105`，未发布。录制中可以添加最多256条章节标记，暂停时长不计入时间；录制结束后可编辑/删除标题并主动另存 UTF-8 FFmpeg `FFMETADATA1` 旁车文件。导出不修改视频、不覆盖已有文件，也不捆绑或自动调用 FFmpeg。

章节序列化测试4项通过；临时目录中的实际导出和冲突不覆盖测试1项通过。当前机未检测到 `ffmpeg`/`ffprobe`，所以尚无实际元数据解析、MP4 remux 与 ffprobe 验证。对应源码记录：[2026-10-16-recorder-chapter-markers.md](../../docs/iterations/2026-10-16-recorder-chapter-markers.md)。
