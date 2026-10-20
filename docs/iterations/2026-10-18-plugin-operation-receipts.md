# 2026-10-18 插件操作回执映射修复

## 问题与修复

Text 0.4.1 的七个操作（JSON 格式化/压缩/校验、Base64 编码/解码、SHA-256、SQL 格式化）在源迁移表中缺少 operationScopes.verificationReceipt，目录打包器却从工具或包级别继承并写入 package-map，导致接收方逐操作语义校验不一致。

- 修复 `scripts/sync_studio_plugin_map.py`，让七个操作显式引用已有的 Text 修订回执；重新生成迁移表。
- 打包时要求源操作显式声明回执，不能仅在输出中补全。新增回归覆盖工具级及候选包级继承的拒绝与显式映射成功。
- 原生成器把差异数量固定为 76/70/76，录屏工具更新后实际为 78/72/76，导致新目录无法生成。现在从本次精确源文件摘要对应的比较报告读取真实差异列表，并拒绝缺失基线比较；README 与收据采用实际数量。旧冻结基线、旧候选目录和旧回执均不覆盖。

## 交付

程序仍为 `0.82.0-dev.105`，没有安装包或线上发布。

- 新目录：`release/plugin-candidates/devtools-catalog-2026-10-18-7d0c6461b1a64c19/`
- 外部回执：`release/plugin-candidates/devtools-catalog-2026-10-18-7d0c6461b1a64c19-verification.json`
- 11 文件、2,110,242 字节，目录索引 SHA-256：`92479abcf7aa1ca63e920dd9739f0abbf1a2e75b79933b7c15338d1562020853`。
- 源目录 SHA-256：`c37d4eac17f3c1321c52cc4c7e59da74cbb00abc07b332c4475171239c8852a5`。
- 源迁移表 SHA-256：`dbfd1c7dd752b1f8f523a8e255a59f9571a580fbecec9cab007a405eba485682`。

## 验证与边界

打包器 7 项 Python 测试通过。运行 Studio 现有 `compare-provider-catalog.mjs` 与 `verify-devtools-catalog-revision.mjs` 只读检查，未修改 Studio 工作区：新修订 verified=true、readOnly=true、directoryIndex=matched，154 个来源工具、31 个候选工具、6 个包，已实现92/开发中35/规划27。语义检查原始报告在 DevTools `target/catalog-7d0c6461-studio-semantic-check.json`。

签名、运行时接受和完整工具接受仍均为 false；acceptedCapabilities=0。本轮没有替代接收方安装、Pi 授权、Host 生命周期及界面验收，也未向其他会话发送消息。
