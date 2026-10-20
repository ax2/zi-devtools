# 工具库中的版本检索

## 目标

工具达到上百项后，允许直接按独立工具版本筛选当前工具及已启用插件工具；用户不需要逐行打开版本菜单，也不需要增加新的常驻筛选控件。

## 修改

- 搜索索引加入工具的当前独立版本，完整版本号返回精确匹配，带点前缀（如 `0.3`）返回对应版本段匹配。
- 搜索索引同时覆盖工具历史版本与变更摘要；历史版本精确/前缀匹配和变更说明匹配使用独立提示，不与当前版本混淆。工具库标题、说明及搜索框提示已明确说明可查当前/历史版本与变更说明。
- 结果说明显示“版本匹配”或“版本前缀匹配”；收藏、最近、常用及名称/用途匹配规则保留。
- 工具库搜索框提示版本检索方式；`Ctrl+K` 使用同一注册表搜索，因此同样支持版本查询。
- 未声明版本的旧插件仍显示“版本未声明”，不根据插件包版本推断，也不会伪命中版本查询。
- 更新独立工具版本说明和桌面路线图；不修改工具自身或应用开发版本。

## 验证

- `cargo test app::registry::tests`：5 passed；覆盖内置及插件工具的当前/历史完整版本、版本前缀、变更摘要、匹配提示与搜索结果；最终界面场景另含运行时断言。
- `cargo test`：898 passed / 27 ignored / 0 failed；忽略项需要本机 Ollama、官方 MCP SDK fixture 或公网 HTTPS 等外部条件。全量测试在最后的 UI 文案/预览场景补充前通过；补充后重新跑了注册表定向测试、Clippy 和真实 UI 场景。
- `cargo clippy --all-targets --features ui-preview -- -D warnings`：最终 UI 场景版本通过。
- `cargo build --release`：最终代码通过；本地开发版本保持 `0.82.0-dev.105`。
- `cargo fmt --check` 与 `python scripts/sync_tools.py --check`：通过。
- `cargo run --example capture_ui --features ui-preview -- target\ui-preview-version-history-search <scene> single`（scene 516–519 分次运行）：实际渲染并断言历史版本 `0.2.0` 与历史摘要词 `sqlformat`；两个主题下分别显示“历史版本匹配”与“版本历史匹配”。人工检查 1600×1125 画面，列表层级及行内提示清楚。
- 历史搜索截图归档于 `C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-tool-version-history-search\`，含四张主题/查询组合、README 与 SHA-256。
- 原当前版本检索截图仍保留在 `C:\Users\ax2\Documents\技术博客\zi-devtools\screenshots\2026-10-16-tool-version-search\`，SHA-256 `1D8A76EBF3AEC1FBED433770FC898C51FB478D537D4F9AD70F9FBD3337A62269`。
- 截图完成后，`scripts/prune-build-cache.ps1` 对 `target\debug\examples` 的 117 个临时编译文件逐项预览、校验并清理，删除逻辑大小 `187,703,406` bytes；源码 examples、debug 依赖/增量缓存和完整 release 输出保留。计划与执行记录在技术博客维护档案。
- 实际 Release 可执行文件由本轮构建更新；未创建发布包或递增版本。

## 边界

- 当前检索支持当前/历史版本精确匹配、点分版本前缀及历史变更摘要关键词；复杂范围表达式仍未实现。历史文案按规范化词项做包含查询，不提供字段/日期过滤。
