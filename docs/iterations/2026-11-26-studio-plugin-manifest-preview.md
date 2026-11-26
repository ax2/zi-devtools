# Studio v0.7.9 · ZiDevTools 插件清单预览

在 Studio 的插件管理启用候选并授予 `resource.read` 后，在工具中心搜索「ZiDevTools 插件清单预览」。确认共享一份 ZiDevTools 原生 `schema_version: 1` 声明式 JSON，运行后查看包身份、原字节 SHA-256、声明的工具数量和类型。以清单原顺序每次查看八项，最多 64 项。输入属于当前确认的文件，原件保持完整。本轮仅覆盖原生 `plugins` 的读取预览。[迁移 #29](https://github.com/ax2/zicode-studio-desktop/issues/29)

共享解析规则核对原生内置工具/操作和 HTTP 声明。预览额外拒绝重复 JSON 键，隐藏正文、样例、模型、URL、响应路径及错误原文。未添加服务调用、账户、数字签名、源插件安装或权限管理。输入上限 256 KiB，去除 JSON 外围空白后的字节与对象字段成本另有总量限制。明确限制见[候选使用说明](../../plugins/plugin-manifest-preview/README.md)。

## Windows 验收

- 候选 02 从 `16913e7d97a3c937ddab550db30bbb345cc7adf3` 冻结，74 件共 925,572 B。目录索引 `e34ff9d18d8cab7ad945cfd69d1e0da518582c750a531198890aacd240d012f3`，逐件收据 `e8a66878b11f143a62a4d07ca37ab4c223b16d37746af9b445242efe044deb05`，WASM `07d922c5f3c21812f400fbc72f7814d3341b020c52ee97b0dbc0211b07ffb67c`。
- Windows Studio Worker 46 组与原生适配器一致，覆盖说明示例、64 工具页尾、精确 256 KiB 文件和复合边界、超限、重复键与脱敏。独立版原插件 21 项、文本流程兼容 2 项和新增两个目标的严格 Clippy 通过。Worker 回执 SHA-256 `b2d13c553324c14ad2759217f43bdc045c0e0d15afd38e1eb610215c66887745`。
- 在已有实际 Windows Studio 0.7.4 Debug `235431bc2c949148ed8ab731f03953385e1ef0ce` 的隔离资料，正常工具中心首装执行六项，重启重跑两项。确认共享、撤权关闭执行并清空结果及原文件摘要保持均通过。未操作系统文件选择窗口。首装、重启回执 SHA-256 `a0b047e48f8090f17c5a0aaadcd45c5b7213feb0724fd07e8d5661f6d355953c` 与 `e6793ff1051e6ec73283ac573a323b3d1b73a6d3a3ae5742eb8d2b1d786f5985`。
- 保留早期 Worker 对密集键、密集转义、过长地址和内部空白的燃料耗尽轨迹。当前候选返回清楚的复杂度错误，不扩大现有 Worker 预算。候选 01 因输出契约深度拒绝安装，候选 02 只等价调整 GET/POST 声明深度，WASM 同字节。
