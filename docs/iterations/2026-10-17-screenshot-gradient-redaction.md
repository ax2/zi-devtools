# 截图渐变遮挡（工具 0.14.0）

日期：2026-10-17
程序开发版本保持：0.82.0-dev.105

## 目标

为截图工作台增加渐变遮挡，覆盖路线图中仍未完成的一项，并纳入已有标注渲染与撤销/重做模型。

## 发现与设计

- 截图标注都从捕获源重算，最多保留100项历史；复用该模型可避免在已处理像素上累积误差。
- 半透明遮罩会让原内容透出，因此渐变使用三色不透明 RGB 色阶覆盖区域内可见像素；保留输入 alpha，透明像素和区域外内容不变。
- 支持横向、纵向和对角线方向；颜色先用固定预设，后续再评估是否需要可配置调色板。

## 修改

- `src/image_tools/screenshot.rs` 增加渐变栅格化、UI 工具按钮/方向菜单、拖动预览和矩形选择，并沿用后台渲染与共享撤销/重做历史。
- `docs/tools.json` 把截图工具版本升至 0.14.0，程序版本不变；同步来源版本映射、生成目录和截图工作台/路线图说明。

## 验证

- 梯度像素单测验证横/纵/对角方向端点与插值、选区边界、透明像素和 alpha 保留。
- `cargo test --offline --locked --features ui-preview`：1007通过、27忽略、0失败。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、工具目录同步检查、插件映射检查及 `git diff --check` 通过。
- `cargo build --offline --locked --release` 通过，用时8分04秒。开发 EXE：`target/release/ZiDevTools.exe`，40,582,656 bytes，SHA-256 `1F55B9CF2359DBF5E92EBB62C787CEA314D761F43003886B38035AF99F11D8AF`。

## 遗留与验收边界

- 颜色暂为固定的三色预设；没有调色板。
- 像素算法和源码构建路径已验证；本轮未对实际桌面 UI 做人工交互/视觉检查，也未验收真实物理多屏、混合 DPI。
- 置顶截图、长截图和 OCR 仍在路线图中。
