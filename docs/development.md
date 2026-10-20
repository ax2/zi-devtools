# 本地开发与缓存复用

本地开发版本保持 `0.82.0-dev.105`。日常修复、工具开发、UI调整和验证不递增 Cargo 版本、清单程序版本或插件包版本，不自动创建标签、安装包、冻结运行目录或公开 Release。提交号、工作日志、验证结果记录每轮进展。确需发布程序或交付新的插件版本时才协调修改版本并按 releasing.md 验证；工具各自的行为版本规则仍见 tool-versions.md。

## 固定入口

从项目目录运行：

```powershell
./scripts/dev.ps1                 # 日常两份桌面程序，debug，无发布
./scripts/dev.ps1 dt-check        # 工作区检查
./scripts/dev.ps1 dt-test         # 工作区测试
./scripts/dev.ps1 dt-lint         # 全目标/全特性严格检查
./scripts/dev.ps1 dt-plugin       # 诊断插件，固定plugin profile与WASI预算
./scripts/dev.ps1 dt-image-plugin # 图片资源插件实验构建，crate 单独声明 256 MiB module ceiling
./scripts/dev.ps1 -UseUserCache dt-test # 明确改用用户本机缓存目录，避免写满项目卷
```

原有 Cargo 参数透传仍可用，如 `./scripts/dev.ps1 test -p zi-diagnostics-core --locked`。dev/test 均保持 debug=0、incremental=true，避免为了省空间关闭有用的增量缓存。release仅用于需要优化构建的验收或发布，保留thin LTO；plugin保持opt-level3/fat LTO/panic abort。WASI compute 默认 64 MiB 最大内存和2MiB guest stack统一定义在.cargo/config.toml。`zi-image-wasi` 因图片解码实测需要，在自己的 build.rs 为资源 profile 单独声明 256 MiB module ceiling；Host Store 仍须单独限制实际运行预算，不得把该 module ceiling 当成宿主已批准的资源额度。

启动脚本默认检测会改变缓存身份的环境覆盖，只显示变量名并拒绝意外覆盖，不清除或改写环境。专门调试或实验需要这些覆盖时，显式传入 `-AllowBuildOverrides`。默认固定使用现有工具链、Cargo缓存和项目target；不按工作日志轮次创建新的target目录或自定义profile。若项目卷空间不足且要保留原有项目缓存，可显式传入 `-UseUserCache`，将本次 Cargo 子进程的 `CARGO_TARGET_DIR` 指向 `%LOCALAPPDATA%\ZiDevTools\zi-devtools`。命令结束后会恢复调用 Shell 原有环境变量；不自动判断容量、不移动或删除项目缓存。需检查外置缓存时使用 `./scripts/maintenance.ps1 -Breakdown -BuildCacheRoot "$env:LOCALAPPDATA\ZiDevTools\zi-devtools"`。

本机运行 MSI smoke test 的 `--compile-only` 模式只生成临时测试安装包，不执行安装；成功后默认删除本轮临时夹具，避免在 `target` 中累积重复的测试包。失败时保留现场供排查；需要检查成功产物时传入 `--keep-artifacts`。CI 事务测试仍把证明文件和截图留在 runner 临时目录供工作流归档。

## 清理

定时维护仍只审计；不以容量阈值自动删除缓存。默认maintenance.ps1保持现有行为，同时报告项目缓存总量和所在卷可用空间，后者仅作观察，不影响清理决策。需要定位占用时，可使用 `./scripts/maintenance.ps1 -Breakdown` 只读查看 `target` 的前两级容量分布；不会改变清理候选、不会写入维护记录，也不会按大小触发删除。需要清理已被后续构建替代的历史缓存时，先形成具体文件/目录清单，保留当前及最近有效构建身份、第三方依赖、源码、用户数据和完整release目录，再使用 `scripts/prune-build-cache.ps1 -Plan <清单.json>` 预览；检查无构建/目标程序、无链接且仅项目target内，再加 `-Apply` 执行。清单记录逐目标类型、大小、文件数量及最后修改时刻；执行前完整复核，变化即拒绝。也允许清理由测试脚本以 `zi-msi-menu-*` / `zi-msi-production-*` 命名留在target根目录的临时测试输出；这些目录不是Cargo增量缓存，必须先确认创建脚本引用、无链接且无进程占用，维护脚本会跳过正在运行的对应Python测试脚本。清理记录保存到技术博客档案，清理后核对保留目标。

不能靠时间或大小单独断言缓存无用，也不清空整个target/debug。清理清单应限定为本项目的历史构建身份，并说明替代它们的已验证构建。未来如需旧提交再编译，对应已删历史身份需要重新生成。


## 接力交互的分阶段验证

`scripts/verify-handoff.ps1`复用`dev.ps1 -UseUserCache`，按格式检查、接力专项测试、三个原生UI夹具构建、范围/滚动/键盘/方向导航八组明暗交互、完整测试、严格Clippy的顺序执行。任何一步失败即停止后续步骤。默认不进行优化构建；需要验证Release时加`-Release`，放在桌面交互通过之后。它不发布、不改版本、不清缓存。`-PlanOnly`只输出阶段计划，不创建输出目录或运行Cargo。

运行证据写到项目target下唯一的handoff-verify目录，记录每阶段日志、退出码和耗时；原生夹具使用独立UUID临时服务配置与偏好文件，不接触真实服务或工作资料。输出仍是开发验证，不等于人工可用性、安装或插件Host接受。

该脚本已实际验证失败时停止（滚动回归失败后没有启动Release）；修复后的四组原生流程、1344项完整Rust测试与严格Clippy通过，1.12.1优化构建已通过。新增键盘夹具仍需按当前流水线重跑。最新完整终态见handoff-access-filters迭代记录，不把PlanOnly当作运行通过。

目录复制与后台任务验证：`scripts/verify-directory-copy.ps1 -Release`，复用共享 Cargo 缓存，执行目录测试、浅/深色任务中心和重试原生流程、工作区测试、严格 Clippy 与 Release；每次日志独立保存到项目 target，不自动清缓存或发布。
