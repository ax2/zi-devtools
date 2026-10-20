# 2026-10-17 格式与检查插件候选 0.1.1

## 范围

把已有颜色、YAML/JSON 双向转换、IPv4 CIDR、JWT 内容检查及 Unicode 检查/规范化共 5 工具、8 操作打包为可审阅的独立 Studio 提供方候选。保持独立桌面 UI，使用同仓共享 Rust 核心；候选是提供方交付，不是 Studio Host 接受。

## 实现和验证

- `plugins/inspect` 清单与 8 个操作由 `zi-inspect-core` 注册表同步；实际 View 经 SDK 形状的本地桥接调用 WASI 模块。
- native/WASI 121 条独立预期请求/结果逐字节相同；最大模块 623,155 字节。Wasmtime 36.0.2 独立预算 121/121 通过，燃料上限实测最高 6,918,538；内存不超过 64MiB。
- 121/121 全新 Python/C-API 进程冷启动通过，最大 442.297ms；这不是 Studio Worker 冷启动证据。JSON Schema 检查覆盖 121 结果和 43 个成功请求/输入。
- Chromium 浅色 1180×900、深色窄屏 390×844 实际调用 8 项 WASI 能力并逐项比对输出；另验证颜色预览及接力预览/取消不触发能力调用。两张截图保存在 `target/inspect-011/`。
- 冻结 unsigned 候选 `release/plugin-candidates/inspect-0.1.1-candidate-01`，606 个文件、2,319,279 字节；包外收据逐项绑定 42 个源码文件和 606 个包文件摘要、模块/fixture/运行器/schema/View/cold 证据。`packageSha256` 为 null，因为没有签名容器；不以目录摘要冒充包摘要。
- 首次预算验证没有传 `--text-results`，报告适配器尝试把文本作为 JSON 解析。使用脚本既有 `--text-results` 模式后完整 121 项预算通过。View verifier 初次误将 WASI 运行器的 stderr 遥测当作错误；修正为验证退出码和内存遥测后，两个主题全部通过。

## 遗留

Studio 实际安装/升级/回滚/撤权/卸载、生产 Worker 隔离及 Pi Broker 授权尚未由接收端验收；`acceptedCapabilities` 保持 0。JWT 输出只是解码，不证明签名或身份；CIDR 不执行网络扫描；Unicode 规范化可能改变语义。
