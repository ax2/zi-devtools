# JFR 有界采样调用树

日期：2026-10-18

## 目标

为 JFR 录制报告增加可读的采样调用路径，补上只有顶层热点、无法观察调用层次的问题，同时保持有界、离线、非 profiler 的定位。

## 实现

- 从 `jdk.ExecutionSample` 的栈帧生成 root-to-leaf 调用树；相同调用路径合并，节点记录经过该节点的 inclusive sample 数。
- 单份报告最多 256 个节点，每条采样栈最多 128 层；方法显示名最多 160 个字符。
- 输入缺失采样栈、空栈或超过节点/深度上限时设置 `truncated=true`，保留原有顶层热点与事件时间线。
- `java-jfr` 工具版本提升到 1.3.0；文档说明 inclusiveSamples 是样本计数，不是 CPU 百分比，功能不替代完整火焰图或 JMC。

## 验证

新增测试覆盖 root-to-leaf 顺序、重复路径 inclusiveSamples、栈深上限、节点上限及缺少采样栈时的截断提示。`rustfmt` 与 `cargo fmt -p zi-diagnostics-core -- --check` 通过；`cargo test -p zi-diagnostics-core --locked` 通过（14 passed）；`cargo clippy -p zi-diagnostics-core --all-targets --locked -- -D warnings` 通过。native 与 wasm32-wasip1 release 适配器构建成功，216 条 native/WASI 字节级一致性用例通过；其中一条直接断言调用树层级和累计样本。`python scripts/sync_tools.py --check`、`python scripts/sync_diagnostics_plugin.py --check` 与 `git diff --check` 通过。完整 workspace 测试、全 workspace Clippy 和桌面 release 构建尚未执行。

## 遗留

完整 workspace 测试、全 workspace Clippy 与桌面 release 构建仍待执行；真实 JFR 文件的调用树外观仍需在桌面 UI 中检查。功能只提供 bounded sampled call tree，不计算 CPU 百分比，也不生成完整 flame graph。
