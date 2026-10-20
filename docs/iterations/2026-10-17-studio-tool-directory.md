# Studio capability directory metadata candidates

日期：2026-10-17\
范围：将本仓 vendored `tool.directory.v1@1.0.0-preview.1` 元数据真正装入 Inspect 与 Trace 新候选包，并核对此前交接文档与磁盘候选号不一致的问题。桌面程序版本保持 `0.82.0-dev.105`。

## 修改

- `scripts/package_transforms_plugin.py` 现在对 transforms/inspect/trace 候选强制检查目录 profile，写入逐能力目录快照、profile schema、别名映射和对应摘要；外部收据记录 profile 摘要，并把这些来源列入源文件哈希。
- Trace 提供方包升至 `0.1.1`，只增加 `tool.directory.v1` 搜索分类/用途/别名，不改变两项计算操作、WASI 模块或 View。重新生成 plugin manifest、catalog 和结果 Schema 证明。
- Inspect `0.1.1` 生成 candidate-02；Trace `0.1.1` 生成 candidate-01。旧 Inspect candidate-01 和 Trace `0.1.0` candidate-01 均未修改。没有补写旧收据中不存在的 Inspect 冷启动失败历史。
- 修正 handoff 中 Inspect 包号/数量与实际目录不符的记录，并补入两个新候选的大小、目录索引摘要和接受边界。

## 候选和验证

- Inspect：`release/plugin-candidates/inspect-0.1.1-candidate-02`，612 文件、2,343,823 字节；目录索引 SHA-256 `62de924883f6f0b246ee74fb34eea530b540e6ad7ccd40e5f1018ad830b8d391`。模块 SHA-256 `1db3fc37df01cd3a8371320d4f897a7fdf750c87e6202b736ed6870f046b8f29`；121 条功能、fuel、冷启动证据绑定该模块，冷启动失败 0，最大 fuel 6,918,538。
- Trace：`release/plugin-candidates/trace-0.1.1-candidate-01`，352 文件、1,428,569 字节；目录索引 SHA-256 `2e8b35249384757bc1c809bf971e2feccfc9396211a4efde7ca4f29fda9820c0`。模块 SHA-256 `bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e`；59 条功能、fuel、冷启动证据绑定该模块，冷启动失败 0，最大 fuel 7,932,681。
- 运行目录接收验证：包内文件集合与外部收据完全相等，逐件大小/SHA-256、目录索引、模块哈希、fuel/cold证明和目录快照均重算通过。两包要求 `tool.directory.v1@=1.0.0-preview.1`，schemaVersion=2。
- `verify_tool_directory.py` 检查两个生成包和两份源 manifest；`package_transforms_plugin.py` 也重跑 source sync 与 compute contract 检查。旧候选 SHA-256 未被覆盖。

## 边界与未完成

这两个 unsigned provider directory 只证明本机模块、View、目录元数据和独立 Wasmtime 预算记录一致，不证明 Studio Host 发现/安装、Pi 授权、Worker 隔离、更新/撤权/卸载或实际 UI 接受。`acceptedCapabilities` 保持 0。没有创建签名包、正式 Release、安装包、ZIP 或 app 版本。
