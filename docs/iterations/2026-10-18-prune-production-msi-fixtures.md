# 清理过期 MSI compile-only 烟测产物

日期：2026-10-18 06:54（Asia/Shanghai）\
程序版本：0.82.0-dev.105（未变更）

## 目标与依据

释放开发盘空间，同时保留有效 Cargo 增量缓存、完整 release 运行目录及验证证据。只评估 `target/zi-msi-production-47jmz9r4` 和 `target/zi-msi-production-ixuayu6_` 两个由 `explorer_production_msi_smoke.py --compile-only --keep-artifacts` 留下的临时目录。每个目录含 5 个文件；烟测证明均标记 `installationPerformed=false`。这些 schema 2 临时 MSI 已被归档的 schema 3 编译证明替代，源码和重新生成路径仍在。

## 路径和进程检查

- 维护脚本确认项目根为 `E:\zi-devtools`，清理计划只包含上述两个 `target` 子目录。
- 目标及子项均无重解析点；清理前未发现运行于项目 `target` 的程序、Cargo/Rust/WiX 构建或对应 MSI smoke 测试进程。
- `scripts/prune-build-cache.ps1 -Plan ...` 先成功预览，验证 2 个目标、逻辑大小共 27,838,993 字节；执行时再次检查清单大小、文件数、时间戳、链接和进程。

## 执行与结果

脚本删除两个临时目录共 10 个文件，报告逻辑释放 27,838,993 字节。E 盘可用空间从 156,147,712 增至 183,353,344 字节，实际增加 27,205,632 字节。`target/debug`、`target/release`、`target/wasm32-wasip1`、SQLite UI 截图目录和归档编译证明均保留。未清理 Cargo 缓存、未生成新构建、未更改程序或工具版本。

清理后再次运行 `scripts/maintenance.ps1 -Breakdown`，target 为 68.212 GiB；`debug/incremental` 仍为 34.722 GiB、`release/deps` 仍为 5.592 GiB。维护脚本只读显示 E 盘可用 0.171 GiB；目标目录不存在性、受保护目录存在性检查与 `git diff --check` 均通过。

## 证据

- 本记录同目录的 `2026-10-18-prune-production-msi-fixtures.json` 为精确目标计划；对应 `.preview.json` 和 `.applied.json` 留存脚本复核与执行结果。
- [归档编译证明](2026-10-16-explorer-production-compile-proof.json)保留更新后的 schema 3 验证结果。
