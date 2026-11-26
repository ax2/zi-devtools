# Studio v0.7.9 · 时区工作台交互

[Studio #744](https://github.com/ax2/zicode-studio-desktop/issues/744) 在同一独立插件中增加专用入口 `devtools.time-zone.workbench`。在工具中心打开「时区工作台」，搜索并选择来源时区，按需添加、移除 1 至 8 个不同的目的时区。结果按选择顺序排列，每行可复制时区、日期时间、UTC 偏移和缩写。可用键盘选择，重复或空缺的本地时间继续由离线换算报告。输入修改后忽略旧结果，取消后可重试。

专用界面只使用 Studio 公共 SDK 和现有浏览器复制事件，权限为空。IANA 2025b 的 597 个名称从与独立版相同的 `chrono-tz 0.10.4` 导出并逐项核对。单次换算请求和结果仍分别限 8 KiB 和 4 KiB。原 `devtools.time-zone.convert` 能力、通用表单、WASM 字节与权限保持。Windows 证据覆盖本机离线输入，独立版即时检查的操作流程仍在原生工具。

## 验收

- 候选 03 的打包源码 `de84fe7af82c687ac30596257a9975954625cf55`。九件冻结目录索引 `245297f65752aee7558cb5ade23269e646ebcf9a22f1fca193155772d57cbdab`，逐件收据 `a98ae1625bb128469c12d1ca3c0cc86720dbefbf640ef83a05baa1364412b300`。WASM `bc172f6ae3835dec7180882ab13f0ffa719541b5e47626a051526f35deab28e0` 与 v0.1.0 完全相同，复用 45 个真实 Windows Worker 与原生固定输入对照。
- 现有实际 Windows Studio `235431bc2c949148ed8ab731f03953385e1ef0ce`（0.7.4 Debug）的工具中心首装英文、重启中文各验证六组。两次均运行真实 Worker，覆盖重复/第九项、移除顺序、跨日、两个重叠时间候选、空缺分钟、非法日期和取消后重试。视图宽 1113 与 433 像素，无横向溢出。回执 SHA-256 `e14c54438663f3a3bdc8808c0b52479fe43b9463fad661dfaeadd0f4e8a5fca5`、`36cbb5ceaf14e5682c76f9e8540d31399964835dd8d7e18caf13d9601d6085b5`。
- 逐行复制造成两次 Chromium 可信 `copy` 事件，核对选区的完整制表符文本和复制后按钮焦点。夹具拦截系统写入，日常 Windows 剪贴板未被读取或覆盖。宿主无 `clipboard-write` 权限；浏览器不接受复制时选中该行并提示 `Ctrl+C`。
- 保留候选 01 的实际沙箱表单提交失败和候选 02 的主题识别差异。候选 03 使用显式按钮和日期字段 Enter，识别 Studio 支持的主题 ID。两项修复均在插件内。
