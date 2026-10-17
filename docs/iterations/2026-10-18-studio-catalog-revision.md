# 2026-10-18 Studio 插件目录修订

## 目标

在不修改 Studio 仓库的前提下，按接收方请求冻结 DevTools 当前目录、逐工具插件映射和来源收据；修正 JSON Object candidate-07 的外部目录索引收据，使 Studio 的通用校验器可以独立验证。

## 发现与决策

- 工作区是 `E:\zi-devtools`。工作树已有大量未提交修改，本次只在 DevTools 新增/更新对应的交付脚本、映射和交接档案，没有清理或回滚其他改动。
- 源应用版本仍为 `0.82.0-dev.105`。它只作为源目录的既有元数据；不可变目录修订使用内容派生 ID `devtools-catalog-2026-10-18-3c7c5aa8ad679a14`，不把日常开发版本伪装成目录修订号。
- JSON Object candidate-07 的逐文件内容和旧收据均保留。Studio verifier 按规范化对象键顺序计算目录索引，故提供新的修订收据，并将迁移清单指向该收据。
- 六个提供方包收据被复制进快照；其中四个原收据的索引编码未能独立证明与 Studio 规范化相同，因此清楚标记 `encoding-unverified`，没有将逐文件摘要通过夸大为目录索引通过。
- Studio 仓库只用于读取其请求和运行只读比较/校验脚本；没有写入 Studio 仓库。`acceptedCapabilities` 保持 0。

## 产物

- 快照目录：`release/plugin-candidates/devtools-catalog-2026-10-18-3c7c5aa8ad679a14/`
- 快照收据：`release/plugin-candidates/devtools-catalog-2026-10-18-3c7c5aa8ad679a14-verification.json`
- JSON Object 修订收据：`release/plugin-candidates/json-object-0.1.0-candidate-07-verification-revision-01.json`
- 生成器：`scripts/package_studio_catalog_revision.py`
- JSON Object 收据重建器：`scripts/revise_json_object_receipt.py`
- Studio 迁移表：`docs/studio-plugin-migration.json`

修订目录共 11 个文件、2,108,664 字节，SHA-256 `3c7c5aa8ad679a141b1268654bc9ef085530d33b1e123b40e08c26df80042ea1`；目录索引 SHA-256 `ef23b73d4310d066ca73753830afeed404843d066de9e374f6363552e69fe2c3`。冻结 `tools.json` SHA-256 为 `37353db759bad2df4320990a55b6c5d32b81058e75f3e98fc39646d1afd0499d`，迁移表 SHA-256 为 `8f67d99638646519e846d9391fd0fc7a4336460752e8cb3c4d7192c2e68d7a68`。

## 对比摘要

对 Studio 冻结 dev.105 基线的比较覆盖 154 个工具：新增 0、删除 0、版本/状态变化 76、范围变化 70、发现字段变化 76。当前目录状态数为 implemented 92、in-progress 35、planned 27。迁移表继续保留 154 项、6 个候选包映射和 `acceptedCapabilities=0`。

## 验证

- Studio `compare-provider-catalog.mjs` 对冻结快照源目录重新比较：154 项一致覆盖，变化摘要与快照一致；Studio 源目录未修改。
- Studio `verify-devtools-package.mjs` 对不可变目录及其外部收据运行成功，退出码 0，`verified=true`、11 文件、2,108,664 字节、`directoryIndex.status=matched`、`readOnly=true`。
- 同一通用 verifier 对 JSON Object candidate-07 和新收据运行成功，退出码 0，`verified=true`、37 文件、328,768 字节、`directoryIndex.status=matched`、`readOnly=true`。
- 两个通用 verifier 均明确显示 unsigned、不含签名验证、runtime 未接受、full tool acceptance 未完成；结果不代表 Studio 实际安装、Pi 授权、Host 生命周期或用户界面已验收。
- Studio 校验器的原始报告保存在本机 `target/` 检查证据中，不进入源码快照。

## 遗留事项

Studio 仍需在接收方工作区执行安装、运行时隔离、Pi 授权与撤销、更新/回滚/卸载及 UI 生命周期验收。工作树仍有本次范围外的历史未提交变更；没有做大范围提交，也没有更改应用开发版本或正式发布版本。
