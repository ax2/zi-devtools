# 本地知识库图片 OCR（2026-10-17）

## 目标

让用户可将图片作为本地知识源，通过明确的预览或手动索引同步动作，在 Windows 本机提取可检索文字；文件扫描本身不启动 OCR，也不把识别内容上传网络。

## 实现

- 知识源清单接受 PNG/JPEG/WebP 并记录文件版本；扫描仅检查路径、大小、修改时间和 SHA-256。
- 文档解析页使用 Windows 用户首选 OCR 语言对单张图片识别并预览带位置的文本分块。只有之后显式同步全文索引，识别正文才会写入本机 SQLite 索引。
- 限制图片文件为 16 MiB、单边不超过 12000 像素、总像素最多 1600 万，识别正文不超过 4 MiB；不支持的格式、无效图片和空识别结果明确失败。没有在线回退。
- OCR/解析错误在知识索引事务中保留旧索引；当前 Windows OCR 调用不能中途取消。
- 这是图片 OCR 的首个功能增量：工具目录当时登记知识源 `1.2.0`、文档解析 `1.1.0`、增量索引 `1.2.0` 与开发中的 OCR 助手 `0.1.0`；后续语言选择和版本变化见[语言选择记录](2026-10-17-knowledge-ocr-language-selection.md)。程序版本未变，未发布。

## 验证

- `cargo test --offline --locked --features ui-preview document_ingestion::tests -- --nocapture`：6 项通过，包含图片解码、格式/像素/文本边界、损坏数据拒绝及原有文档解析回归。
- 初始功能增量只运行了带 mock 的 OCR 解析测试；后续在[语言选择记录](2026-10-17-knowledge-ocr-language-selection.md)中补做了真实 Windows OCR 和索引冒烟测试。临时 PNG 在测试结束后删除。
- `cargo fmt --check`、全量 `cargo test --offline --locked --features ui-preview --quiet`（1018 通过、28 忽略、0 失败）、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py`、`python scripts/sync_tools.py --check` 和 `git diff --check` 已通过。
- 真实 OCR 证据与限制以[语言选择记录](2026-10-17-knowledge-ocr-language-selection.md)为准；高对比度英语合成图不代表复杂扫描件的识别质量。

## 未完成与风险

- 若 Windows 用户首选语言对应的 OCR 功能未安装，解析会明确失败。
- 目前未提供 OCR 语言包选择、扫描 PDF 页面栅格化、页面文字坐标、独立可搜索文本导出或 OCR 结果校对编辑。
- 单图 OCR 不支持运行中取消；索引同步取消需等当前文件解析返回后才生效。
