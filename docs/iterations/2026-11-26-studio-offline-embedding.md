# Studio v0.7.9 · 离线向量比较

在 Studio 插件管理启用候选并授予 `resource.read` 后，在工具中心搜索「离线向量比较」。确认共享两份独立版复制的 `.json` 数组，列表顺序对应 A/B。也可确认一份已有、含正好两个 `embeddings` 的本机响应。结果只展示维度、两份范数、余弦相似度和欧氏距离。原件不变，原生 `embedding-playground` 的 Ollama 生成、模型选择继续保留。[Studio #29](https://github.com/ax2/zicode-studio-desktop/issues/29)

向量最多 8192 维，字段须为有限数、绝对值最多 1,000,000，且两份等维非零。每文件限 256 KiB。Windows 已验证 8192 维整数、4096 维长小数和两份各 256 KiB。去掉外围 JSON 空白后按内容字节与小数/指数标记计入预算。以[候选使用说明](../../plugins/embedding-files/README.md)为准。

## Windows 验收

- 候选 02 源码 `0b5001c8c8c599a8789d3ff2becfbfd634edf26c`。8 件，共 250,479 B。目录索引 `7abbbe98440395e72032d0db7cbb3c2cf4691adb1ee60cb6e34727f33c404da7`，逐件收据 `b7dd67fb245a85970320b85c4425d0d13d8108698c84a4da9687cc6fb0b39b5e`，WASM `2fe71885d5925a6b9795a0ea8a7f8765a436c88fcd70292bcd03a024e4c2f244`。
- 57 项实际 Windows Worker 验证通过，独立版 `embedding` 兼容用例 1 项，新增两目标的原生和 WASI 严格检查通过。固定用例的可读结果与原生适配器逐字相同。结构化浮点度量用绝对与相对容差 `2e-14` 对照，并对独立计算数值验收。完整 Worker 证明 SHA-256 `51c4ccacdffca501dbf62b0539ce28d1802261b153ee673b005173d6c7efeae5`。
- 复用 Windows Studio 0.7.4 Debug 源码 `235431bc2c949148ed8ab731f03953385e1ef0ce`。在实际隔离工具中心首装后执行 8 项，重启后重跑 2 项。确认两份反序文件，A/B 范数为 2/5，撤权后清空结果并禁用执行。一份响应仍返回相同度量，原文件摘要保持，测试使用合成输入。首装及重启证明 `2af8145b902291c84d9e4c88f8cac04155784735356b2fc3bf4f38e37f401054`、`84c8469567a3c50d3df274e0c4b660db3f8ad35498aff9bbc4f87fcec32caa87`。
- 保留初版密集小数和大段外围空白的燃料失败。最终共用 Worker 限额不变，按解析量提前给出业务提示。保留候选 01 的独立证据，02 固定 SIMD 分块写法后重新全量核对。系统文件选择窗口留集中验收。
