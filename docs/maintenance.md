# 本机磁盘维护与容量控制

`target/` 以及明确指定的独立 Zi DevTools 构建缓存 `debug/` 都可重建。完整阶段目录 `dist/`、`release/`、源码、备份、用户数据、知识索引、技术博客档案和截图保留；共享 `CARGO_HOME` 不属于清理范围。

## 默认策略

- 开发和测试 profile 关闭增量编译、调试符号，避免增量缓存和 PDB 持续增长。需要断点调试时临时设置 `CARGO_PROFILE_DEV_DEBUG=1`；完成后清理对应缓存。
- 项目 `target/` 和显式指定的外置 `debug/` 各自采用 **6 GiB** 维护阈值。超限时，下一次空闲维护清理整个对应缓存。
- 已有每周维护仍只检查项目 `target/`：按月深度清理，其余时间清理增量缓存；超限提前深度清理。
- 新的每日容量任务 03:30 检查阈值；未超限不删除。外置缓存必须显式注册，外置 `release/` 始终保留。
- 阈值不是文件系统硬配额：正在构建或缓存内有程序运行时跳过，不能保证构建期间始终低于阈值。空间不足前可主动运行维护；清理后首次构建会更慢。

## 预览与手动清理

```powershell
pwsh -NoProfile -File scripts/maintenance.ps1
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -Apply
# 本机实际使用的专属外置缓存；只删除其中 debug，不删除 release。
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -BuildCacheRoot D:\ZiBuildCache\zi-devtools
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -Apply -BuildCacheRoot D:\ZiBuildCache\zi-devtools
# 只在超过容量阈值时清理，可调整 BudgetGiB。
pwsh -NoProfile -File scripts/maintenance.ps1 -BudgetOnly -Apply -BudgetGiB 6
```

不要把其他数据目录传为构建缓存。外置目录必须名为 `zi-devtools`，与项目目录分离，且为无链接的普通目录。项目所在卷挂载路径通过 Windows 文件句柄解析为实际物理路径；候选目标及其父级、后代中的链接均被拒绝。所有候选目标先校验，再开始删除。Rust/WiX 构建进程、从待维护缓存运行的程序都会阻止删除；维护互斥锁防止两个任务同时删除。

## 开发入口与定时任务

```powershell
# 构建前执行容量检查；识别显式设置的 CARGO_TARGET_DIR。
./scripts/dev.ps1 test --locked
./scripts/dev.ps1 build --release --locked
# 原有每周任务：仅项目 target。
pwsh -NoProfile -File scripts/install-maintenance-task.ps1
# 每日容量任务；可省略外置缓存参数，只维护项目 target。
pwsh -NoProfile -File scripts/install-storage-budget-task.ps1 -BuildCacheRoot D:\ZiBuildCache\zi-devtools
Get-ScheduledTaskInfo -TaskName ZiDevTools-DailyStorageBudget
```

直接执行 `cargo` 仍可用，但不会执行构建前容量检查；Cargo profile 的增量与符号设置仍然生效。每日任务使用当前用户普通权限，错过时间后补跑，遇占用正常跳过。迁移项目或外置缓存后重新注册任务。停用：`Disable-ScheduledTask -TaskName ZiDevTools-DailyStorageBudget`。

维护最新结果与追加历史保存在 `%LOCALAPPDATA%\ZiDevTools\maintenance\last-run.json` 和 `history.jsonl`。历史记录包含非敏感路径、字节数和维护状态。预览不写记录；实际任务失败可检查计划任务 `LastTaskResult`。维护安全性可用 `pwsh -NoProfile -File scripts/test-maintenance.ps1` 验证，测试只操作独立临时样本。

完整阶段版本按项目档案规则长期保留，本轮未设置自动删除版本的策略。若版本累积成为主要占用，应单独制定归档迁移方案。
