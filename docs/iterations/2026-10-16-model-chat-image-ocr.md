# 模型工作台图片本机 OCR（0.4.0，2026-10-16）

## 目标

将现有 Windows 本机 OCR 引擎接入模型工作台图片附件，让用户在发送前提取和校对图片文字，并明确决定是否把文字加入模型消息。

## 实施

- 图片附件显示“本机识别图片文字”操作；解码和 OCR 在后台线程执行，使用 Windows 系统首选语言，不阻塞界面。
- 识别文本仅暂存在当前附件状态中，显示可编辑预览；OCR 本身不会把文字写入消息，也不会向模型发送图片或文字。
- 用户明确选择“添加为文本附件”后，结果才进入消息预览。它沿用已有 64 KiB 单附件、128 KiB 总正文和 4 个附件上限；超限时不做截断，拒绝加入。
- 加入后仍需用户再次点击运行发送。会话按现有策略保存文本消息；图片字节仍不进入会话历史或导出。关闭/清除附件会丢弃未加入的识别文本。
- 复用现有 Windows OCR 工作线程和语言包，不新增依赖；更新 `model-chat` 为 0.4.0，应用版本保持 `0.82.0-dev.105`，未发布。

## 验证

- `cargo test --lib chat_attachments::tests`：7 项通过，包含 OCR 文本显式加入、单文件/总量/附件数限制与失败原子性。
- Windows OCR 已安装语言枚举测试通过。
- `cargo test` 和 `cargo test --features ui-preview --quiet` 均通过；ui-preview 库测试 910 项通过、27 项忽略，启用的集成测试通过。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check`、`git diff --check`：通过。
- 1600×1125 浅色主题附件界面截图已目视检查：3/4 附件时“添加为文本附件”可用；识别文字可校对，正文和按钮无裁切。示例 OCR 文本为合成内容。
- 截图：`C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-model-chat-image-ocr\attachments-light.png`；SHA-256 1415C56E7D1D19EE466C3E94F5C05DC869165B9F96944900EB56A1DC6F8CCEA8。

## 限制与待办

- 未验证 Windows OCR 引擎对真实/合成文字图片的识别质量；语言枚举通过不代表相应 OCR 语言功能已安装。识别失败会在附件卡片显示错误。
- OCR 使用 Windows 系统首选语言，界面暂不提供独立语言选择；运行中的 WinRT 识别不能中途取消。
- 扫描型 PDF OCR、非 Windows OCR、模型服务端到端兼容性仍待验证。
