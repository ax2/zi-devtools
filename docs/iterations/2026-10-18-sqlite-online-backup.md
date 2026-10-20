# SQLite 在线备份

日期：2026-10-18

## 目标

在 SQLite 浏览器的单格修改和单行删除之外，为用户提供一个显式、本机、不会覆盖现有文件的一致数据库副本流程，降低数据写入前后的恢复风险。

## 实现

- 新增“备份数据库…”入口；用户在系统保存对话框选择目标即授权创建副本。路径不写入配置或历史。
- 只读打开源数据库，使用 SQLite 在线备份 API 复制已提交数据，包含 WAL 中的已提交内容；过程中可取消并查看进度。
- 在同一目标目录创建唯一临时文件，完整备份后运行 `PRAGMA integrity_check`；只有检查通过才以硬链接原子无覆盖发布。目标已存在、源文件身份变化、取消、备份错误或目标文件系统不支持硬链接时不发布最终目标，并尽力清理临时文件。
- 不自动备份、不创建备份计划、不提供恢复操作；不会改变源数据库。备份可能需要与数据库相近的磁盘空间。
- SQLite 浏览器版本升至 `1.5.0`；授权写入与变更预览版本升至 `0.4.0`。开发版程序版本不变。

## 验证

- `cargo test --offline --locked sqlite_browser::backup::`：3 项通过，覆盖含未检查点 WAL 数据的一致备份、源库/WAL 保持不变、完整性查询、目标文件冲突不覆盖和备份前取消。
- `cargo test --offline --locked --quiet`：1,077 项通过、32 项因本机服务/模型/交互条件忽略、0 失败；包括 WAL 快照、目标冲突和取消测试。
- `cargo fmt --all -- --check`、`cargo clippy --offline --locked --all-targets -- -D warnings`、`python scripts/sync_tools.py --check` 与相关文件 `git diff --check` 通过。目录同步为 92 个已实现工具、62 个规划/进行中工具。
- `cargo build --offline --locked --release` 通过，耗时 9m42s；`target/release/ZiDevTools.exe` 为 41,330,176 bytes。构建后 E 盘可用空间为 1,085,673,472 bytes（约 1.01 GiB）；未清理 Cargo 缓存，程序版本保持 `0.82.0-dev.105`，没有生成正式发布包。
- eframe 真实渲染的 SQLite 工具页 light/dark 截图均为 1600×1125；目视检查确认新增按钮和表格工具栏在两种主题中对齐、可见、未裁切：`screenshots/sqlite-online-backup-light.png`、`screenshots/sqlite-online-backup-dark.png`。

## 遗留限制

备份副本需要用户自行选择保存位置并管理；不保证目标目录空间充足，不自动删除旧副本。某些文件系统不支持硬链接时会安全失败；不进行跨重启备份任务恢复，也不提供将副本恢复为活动数据库的入口。


## 后续体验优化（1.5.1）

此前备份 UI 仅展示复制百分比；复制完成后执行完整性检查时，进度会长时间停留在接近完成的数值，无法解释当前阶段。现改为明确显示“复制数据库”“核验完整性”“发布备份”与“已完成”阶段，并将完成提示统一为简体中文。版本目录与 SQLite 使用说明已同步更新。

- 1.5.1 阶段提示改进验证：3 项 SQLite 备份测试通过；全量 cargo test --offline --locked --quiet 为 1,077 通过、32 忽略、0 失败；Clippy、cargo fmt --all -- --check、工具目录 --check 均通过。修改后的 Release 构建通过，耗时 12m36s；target/release/ZiDevTools.exe 为 41330176 bytes，生成时间 2026-10-18 06:10:34；构建后 E 盘可用空间 5263966208 bytes。开发版仍为 0.82.0-dev.105，未创建正式发行包。
