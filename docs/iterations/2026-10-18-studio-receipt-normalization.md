# 2026-10-18 Studio 插件收据索引规范化

## 目标

使不可变目录快照中的六份包外收据都能由 Studio 通用只读 verifier 按同一结构化目录索引编码独立核验。只修订收据，不改候选插件包内容，不覆盖旧快照或旧收据。

## 发现与操作

- 上一修订 `devtools-catalog-2026-10-18-3c7c5aa8ad679a14` 中，diagnostics、inspect、text、transforms 的原收据被准确标记为 `encoding-unverified`；JSON Object 和 trace 已匹配。
- 为 diagnostics、text、transforms 新增 `revision-01` 收据；Inspect 已存在的 `revision-01` 结构化收据通过接收方校验后复用，旧收据保留。
- JSON Object candidate-07 继续使用其既有规范化 `revision-01` 收据；trace 原收据保持不变。
- `scripts/sync_studio_plugin_map.py` 改为映射对应修订收据；生成器在新不可变目录内重新冻结两份源 JSON、逐工具/逐操作包映射和六份收据副本。首轮对照发现 Text 操作级行没有 `verificationReceipt`，目录生成器未从工具级 `candidatePackages` 继承新路径而回退旧收据；修复路径解析后再生成最终目录。
- 所有外部候选目录逐文件集合、大小、SHA-256 均与原收据一致，确认本轮没有改动插件包。

## 新产物

- 不可变目录：`release/plugin-candidates/devtools-catalog-2026-10-18-d7d61b707a89a366/`
- 外部目录收据：`release/plugin-candidates/devtools-catalog-2026-10-18-d7d61b707a89a366-verification.json`
- 通用收据工具：`scripts/revise_plugin_receipt.py`
- 新目录修订 ID：`devtools-catalog-2026-10-18-d7d61b707a89a366`
- 修订 SHA-256：`d7d61b707a89a366a5382c9dca8f253043242eac8c8a873e6931b71ee4207689`
- 目录索引 SHA-256：`dee6a98d21de3c98d37db81c84928ffc6adec28b4859dbdbea5bf2bc5f1fe8ca`
- 文件数/总字节：11 / 2,105,306
- `tools.json` SHA-256：`37353db759bad2df4320990a55b6c5d32b81058e75f3e98fc39646d1afd0499d`
- 迁移映射 SHA-256：`613bddd0db2aef56ffd388e7e67bcdaed1085e8c24cc88b66a836922dc3d568d`

六包映射状态均为 `matched`：diagnostics 503 文件、4,287,534 字节；inspect 616 文件、2,341,254 字节；JSON Object 37 文件、328,768 字节；text 112 文件、1,032,175 字节；trace 346 文件、1,403,946 字节；transforms 798 文件、3,920,494 字节。

## 验证

- Studio `verify-devtools-package.mjs` 对新目录快照及六个候选目录分别运行，全部退出码 0；每项均为 `verified=true`、目录索引 `matched`、`readOnly=true`。
- 四个新/复用包收据均按 UTF-8、紧凑 JSON、`path,size,sha256` 键顺序计算 SHA-256；收据修订只在逐文件比对通过后写新文件，拒绝覆盖既有收据和越界/链接路径。
- 迁移表生成器用 UTF-8 LF 字节写文件，避免 Windows 文本换行使 `git diff --check` 将 CR 误判为尾随空白；目录快照按新迁移表摘要重新派生。
- 迁移表同步和 `--check` 通过；Studio 比较范围仍为154项、新增/删除0、版本/状态76、scope70、discovery76；状态分布92/35/27。
- unsigned、runtime 未接受、full tool acceptance 未完成；`acceptedCapabilities=0`。这轮只完成文件/目录收据校验，不代表 Studio 安装、Worker 隔离、Pi 授权、Host UI 或工具功能验收。

## 后续防错加固

重新核对包映射明细时发现，Text 操作级条目省略 `verificationReceipt`，旧生成器会回退到默认旧收据。现由 `receipt_for_scope` 从工具级候选包映射解析包收据；快照生成要求每个包恰有一个显式收据，缺失或冲突立即失败，不再猜文件名。`scripts/tests/test_package_studio_catalog_revision.py` 新增 5 项解析/拒绝回归测试；脚本测试套件共 28 项通过，插件映射、目录同步和不可变覆盖保护验证通过。

## 遗留

Studio 接收方仍需完成签名、Host/Pi、Worker、真实 UI、权限与生命周期验收。新快照是新的目录修订，不覆盖旧修订，也不改变桌面应用版本。
