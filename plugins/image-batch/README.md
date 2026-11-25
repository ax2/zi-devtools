# 图片批处理（小型 PNG）

在 Studio 工具中心搜索「图片批处理」，选择一至两张本机静态 PNG，核对排列顺序并确认提供。最大宽度可留空保留原尺寸，或选择 1–48 像素按比例缩小，较小的原图不会放大。执行后按标注的输入顺序生成独立 PNG，每张图可分别预览和确认保存。取消某张图的保存不影响其他结果或原文件。

每张 PNG 最多 32 KiB，宽和高分别最多 64 像素。输出保留透明度和无损像素编码；原生工具的 Lanczos3 缩放和取整规则保持一致。动画、16 位通道和其他图片格式会提示不支持。一张文件无效时，本次批次不会生成部分可保存结果。PNG 重新编码可能改变文件字节及元数据，并可能比压缩过的原件大。

插件只需 `resource.read`，通过版本化的文件快照通道处理当前明确确认的图片。原生工具的 100 张处理、JPEG／WebP、目标目录批量保存、运行中部分提交与实例迁移保留原有范围。Studio 选文件和保存在其可信界面完成。

## 构建与验证

使用既有 Windows Cargo 缓存先构建 `cargo build --offline --locked -p zi-image-batch-wasi`，再构建 `cargo build --offline --locked -p zi-image-batch-wasi --profile plugin --target wasm32-wasip1`。通过 `scripts/verify_image_batch_plugin.py` 对照原生适配器与真实 Windows Worker，固定用例覆盖输入顺序、多个输出、PNG 字节及像素回读和有效与无效边界。`scripts/package_image_batch_plugin.py` 根据真实用例收据冻结包与包外验证文件。
