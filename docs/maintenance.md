# 本机磁盘维护

项目根目录的 `target/` 是 Rust 构建缓存，可以重新生成。`scripts/maintenance.ps1` 每周清理 `target/debug/incremental` 和 `target/release/incremental`；每月第一个周日清理整个 `target/`，避免旧依赖产物长期累积。不会碰源码、`dist/Stage-*` 完整阶段版本、`release/` 安装包、技术博客档案、用户配置或知识索引。完整清理后第一次构建会更慢。

先预览占用：

```powershell
pwsh -NoProfile -File scripts/maintenance.ps1
```

确认后立即清理：

```powershell
pwsh -NoProfile -File scripts/maintenance.ps1 -Apply
```

手动预览或执行完整构建缓存清理：

```powershell
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep
pwsh -NoProfile -File scripts/maintenance.ps1 -Deep -Apply
```

脚本在删除前检查目标绝对路径仍位于项目目录、目标是普通目录且内部没有链接。检测到 Cargo、rustc、rustdoc 或 WiX 构建进程时跳过定时维护，手动运行则报错，避免和构建同时操作。

在本机安装每周日 03:00 的当前用户计划任务（每月第一个周日执行完整清理）：

```powershell
pwsh -NoProfile -File scripts/install-maintenance-task.ps1
Get-ScheduledTask -TaskName ZiDevTools-WeeklyMaintenance
```

任务使用当前登录用户的普通权限；错过执行时间后会在可运行时补跑。项目迁移目录后应重新运行安装脚本以更新路径。需要停用时运行 `Disable-ScheduledTask -TaskName ZiDevTools-WeeklyMaintenance`；需要移除时运行 `Unregister-ScheduledTask -TaskName ZiDevTools-WeeklyMaintenance -Confirm:$false`。只想扩大清理范围时，先审查目标及阶段归档，不要把知识索引或发布目录当成缓存。
