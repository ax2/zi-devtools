# Studio 工具目录元数据扩展接入

日期：2026-10-17\
范围：`tool.directory.v1@1.0.0-preview.1` 的本仓库快照、Text 0.4.0 unsigned 候选及 Inspector/Trace 源 manifest 同步。没有修改 Studio 仓库。

## 目标与证据

Studio 目录筛选依赖提供方逐能力声明 categoryId；显示名本地化时还需有对应搜索别名。提供方不得改变稳定 capability ID 或冻结 compute/output 合约。源文档和 schema 摘要记录在 `contracts/studio-tool-directory/1.0.0-preview.1/README.md`，构建只读取本仓快照。

将有效类别与每个 Inspect、Trace、Text capability 的分类、用途描述、英语和中文搜索别名记录在 `capabilities.json`。共享生成器按 Host 规则检查 UTF-8 字节预算、控制字符和 NFKC/小写/trim 别名唯一性。首次检查发现 HEX 别名与英文字母规范化后重复；修正后通过。

## 修改

- `sync_inspect_plugin.py` 与 `sync_trace_plugin.py` 现在把 `schemaVersion: 2`、`requiredProfiles` 和逐能力 `directory` 元数据写入源 manifest；未覆盖其历史冻结目录。
- Text 包生成器升到插件包版本 0.4.0，添加目录快照、profile/schema/映射摘要和迁移说明。独立工具版本及 7 个 capability ID 不变。
- 新候选 `release/text-plugin-0.4.0-candidate-02`：17 个文件，820,745 字节；外部逐文件清单及索引收据可重算。WASM 在本仓源码重建，762,607 字节，SHA-256 `e979dd076d160e22a1ea69888e05629b38eaa3f339003e934f4afccc90d71b96`。
- `candidate-01` 是先前从旧模块重打的目录，原样保留。新建 `candidate-02` 以避免覆盖并绑定新 WASM。

## 验证

`verify_text_wasi.py` 的 43 条 native/WASI 字节对照通过；`verify_text_plugin_view.py` 的两主题操作检查通过；目录扩展验证覆盖17个能力，Inspect/Trace 源清单同步检查及19项目录/收据专项单元测试通过。插件迁移表已同步到 Text 0.4.0-candidate-02，154项完整映射校验通过，`acceptedCapabilities=0`。新旧收据均按文件路径重算大小、摘要和目录索引；旧冻结 Inspect 0.1.1、Trace 0.1.0 和 Text 0.3.0 的清单核对仍通过。

尚未为新模块运行官方 Wasmtime fuel 和冷启动复验，因此旧 0.3.0 的这些数字不可用于新构建。候选未签名；Studio 实际 Host 接入、筛选/执行、Pi 授权、安装生命周期及截图未完成。Inspect/Trace 的源 manifest 已有新元数据，但它们的新候选仍待单独重建与验证。
