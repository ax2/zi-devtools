# 计算器复数支持（0.12.0）

日期：2026-10-16。程序版本保持 `0.82.0-dev.105`，本轮只更新计算器独立版本。

## 目标与实现

- 增加 `i` 常量、复数加减乘除与幂；加入主值平方根、对数、指数与复数三角函数，以及模、辐角、实虚部和共轭函数。
- 保持现有 i128 精确实数路径；复数结果明确标记为近似值。采用有限分量校验、缩放复数除法和稳定的正切/平方根计算。
- 工作表升级到 schema7，复数分量以规范科学计数法字符串保存；兼容 schema1–6，并拒绝旧 schema 冒充包含复数。
- 矩阵与数值表格拒绝复数类型；统计和绘图拒绝非实数结果，避免静默丢弃虚部。
- 更新计算器帮助、输入示例、工具版本目录、路线图及生成的工具清单，并加入“复数/虚数”搜索别名；未改程序开发版本。

## 验证

- `cargo test --lib`：901 passed，27 ignored。
- `cargo test calculator --lib --features ui-preview`：53 passed。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`：通过。
- `cargo fmt --check`：通过。`cargo build --release`：通过（9 分 18 秒；仅本机验证，未发布）。
- `python scripts/sync_tools.py --check`：通过，目录版本 `advanced-calculator` 为 0.12.0。
- `cargo run --example capture_ui --features ui-preview -- target\ui-preview-complex-calculator 520 single` 与 scene 521：实际渲染 1600×1125 深/浅主题；复数结果卡片、工具版本和工作区摘要均清晰可读，截图已归档。

## 边界

本轮没有发布程序或构建安装包。计算器整体仍为开发中；任意精度、CAS、非线性方程、更多高级绘图和更广泛的跨工具复数协议未实现。工作表元数据和分量格式是本地数据格式，不构成数值精度保证。
