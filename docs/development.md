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
```

原有 Cargo 参数透传仍可用，如 `./scripts/dev.ps1 test -p zi-diagnostics-core --locked`。dev/test 均保持 debug=0、incremental=true，避免为了省空间关闭有用的增量缓存。release仅用于需要优化构建的验收或发布，保留thin LTO；plugin保持opt-level3/fat LTO/panic abort。WASI 64MiB最大内存和2MiB guest stack统一定义在.cargo/config.toml，避免漏参数或临时profile产生另一套构建。

启动脚本默认检测会改变缓存身份的环境覆盖，只显示变量名并拒绝意外覆盖，不清除或改写环境。专门调试或实验需要这些覆盖时，显式传入 `-AllowBuildOverrides`。固定使用现有工具链、Cargo缓存和项目target；不按工作日志轮次创建新的target目录或自定义profile。

## 清理

定时维护仍只审计；不以容量阈值自动删除缓存。默认maintenance.ps1保持现有行为。需要清理已被后续构建替代的历史缓存时，先形成具体文件/目录清单，保留当前及最近有效构建身份、第三方依赖、源码、用户数据和完整release目录，再使用 `scripts/prune-build-cache.ps1 -Plan <清单.json>` 预览；检查无构建/目标程序、无链接且仅项目target内，再加 `-Apply` 执行。清单记录逐目标类型、大小、文件数量及最后修改时刻；执行前完整复核，变化即拒绝。清理记录保存到技术博客档案，清理后核对保留目标。

不能靠时间或大小单独断言缓存无用，也不清空整个target/debug。清理清单应限定为本项目的历史构建身份，并说明替代它们的已验证构建。未来如需旧提交再编译，对应已删历史身份需要重新生成。
