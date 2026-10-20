# 资源管理器 MSI 能力清单验收（2026-10-16）

## 目标

让开发包构建可以证明当前 MSI 实际携带的右键菜单能力与 EXE 一致，并避免开发版本后缀传给 Windows Installer 的 ProductVersion。

## 修改

- `scripts/package.ps1` 从 EXE 只读能力检查器取得能力声明和 SHA-256；文件名继续保留完整开发版本，MSI 的 ProductVersion 使用基础数字语义版本。
- 新增 `scripts/verify_msi_menu_manifest.py`，编译后通过 Windows Installer API 反读 MSI 数据库，检查 ProductVersion、能力声明、EXE 摘要、六项事务动作及执行序列、桌面 EXE 文件条目，并分块读取 MSI Binary 表中的 `ZiMenuActions` 流，核对其 SHA-256 与本次构建 DLL 一致。
- `scripts/explorer_production_msi_smoke.py` 在编译型烟测中运行同一验收器，并把证明写入烟测结果。该脚本的真实安装/升级部分仍只允许 GitHub Hosted Windows 一次性 runner 执行。

## 验证

- `scripts/package.ps1 -OutputDir target\msi-manifest-final-20261016`：通过，显示 `PASS MSI menu manifest: 11 declared capabilities; exact EXE digest and transaction chain`。
- 使用当前 `0.82.0-dev.105` 开发构建，MSI ProductVersion 为 `0.82.0`，本次输出未出现 WIX1148 版本警告。
- 只验证了构建产物数据库内容；本地未安装 MSI，也未执行安装、修复、升级或卸载。

## 遗留与风险

MSI 清单一致性不能证明 WiX 自定义动作运行正确。真实安装生命周期仍由隔离 Windows runner 的 MSI smoke 覆盖；当前工作不关闭资源管理器功能的端到端验收项，也不把 `explorer-menu` 标记为已实现。程序版本未变更，未发布。


## DLL 二进制流核对（工具版本 0.28.0）

- 正向：重新打包后 MSI 内 `ZiMenuActions` 流摘要与 `target/release/zi_msi_actions.dll` 完全一致，证明 schemaVersion 2 输出记录了两个相同摘要。
- 负向：仅在临时目录创建并改写一份 DLL 副本，将它传给验收器；验收器以 `MSI menu transaction DLL differs from the built DLL` 拒绝，且没有生成证明。原构建 DLL 和 MSI 均未改写。
- 本轮 MSI、MSI 调试符号和临时包副本随后移除；schemaVersion 2 证明保存在本迭代目录。


## 生产烟测负向回归（工具版本 0.29.0）

`explorer_production_msi_smoke.py` 现在在安装前对临时 DLL 副本改写一个字节，并调用同一验收器。只有在验收明确报告 DLL 内容不匹配、返回失败且没有生成通过证明时，该烟测才继续；真实包和构建 DLL保持不变。当前工作机已独立验证同一负向条件，生产完整安装矩阵仍须由隔离 Hosted Windows runner 执行。


## 生产模板编译型烟测

本机实际运行 `python scripts/explorer_production_msi_smoke.py --compile-only --keep-artifacts`，通过未修改的正式模板编译、11项能力清单和EXE/DLL精确摘要、事务动作顺序与安装菜单选项表检查；该脚本还实际生成临时扰动DLL并确认验收器拒绝。烟测证明明确记录 `installationPerformed=false`。证明见 [`explorer-production-compile-proof.json`](2026-10-16-explorer-production-compile-proof.json)。


## CustomAction 绑定核对（工具版本 0.30.0）

验收器现在反读 `CustomAction` 的 `Type`、`Source`、`Target` 三列，对六个事务动作逐项比较共享 WiX 模板预期值：DLL 必须来自 `ZiMenuActions` Binary 表，执行时机必须对应 immediate、deferred、rollback 或 commit，ignore-return 标志也必须符合定义，并且 Target 必须是对应 Rust 导出函数名。用本机编译的实际 MSI 查询到六项精确匹配；正式模板 compile-only 烟测在本轮再次运行后归档的 proof 会包含完整绑定。


## CustomAction 类型与入口证明（工具版本 0.30.0）

实际编译型烟测已读取六项动作的 Type/Source/Target：Prepare=1、Rollback=1281、Execute=1025、CommitReady=1537、CommitReceipt=65、Finalize=65；六者均从 `ZiMenuActions` Binary 表取 DLL，并调用同名导出入口。证明已升级到 schemaVersion 3，见 [`explorer-msi-manifest-v3.json`](2026-10-16-explorer-msi-manifest-v3.json) 和 [`explorer-production-compile-proof.json`](2026-10-16-explorer-production-compile-proof.json)。这些表值不能替代安装期实际加载与运行验证。
