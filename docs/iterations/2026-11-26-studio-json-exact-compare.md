# Studio JSON 精确对比

2026-11-26 · Studio v0.7.9 [#28](https://github.com/ax2/zicode-studio-desktop/issues/28)

两个版本为 0.1.1 的独立候选覆盖工具中心文本对比，以及每份最多 1 MiB 的授权文件对比。有序数组按索引比较。无序数组比较递归多重集，重复数量仍影响结果。两种入口会准确区分相邻高精度小数和超出 u64 的整数，变化报告保留 before 与 after 的原始数字写法。整数与小数类型以及负零比较沿用原有规则。独立工具与 JSON 路径查询保留各自数值策略。

- 内联入口保留每份 8 KiB、完整信封 48 KiB 和现有 Worker 限制。真实 Windows Worker 32 项通过，包括最大字节输入、极指数、无效 Unicode 及可提前确认的结果超限。
- 独立文件入口沿用 resource.wasi.json-file-compare.v1、逐份文件授权和 8 MiB 结果上限。真实 Windows Host/Worker 验证了两种模式的 14 份读回与字节级保存，包含每份恰好 1 MiB、极指数、两份输入顺序和源文件摘要保持。取消确认、语法错误和超限的对照通过。
- 内联 Host 完成两种模式 12 份逐字节保存检查及 2 个无结果的语法错误对照。两种保存入口的高指数结果需要 Studio 对精确报告数据的 Host 校验支持。Windows 安装包、原生系统文件选择与签名可另行验收。
- 22 项定向 Rust 单测、有限范围的严格 Clippy 和目录检查通过。已有正则依赖的完整 lint 仍报未改动的基线告警。

冻结操作使用本仓库的 scripts/package_exact_json_compare.py、现有 0.1.0 候选外部收据及同一 Windows 实测 WASM。新目录和外部收据保存在 release/plugin-candidates 下。
