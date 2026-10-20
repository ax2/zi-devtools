# 构建缓存容量只读盘点（2026-10-18）

## 目标

确认 E: 剩余空间过低是否由项目构建缓存造成，并判断是否存在不会清除现有缓存的安全后续方案。

## 只读证据

- `E:\zi-devtools\target` 占用 68.212 GiB；执行 `scripts/maintenance.ps1 -Breakdown`，仅统计，没有清理。
- `target` 最大分项：`debug/incremental` 34.722 GiB、`debug/deps` 21.579 GiB、`release/deps` 5.592 GiB、`debug/examples` 3.503 GiB、`debug/build` 0.800 GiB、`release/build` 0.448 GiB。其余输出分散在 WASI、插件和根目录产物中。
- 盘点时 E: 剩余约 0.167 GiB；D: 剩余约 614.6 GiB。
- 项目路径中的 `target` 是普通目录，不是链接；盘点前未发现指向本项目的 Cargo/Rust 编译进程。
- `docs/development.md` 要求固定复用项目本地 target，不按迭代创建额外 target；维护脚本针对项目内 target 检查并拒绝链接路径。

## 处理与边界

保留所有现有构建缓存，没有按容量阈值删除或移动文件。E: 空间不足以安全运行新的 Cargo 测试；格式检查与目录脚本可继续运行。当前证据说明缓存占用是空间压力的重要来源，但单靠大小不能判断其中哪些缓存已经失去开发价值。

## 验证

仅运行只读 `scripts/maintenance.ps1 -Breakdown` 并检查 Cargo 配置、项目开发约定与进程状态。没有运行 Cargo 构建或测试，没有更改编译配置，也没有删除用户文件。
