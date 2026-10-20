# 时区工作台首版

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`0.1.2`（目录状态：已实现）

## 目标与实现

为跨地区会议和时间核对提供本地 IANA 时区转换。一个来源时间可转到最多 8 个目的时区；使用 `chrono-tz` 内置数据库，不进行网络请求。日期格式支持分钟或秒精度，结果显示当地日期时间、UTC 偏移、时区缩写和 UTC 时刻。

来源时间若因夏令时结束而重复，必须明确选择较早或较晚的时刻；若因夏令时开始而不存在，则拒绝转换并提供最近有效分钟。最近有效时刻按最多 24 小时边界进行对数步进搜索，以便处理整日跳过类民用时间变更，同时避免界面每帧线性扫描数千分钟。重复时刻单选项只有在实际切换时才使结果失效，避免选择后下一次界面绘制清除转换结果。

本机 eframe 深色主题渲染截图发现：重复时刻的候选偏移错误地直接格式化 UTC，显示两个 `+00:00`，用户可能据此选错。现改为将候选 UTC 转回来源时区后显示真实偏移，并增加纽约回拨的 `-04:00`/`-05:00` 回归断言；同步增加该页面的深浅主题截图场景。

## 验证

- `cargo test --offline --locked --features ui-preview time_zone::tests -- --nocapture`：6 项通过，0 失败。覆盖多目标跨日转换、DST 重复/缺失时间、整日跳时边界、目标数量限制和重复时刻选择在多帧界面绘制后仍保留。
- `cargo test --offline --locked --features ui-preview app::registry::tests::every_visible_definition_has_an_executable_route_and_valid_discovery -- --nocapture`：通过；目录类别校正为现有“时间与生成”，页面有统一工具库可执行路由。
- 首次全量测试暴露的分类校验失败已修正。最终 `cargo test --offline --locked --features ui-preview --quiet`：1042 项库测试中 1015 通过、27 忽略、0 失败；可运行的集成测试通过。
- `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 均通过；`git diff --check` 通过（Git 对工作区大量既有文件提示 LF/CRLF 转换警告）。
- 工具目录升到 `0.1.1` 时曾验证 release 构建通过，用时 7 分 08 秒；其后发现了来源偏移显示错误，因此最终验收以 `0.1.2` 修复版的重新构建为准。

## 视觉复核发现

- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\\debug\\time-zone-preview-dark-20261017 542 single` 与对应索引 543 的浅色命令均通过。真实 eframe 窗口截图均为 1600×1125，已查看深浅主题：输入和目标列表未截断；重复时间明确显示 New York 的 `-04:00`/`-05:00`。截图只写入项目 `target/debug`。
- 首张深色截图曾显示两个错误的 `+00:00` 候选偏移；已修复实现并增加单元断言，工具版本升到 `0.1.2`。
- 修复版 `cargo test --offline --locked --features ui-preview --quiet`：1042 项库测试中 1015 通过、27 忽略、0 失败；可运行集成测试通过。严格 Clippy、fmt、工具目录与 Studio 插件映射同步检查、目标文件 `git diff --check` 均通过。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\\debug\\time-zone-interaction-smoke-20261017 542 time-zone-smoke`：成功以合成鼠标点击“较晚 -05:00”和“转换时间”；断言结果为 `2026-11-01T06:30:00+00:00`。截图和机器证明在同一 `target/debug` 目录，截图显示来源 EST、UTC 06:30、上海 14:30 和伦敦 06:30。
- eframe 截图和合成鼠标交互确认主窗口实际路由及核心操作，注册表测试确认统一工具库路由可执行；尚未使用物理鼠标，触屏和混合 DPI 不在本工具的功能范围内。
- `cargo test --offline --locked --features ui-preview --quiet`：1042 项库测试中 1015 通过、27 忽略、0 失败；可运行集成测试通过。严格 Clippy、fmt、工具目录与 Studio 插件映射同步检查及目标文件 `git diff --check` 均通过。
- 将工具目录状态更新为 `implemented` 后，`cargo build --offline --locked --release` 再次通过，用时 7 分 28 秒。最终 `target\\release\\ZiDevTools.exe` 为 40,626,176 字节，SHA-256 `44DA3542E303D21652131D3063067B11ACCCF4A3A8B9AFA3353582006DF88C79`；程序文件与产品版本仍为 `0.82.0-dev.105`，未发布。

验证边界：交互流程以 eframe 原生窗口及合成鼠标事件执行，未使用物理鼠标。工具不操作桌面坐标或显示器捕获；触屏和混合 DPI 不属于本工具的专项范围。

## 遗留

当前数据只保存在界面会话；依赖随程序打包的 IANA 规则，离线可用但数据库仅随软件更新。尚未覆盖批量导入、日历邀请或行程规划。
