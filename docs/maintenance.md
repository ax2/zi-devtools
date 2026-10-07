# 本机磁盘维护与缓存保留

缓存是否可重建不等于是否应该删除。优先保留能提高编译、测试、调试效率的缓存；容量只作为观察信息，没有固定容量清理门槛，也不按月清空缓存。

## 默认行为

- 维护脚本默认只检查并保留。`-Apply` 写入非敏感维护记录，不会自动删除缓存。
- 所有 `-Scheduled` 调用始终为审计模式，即使旧任务带有 `-Deep` 或 `-BudgetOnly`，也不会删除。
- 旧参数 `-BudgetOnly` 保留兼容，但只审计。`-BudgetGiB` 默认 0，表示不设参考容量；显式提供时仅记录是否超过参考值，绝不触发删除。
- 每周、每日任务保留原任务名称和频率，执行非破坏性检查；有构建或缓存内程序运行时跳过。
- `scripts/dev.ps1` 直接启动 Cargo，不在开发前扫描、清理缓存，也不覆盖增量编译环境设置。
- dev/test profile 启用增量编译。调试符号默认关闭，需断点调试时设置 `CARGO_PROFILE_DEV_DEBUG=1`（测试用 `CARGO_PROFILE_TEST_DEBUG=1`）。既有 `CARGO_INCREMENTAL=0` 环境设置会覆盖 profile，可按需要移除或改为 1。

## 检查与明确的手动清理

```powershell
pwsh -NoProfile -File scripts/maintenance.ps1
pwsh -NoProfile -File scripts/maintenance.ps1 -Apply -Scheduled -BuildCacheRoot D:\ZiBuildCache\zi-devtools
# 仅在确认 debug 缓存确实不再需要时使用：先预览，再明确执行。
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -BuildCacheRoot D:\ZiBuildCache\zi-devtools
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -Apply -BuildCacheRoot D:\ZiBuildCache\zi-devtools
```

只有非定时、非 BudgetOnly 的手动 `-Deep -Apply` 可以删除项目 `target/debug` 与显式外置 `debug`。它会增加下次构建时间，不能作为常规开发步骤。项目与外置 release、其他目标架构产物、源码、用户数据、知识索引、共享 Cargo 依赖、技术博客档案和截图均保留。

路径通过 Windows 文件句柄解析为实际物理路径；拒绝链接、越界和不合法外置目录。外置目录必须名为 zi-devtools，与项目分离。所有候选先校验；Rust/WiX 构建进程、缓存内运行程序会阻止删除；互斥锁阻止并发清理。没有“安全校验通过就应该删除”的含义，安全范围与文件必要性是两件事。

## 定时任务与验证

```powershell
./scripts/dev.ps1 test --locked
pwsh -NoProfile -File scripts/install-maintenance-task.ps1
# 历史任务名保留，现为每日缓存审计。
pwsh -NoProfile -File scripts/install-storage-budget-task.ps1 -BuildCacheRoot D:\ZiBuildCache\zi-devtools
pwsh -NoProfile -File scripts/test-maintenance.ps1
```

记录保存在 `%LOCALAPPDATA%\ZiDevTools\maintenance\last-run.json` 和 `history.jsonl`；预览不写记录。定时检查正常保留时无需通知。明显增长需要结合实际用途评估，不能仅凭体积、时间或可重建属性认定缓存无用。

2026-10-07 策略修正前曾清理 13678660811 字节 debug 缓存；已删除内容需要后续构建重建，不能恢复为原缓存。此前完整运行目录、用户数据与档案均保留。此历史清理结果不代表今后的默认策略。
