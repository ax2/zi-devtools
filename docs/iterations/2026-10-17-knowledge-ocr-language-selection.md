# 知识库 OCR 语言选择（2026-10-17）

## 目标

让用户能选择本机已安装的 OCR 语言，并保证文档预览与之后的手动知识索引同步使用同一语言。

## 实现

- 文档解析页增加异步加载已安装 Windows OCR 语言及选择控件；默认仍为 Windows 用户首选语言。
- 选择只保存在当前应用会话内；应用重启后不写入设置文件。语言清单加载不自动识别图片，只有用户点击预览或同步索引才会执行 OCR。
- 语言选择传入后台预览与 SQLite 增量索引同步；调整语言后清除旧预览，语言清单加载和解析运行期间锁定 OCR 操作/选择，避免结果与当前设置不一致或争用 OCR 队列。
- OCR 引擎对所选语言要求精确匹配；缺失语言时明确失败，不回退到另一个语言或网络服务。
- 工具版本递增：`rag-ingestion 1.2.0`、`rag-index 1.3.0`、开发中的 `ocr-documents 0.2.0`。程序版本保持 `0.82.0-dev.105`，未发布。
- 工具目录校验发现 `network` 条目版本为 `1.2.0`、历史记录却止于 `1.1.0`；依据对应任务中心阶段记录将当前目录版本校正为 `1.1.0`，未改动网络诊断实现。

## 验证

- Windows 本机语言枚举和高对比度合成英文图识别通过；新增手动索引集成冒烟测试，明确指定 `en-US` 后确认合成图中的 `HELLO` 能通过 FTS 检索命中。
- 上述真实 OCR 测试使用 `ZIDEVTOOLS_OCR_FIXTURE` 指向临时合成 PNG，测试结束后删除；未读取用户图片或文档。
- `cargo fmt --check`、全量 `cargo test --offline --locked --features ui-preview --quiet`（1018 通过、29 忽略、0 失败）、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py`、`python scripts/sync_tools.py --check` 和 `git diff --check` 均通过。
- eframe `ui-preview` 在深色与浅色 1600×1125 视口各渲染一张；已目视检查语言选择、刷新按钮和 OCR 操作无裁切，截图见 `screenshots/knowledge-ocr-language-selection-20261017/`。

## 边界

- 仅验证本机 `en-US` 高对比度合成图；中文、多语言混排、低分辨率、倾斜、阴影和复杂扫描件尚未评估。
- OCR 语言设置只存在于运行中的应用状态，不跨重启持久化；已有索引不因语言切换自动重建，需要用户手动重建或让文件版本变化后再次同步。
- 本迭代完成时，扫描型 PDF OCR、页面坐标、独立导出及用户编辑识别结果仍未实现；扫描 PDF 后续实现记录见 `2026-10-17-scanned-pdf-ocr.md`。
