# 字段插件固定矩阵冷启动复核

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
插件范围：`zi-fields-wasi/0.1.0` 实验适配；未交付 Studio

## 结果

- 用当前 WASI 模块 `target/wasm32-wasip1/plugin/zi-fields-wasi.wasm`（SHA-256 `8636ab931e5ed9fa93e47993f8e8e819cd4b9a631ceaa434da9aa725f8f96ac9`）及现存 native adapter 重建 219 项请求夹具。native/WASI 精确输出和结果 Schema 全部通过。
- 使用 Wasmtime Python binding 36.0.0、原生引擎 36.0.2、原 10M fuel、64 MiB 内存、2 MiB guest stack 和独立进程外部 5 秒期限，219 项冷启动全部通过；最高 856.873 ms，最慢项为 `outer-missing-capabilityId`。
- 原始 v4 的 218/219、其中一项 5037.761 ms 超时，以及 v3 的 210/219 记录均保留。一次新完整通过不抹除先前超时，也不代表所有运行环境都可稳定通过。
- 首次复跑因 provenance 指向同哈希的另一份 Wasmtime DLL 路径，被验证脚本按路径一致性检查拒绝；该失败不计为产品结果。随后将 provenance 绑定到 Python 实际加载的 DLL，并以单项预检通过后再执行完整 219 项门禁。

## 证据与限制

- 运行结果：`target/fields-cold-audit-e7c8946233ce4b18859add85554ad322/cold-proof-revalidated.json`。
- 固定功能证据：同目录 `functional-proof.json` 与 `fixtures/`；仅含合成输入、结果、哈希和耗时，不含凭据。
- 更大合法正则仍未满足当前 WASI 预算：成本对照中 `a{300000}` 为 309192660 fuel，且超出本轮 256 KiB 字面量加速边界。固定矩阵通过不代表完整正则语法的 10M fuel 保证。
- 没有 View、Host 协议协商、生产 Worker 冷启动、安装/权限生命周期、Pi、签名包或候选交付证据；迁移状态保持 `not_delivered`。
