# DRF OpenAPI 本地 Schema 引用传播

日期：2026-10-18\
工具：DRF OpenAPI 契约对比 `1.3.0`\
程序版本：`0.82.0-dev.105`（本地开发不递增程序版本）

## 目标

比较引用了共享组件的 OpenAPI 契约时，将 Schema 字段变化传播到实际引用该 Schema 的端点，减少只看到孤立组件差异、漏掉受影响 API 的情况。整个过程只分析用户输入的 JSON，不联网读取外部引用。

## 实现与关键决策

- 对 `paths` 和 `components.schemas` 中的 Schema Object 递归解析同文档 JSON Pointer `$ref`，支持 JSON Pointer `~0`、`~1` 转义。
- 使用活动引用集合阻止递归展开；递归边保留占位符并加入 `referenceAnalysis.cycles`。外部 URI、锚点、无效/缺失目标和不适用的目标写入 `unresolvedSchemaRefs`。
- 仅在 Schema Object 上解释 `$ref`；示例数据里的同名字段与 Response、Parameter、Path Item 等 Reference Object 不会被当成 Schema 引用展开。
- 按规范版本区别相邻字段：OpenAPI 3.0 Reference Object 的额外字段不参与解析；OpenAPI 3.1 及以上 Schema Object 的 `$ref` 相邻关键字会保留为 `allOf` 约束。3.1 及以上允许布尔 Schema 作为引用目标。
- 每份输入的展开预算为 4096 次引用、50000 个节点、2 MiB 展开文本、64 层和 256 条诊断记录；越界时保留未展开内容并设置 `referenceAnalysis.<side>.truncated`。

## 验证

- `cargo fmt -p zi-diagnostics-core -- --check`：通过。
- `cargo test -p zi-diagnostics-core --locked`：17 项通过；含递归引用、远程引用不访问、转义 JSON Pointer、3.0/3.1 相邻字段语义、布尔 Schema 和超过 4096 次展开时的显式截断。
- `cargo clippy -p zi-diagnostics-core --all-targets --locked -- -D warnings`：通过。
- `python scripts/sync_tools.py --check`：目录 92 项已实现、62 项规划/开发中，检查通过。
- 重建原生诊断适配器与 `wasm32-wasip1` release 适配器；原生与 WASI 对 258 个诊断向量逐字节一致，含新增的 Schema 引用传播向量。详细非敏感回执：`target/openapi-refs-proof.json`。
- 实际页面入口 `Tool::Drf` 仍调用共享 `zi-diagnostics-core`，插件适配器同样调用该核心；本次未改变插件 Host 接受状态。

## 未完成与风险

本次只实现同文档 Schema `$ref` 传播，不处理外部文件、锚点、Path Item/Response/Parameter 等非 Schema 引用，也不执行完整 OpenAPI JSON Schema 校验。数组仍按整体比较；兼容性提示仍是保守提示，不等同于真实服务端行为验证。已运行诊断核心测试和双适配器向量，没有运行整个桌面工作区测试、全工作区 Clippy 或桌面 release 构建。
