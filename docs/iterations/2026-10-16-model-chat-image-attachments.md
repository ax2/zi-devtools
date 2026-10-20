# 模型工作台图片附件（0.3.0，2026-10-16）

## 目标

让模型工作台可以安全、明确地发送本机图片，并支持 OpenAI 兼容接口和 Ollama 的图像消息格式。用户选择图片后先在本机预览；只有显式运行请求时才发送图像数据。

## 实施

- 支持 PNG、JPEG 和 WebP；原始文件最多 8 MiB。复用本机有界图片解码，超过 2048 像素的边长先缩小，再以 JPEG 质量 84 归一化。
- 透明像素合成到白色背景后编码；单张压缩图不超过 512 KiB，所有图片总计不超过 768 KiB，文本、文档和图片附件合计最多 4 个。
- 展示缩略图、文件名、像素尺寸和压缩体积；不将本机完整路径放进提示词或请求。
- OpenAI 兼容请求使用 `image_url` 的 base64 data URL；Ollama 请求使用 `images` 数组。固定模型连接探测不会携带图片。
- 会话文字及导出仅保留图片文件名和尺寸，不保存图像字节；需要后续分析时须重新附加。
- `docs/tools.json` 中 `model-chat` 更新为 0.3.0，并同步生成 `docs/tools.md`。应用版本保持 `0.82.0-dev.105`，未发布。

## 验证

- 图片测试覆盖本机 PNG 读取、尺寸/压缩上限、透明区域白底合成、路径不外泄、OpenAI 与 Ollama 消息结构、无效输入和超限原子拒绝。
- 附件区域在 1600×1125 浅色主题下完成截图检查，缩略图、附件信息及远端服务提示均可见，未发现布局裁切。
- `cargo test --lib chat_attachments::tests`：5 项通过。
- `cargo test --features ui-preview`：库测试 908 项通过、27 项忽略；所有启用的集成测试通过。需要本机 Ollama、正式服务或外部网络的用例按要求忽略。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 和 `git diff --check`：通过。
- 截图：`C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-model-chat-image-attachments\attachments-light.png`，SHA-256 `114FDFFE2BBCC81BC9308C99A179ECD809968F8C0DDA488B2BB1620AA958F302`。

## 限制与待办

- 未对真实 OpenAI 兼容服务或 Ollama 实例发起请求；协议兼容性目前由请求结构测试覆盖。
- 扫描件 OCR 未实现；只支持 PNG/JPEG/WebP，透明素材会以白色背景发送。
- 图片字节不进入对话历史或导出，因此继续会话时需重新附加。
