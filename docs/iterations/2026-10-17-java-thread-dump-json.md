# Java 线程转储 JSON 支持

日期：2026-10-17\
工具：Java 线程转储分析 `1.3.0`\
程序版本：`0.82.0-dev.105`（未发布）

## 目标

让 Java 线程转储工具读取传统 `jstack` / `Thread.print` 文本，以及 JDK `jcmd <PID> Thread.dump_to_file -format=json <file>` 输出，支持虚拟线程诊断而不臆测输入没有提供的事实。

## 实现

- 识别 JSON 根结构 `threadDump.threadContainers[]`，展平线程用于统一列表，同时保留容器名称、父级、所有者、容器索引、线程 ID、名称、状态和栈帧。
- 只在输入明确提供 `virtual` 布尔字段时分类为虚拟或平台线程；旧格式没有该字段时显示“未知”。缺失状态也显示未知。
- JSON 模式报告来源格式、线程/容器计数和状态统计；明确说明该格式没有报告完整锁等待关系和死锁判定，不从栈帧猜测。
- 工作台说明与工具目录 scope 同步；共享诊断核心和 Studio 插件候选的原生/WASI 输入输出保持一致。插件仍为候选，不代表 Studio Host 已接受。

## 验证

- 使用本机 JDK 21 运行虚拟线程夹具并实际生成 JSON 转储；脱敏后保留为测试夹具。JDK 21 输出未提供 `virtual` 和 `state`，测试确认其线程类型保持未知，但栈帧仍包含虚拟线程实现证据。
- 覆盖明确 `virtual: true` 的较新格式、容器元数据、错误结构及输出大小上限。
- `cargo test -p zi-diagnostics-core --offline --locked jcmd_thread_dump_json -- --nocapture`：2 项通过。
- `cargo test -p zi-diagnostics-core --offline --locked sanitized_real_jdk21_json_fixture -- --nocapture`：1 项通过。
- 原生与 WASI 适配器通过 202 个字节一致向量（WASM 模块 800,196 字节）；同一组 202 项在 Wasmtime 49.0.0 的 10,000,000 fuel / 5 秒 / 64 MiB 限制下全部通过，最高消耗 9,085,458 fuel（`django.openapi-output-expansion`）。修正向量生成器，使 JSON 8 KiB UTF-8 边界样例通过合法容器字段填充保持可解析。
- `cargo fmt --all -- --check`、工具目录同步检查、Studio 插件映射检查及 `cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings` 通过。
- 本轮前完整 `cargo test --offline --locked --features ui-preview --quiet`：983 通过、27 忽略、0 失败。本轮 Rust 改动仅格式化测试断言；边界生成器改动已由 202 个原生/WASI 向量复验。

## 限制与后续

JSON 是适合工具消费的转储格式，不是所有线程在同一瞬间的一致快照。JSON 视图不替代传统文本解析中的 JVM 明确死锁信号，也不推断锁竞争关系。若未来 JDK 增加字段，应按显式 schema 证据扩展并添加真实样例。
