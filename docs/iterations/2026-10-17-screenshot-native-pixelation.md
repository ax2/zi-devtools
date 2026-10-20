# 截图工作台原生像素化（2026-10-17）

## 目标

让用户直接在截图工作台中对选区进行像素化，调整块大小、检查实际输出并随时撤销，而不需要把图像接力到编辑器。

## 实现

- 扩展遮挡历史条目，支持纯黑遮挡和像素化两种矩形操作，保留每个像素化操作当时的块大小。
- 截图预览提供4–64像素块大小控制；拖动时以蓝色矩形提示目标，松开后后台生成实际结果。
- 像素化使用块内alpha加权颜色平均，并保留每个源像素的alpha，避免矩形像素化改变透明轮廓或抗锯齿边缘。
- 黑色遮挡和像素化共用最多100项的撤销/重做历史；任何重算都从捕获源、当前轮廓与羽化状态重新生成。
- “截图与透明套索”工具版本升至0.10.0；程序开发版本仍为`0.82.0-dev.105`。

## 验证

- 算法测试验证像素块颜色平均及alpha保留；异步渲染测试验证像素化、撤销、重做和原块大小恢复；截图模块定向测试19项通过。
- `cargo test --offline --locked --features ui-preview`：1000项通过、27项忽略（需要本机模型、显式网络或专用CI环境）；所有已执行测试通过。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；工具目录检查为91项已实现、63项规划/开发中。
- `cargo build --offline --locked --release` 通过，用时约7分35秒。产物`target/release/ZiDevTools.exe`为40,550,912字节，SHA-256：`E39C2A803F981ACB378637110C743F1807483BC720499A174CA5899F99E043B9`。程序版本仍为`0.82.0-dev.105`，未发布。
- 1600×1125浅色与深色合成界面：[浅色](../../screenshots/screenshot-redaction-20261017/screenshot-lasso-light.png)（347,044字节）、[深色](../../screenshots/screenshot-redaction-20261017/screenshot-lasso-dark.png)（339,373字节）。只用于布局/对比度检查，不代表真实桌面或物理交互验收；固定视口仅展示部分透明预览。

## 遗留与边界

- 原生箭头、文字、模糊/渐变遮挡和置顶截图仍未实现。
- 真实多屏、混合DPI和物理鼠标跨应用验收仍待进行；合成输入测试不替代该验收。
- 截图仍需用户主动保存或复制，本次不自动创建临时文件、不更改程序版本、不发布。
