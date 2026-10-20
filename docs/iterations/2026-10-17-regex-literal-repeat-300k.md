# 正则固定字面量重复加速扩展至 300,000 字节（1.0.8）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：正则表达式 `1.0.9`（含下文1.0.8验证纠正）

## 修改

将共享核心中无显式捕获、根 HIR 为精确重复、子项为非空 UTF-8 字面量的快速路径上限从 256 KiB 提升到 300,000 字节。按锁定 Thompson 编译器每个展开字节一个 32 字节 Range 状态，300,000 字节占 9,600,000 字节，低于既有 10 MiB 限额并为固定包装保留 885,760 字节。增加 300,000 次 ASCII 与多字节 UTF-8 的原生引擎编译边界测试和完整 300,000 次匹配报告测试；超界表达式仍走原引擎。

新增 `regex-large-valid-repeat-300k` 固定 native/WASI 用例。输入文本为空时，精确正则 `a{300000}`确定不可能匹配，因此无需展开巨型 Thompson 程序，返回原“没有匹配”结果。

## 验证

- `cargo test --offline --locked -p zi-regex-core`：9 项通过；包括原引擎在既有限额下编译 300,000 字节重复，以及完整匹配报告对照。
- 原生 `zi-fields-wasi` 与 `wasm32-wasip1` 模块重建。`scripts/verify_fields_wasi.py`：220 项实际 native/WASI 输出及 Schema 逐字节通过。
- 原 Wasmtime 36.0.2 预算不变：10M fuel、5 秒、64 MiB 内存、2 MiB guest stack。220/220 通过，最高 fuel 9,765,441；`a{300000}`空文本用例为 31,898 fuel，历史完整成本诊断 309,192,660 fuel。
- 28 项 `verify_regex_program_limits.py` 原生/WASI 编译路径与长文本匹配对照通过。
- 220 个全新独立进程冷启动全部通过，最高 842.455 ms；旧 v4 的单项 5037.761 ms 超时及 v3 的九项超时记录仍保留。
- `cargo fmt --check`、完整 `cargo test --offline --locked --features ui-preview` 和 `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings` 通过。
- `cargo build --offline --locked --release` 通过，5分49秒；程序版本仍为 `0.82.0-dev.105`，没有创建发布包。
- 新 WASI 模块 1,427,833 字节，SHA-256 `6685fd7de6188d8f16e372848a86768efff99a8243a674d38427a89c1dfd55bf`。

## 边界

本优化只涵盖上述 HIR 形状及 300,000 字节以内的重复；更大的重复、捕获、字符类、断言、交替和嵌套语法未因此获得统一 10M fuel 保证。固定矩阵通过不表示完整正则预算通过，也不代表 View、协议协商或 Studio Host/Pi 接受。插件仍为实验模块，未打包、签名或交付。

验证脚本首次复跑时使用了与 Python 实际加载路径不同的同哈希 Wasmtime DLL provenance，严格路径校验将所有项拒绝；重新绑定实际 DLL 并通过单项预检后，完整冷启动矩阵通过。另一次 fuel 脚本调用漏传 `--text-results`，在结果解码阶段停止；补齐模式后完整 220 项通过。上述是验证命令配置问题，不计为产品失败，也没有降低预算或跳过用例。

## 1.0.9 预算复核与修复

随后把更新后的 compare 插件源码重新纳入完整 10M fuel 检查时，发现 `regex-large-valid-repeat-300k` 实际耗尽全部 fuel。此前“31,898 fuel”的记录来自较早的 fields WASI 构建产物，不能证明当时更新后的 compare 模块；原失败证据保留在 `target/proposal3-audit-20261017/fuel-run/fuel-proof.json`。根因是标准 HIR 解析器先展开大重复表达式，尚未进入短输入快速返回分支就耗尽预算。

修复在 HIR 解析前识别严格限定的单个字面 Unicode 标量加精确数字重复，并继续限制展开字节数不超过300,000；转义字符、字符类、分组、边界、区间重复及其他表达式仍交给原解析器。新增 `中{100000}`空文本边界，确认 UTF-8字节上限路径也在预算内。

- `cargo test --offline --locked -p zi-regex-core`：9项通过。
- 更新后的 compare 原生/WASI功能矩阵：265/265通过（包含时钟不可用降级检查）。
- Wasmtime 36.0.2固定输入矩阵：260/260通过；10,000,000 fuel、5秒、64MiB、2MiB guest stack保持原值，最高9,829,644 fuel。`a{300000}`空文本为44,643 fuel，`中{100000}`为空文本为45,407 fuel。
- 当前 compare WASI模块1,822,943字节，SHA-256 `36a64204a90dc48b781e1fe27cf739cc37f2d4dde36493bfa76d8893e59b8c00`。

1.0.8 的冷启动记录对应修复前构建；1.0.9 随后重新执行完整 `cargo test --offline --locked --features ui-preview`（956通过、0失败、27忽略）、严格全目标Clippy、格式与目录同步检查，并完成 `cargo build --offline --locked --release`（5分26秒）。新EXE为35,511,296字节，SHA-256 `ECCD767C26EEC824DD33EFE24750D128574E4E3C433DD5A9EDB7F8631107DC72`，文件版本仍为 `0.82.0-dev.105`。没有启动桌面GUI做人工视觉验收，也没有生成安装包或发布。修复后完整语法预算仍未证明，候选没有提交Studio Host，未发布。
