# 2026-10-17 插件候选映射与文本 View 覆盖

## 发现与修正

- 迁移表有 32 个 source tool 被标为 candidate，但部分行没有明确关联目录/收据；Base64/SHA 的当前文本候选只在 UI 测试中覆盖 JSON/SQL，清单有能力不等于界面完成。
- `sync_studio_plugin_map.py` 现在将每个 candidate 行关联到候选目录和包外验证收据，并在生成时拒绝缺失路径或收据的 candidate 状态。覆盖文本、转换、Inspect、Trace、Diagnostics 与 Actuator 六组候选。
- 扩展 `verify_text_plugin_view.py`：两主题真实浏览器通过 SDK 形状桥接调用实际 WASI，验证 JSON 格式/压缩/校验、通用 SQL、Base64 编码/解码、SHA-256 七个成功操作及重复键 JSON 负例。所有输出按预期逐字节/文本比较，确认没有页面错误。

## 验证和交付

- 43 条文本模块 native/WASI 字节对照保持通过；WASI 模块 705,993 字节，最高结果 47,191 字节。
- 桌面浅色 1180×900 与窄屏深色 390×844 的七项操作和 JSON 负例均通过；两张截图保存在 `target/text-plugin-030/` 并已目视检查。
- 新 unsigned 候选 `release/text-plugin-0.3.0-candidate-03`：15 文件、757,289 字节；外部收据绑定每个包文件和指定源码文件摘要，记录双主题 View proof。candidate-01/02 保留，不覆盖。签名/Studio Host/Pi/安装生命周期仍未验收。
- 迁移表 154 项；32 个候选 source tool 指向 6 个候选目录和收据，`acceptedCapabilities=0`。全量同步/缺失路径审计通过。

## 遗留

Base64、SHA 与其余候选在 Studio Host 的真实安装、Pi 授权、Worker 隔离、更新/撤权生命周期仍待接收方验收。桌面工具版本不因插件测试脚本和候选包目录轮次变化。
