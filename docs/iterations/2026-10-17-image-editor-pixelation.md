# 图片编辑器像素化遮挡（1.5.0）

状态：本地开发迭代；程序版本保持 `0.82.0-dev.105`，未发布。

## 目标与实现

让截图/图片编辑流程可对矩形选区做可调节像素化，同时保留已有黑色遮挡作为高敏信息的建议方式。图片编辑器现提供 4–64 像素块大小滑杆，按原图像素绘制选择区域；拖动过程显示像素化预览，输出阶段在裁剪后的图像上按编辑顺序应用像素化，alpha 按原像素保留，块颜色使用 alpha 加权平均以避免透明像素的隐藏 RGB 污染可见颜色。原始图片不修改，实际编码预览与另存确认流程不变。

截图工作台的快捷编辑器接力复用这项能力。像素化提示明确告知仍可能辨认，高敏信息请改用黑色遮挡并检查最终输出。工具目录将图片裁剪与标注升到 `1.5.0`，截图与透明套索目录对齐到 `0.8.0`；程序开发版本未变。

## 验证

- 像素算法单测覆盖块内一致、块间差异、透明度加权颜色、alpha 保持及选区外不变。
- 图片编辑导出回归覆盖裁剪、黑色遮挡、像素化、箭头与源图不变。
- `cargo test --offline --locked --features ui-preview --quiet`：995 个库测试通过；其余集成测试通过，27 个测试按需依赖本地 Ollama/HTTPS 等外部服务而忽略。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check`：通过。
- `cargo build --offline --locked --release`：通过。`target/release/ZiDevTools.exe`，40,531,456 字节，SHA-256 `C96961A9F9292B74D0FB6E3AF2B88DB90299930A8AFCDFD686842192782DAAE3`。仅本机验证构建，未发布或更改程序版本。
- UI 合成截图：`screenshots/image-editor-pixelation-20261017/image-editor-dark.png`，1600×1125，370,501 字节。已检查像素化工具选中态、滑杆数值/提示、预览画布和其他编辑工具布局。截图只含合成测试图，不含用户截图。

## 限制与遗留

像素化不是安全擦除，信息可能从上下文或多张图像中推断；高敏内容应使用纯色遮挡并检查实际编码预览。画布预览为缩放近似，最终输出采用原图像素算法。截图工作台内原生绘制、模糊/渐变遮挡、置顶窗口以及真实多屏/混合 DPI 仍需继续实现或验收。本轮未发布。
