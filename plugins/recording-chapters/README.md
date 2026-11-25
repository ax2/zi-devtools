# 录屏章节元数据

在 Studio 工具中心搜索「录屏章节元数据」，选择并确认一份 UTF-8 JSON 章节计划。`durationMs` 是视频总时长，`startMs` 是章节起点，两者都用整数毫秒。`title` 可省略或留空，按导出顺序生成「第 1 章」等默认标题。

```json
{
  "durationMs": 90500,
  "chapters": [
    {"startMs": 0, "title": "开场"},
    {"startMs": 12000, "title": "步骤一"}
  ]
}
```

执行后会显示导出和跳过的数量。有效章节必须依起点严格递增。每章结束于下一章起点，最后一章结束于总时长；达到或超过总时长的标记会按独立版规则跳过。可预览全文，然后确认保存一份新的 UTF-8 `FFMETADATA1` 文件。Studio 通用文本保存使用 `.txt` 后缀，文件内容仍是标准章节元数据。原计划保持原样。格式规则见 [FFmpeg 元数据文档](https://ffmpeg.org/ffmpeg-formats.html#Metadata)。

仅申请 `resource.read`。每次一份最长 32 KiB 的计划，最多 256 项，有效标题不超过 512 UTF-8 字节，时长最多 31 天，完整元数据最多 48 KiB。共享独立版毫秒时间基、标题转义及回退规则。原生实时章节标记、屏幕和音源采集、视频轨写入继续留在原产品。

## 构建与验证

使用原 Cargo 缓存构建 `cargo build --offline --locked -p zi-recording-chapters-wasi` 和 `cargo build --offline --locked -p zi-recording-chapters-wasi --profile plugin --target wasm32-wasip1`。`scripts/verify_recording_chapters_plugin.py` 用真实 Windows Worker 和原生适配器比较文件字节，再独立读回每章时间与标题；`scripts/package_recording_chapters_plugin.py` 为通过的模块生成不可覆盖的候选目录和包外收据。
