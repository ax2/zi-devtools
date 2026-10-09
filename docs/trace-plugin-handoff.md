# Java / Django 堆栈插件交付（2026-10-09）

`com.zicode.devtools.trace/0.1.0` 是未签名的提供方候选，源码修订 `9c8b38916d81cd47a98e9bcfac3cba796a66e0ad`。独立程序固定 `0.82.0-dev.105`，两项源工具为 `1.0.3`。不创建正式 Release、安装包或 ZIP。

## 完整范围与边界

独立版和 WASI 共用 `zi-trace-core`。`java.trace` 保留标准 printStackTrace 的独立异常、cause/suppressed 层级、主链、模块/Native/Unknown Source 帧及省略计数；`django.trace` 保留文本 Python Traceback 的因果链、路径/行号/函数、缺头/截断标记及 Django 规则提示。不执行项目，不将提示当作确认根因；不支持 HTML 调试页或 ExceptionGroup 树。

冻结 rc.1 契约不变：输入 8192 UTF-8 字节，请求与序列化结果各 48 KiB；模块 2 MiB、10M fuel、五秒、64 MiB 内存、2 MiB 栈。独立版完整报告保留原范围。固定扫描器与测试专用原正则比较捕获组、Unicode、CRLF、完整报告和错误；没有通过缩小算法范围适配预算。

## 候选及证据

- 目录 `release/trace-0.1.0-candidate-01`：346 文件、1,403,946 字节。外部同名 `-verification.json`，`packageSha256=null`；这是目录候选，不是签名安装容器。
- 模块 241,191 字节；SHA-256 `bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e`。
- 目录索引 SHA-256 `45dfdda2bb48f4878cc4166241db343bfc7a91c942e4fce3d7c6dd668b848ecd`。全部文件集合、大小及摘要再次复核一致。
- 59 条实际 native/Node-WASI 精确字节对照、独立 Wasmtime C-API 36.0.2 预算及全新进程冷启动全部通过。最大 fuel 7,932,681、执行 5 ms、内存 2,686,976 字节；最慢冷启动 504.374 ms，59 个子进程全部关闭。
- 包含 350/351/352/512 节点超限报告、513 节点解析错误、UTF-8 边界、重复键、类型及外层错误。超限检查以实际序列化输出为准。
- 实际 JSON Schema 验证 59 结果、19 成功请求及两个目录操作通过。
- 公共 SDK 注入并执行最终 WASI 的页面检查通过：两个操作、1180 浅色/390 深色、搜索、草稿、错误保留、过期响应、快捷键、选中复制与离线禁用。截图已检查；不等同实际 Studio 或原生 GUI 验收。
- 最终完整工作区测试 829 通过、0 失败、34 忽略；格式及全工作区/全目标/全特性严格 Clippy 通过。原生优化构建 19m20s，新 EXE 隔离服务启停 2171ms、exit0、端口释放。
- [主 CI 37928511885](https://github.com/ax2/zi-devtools/actions/runs/37928511885) 对修订 `c431fe97ed9cda865351984999d287cc2e23ec17` 已完成并通过。后续文档提交的 CI 独立记录，不套用此结论。

`source.dirty=true` 如实记录七项原有无关修改，未覆盖或纳入提交。初始正则预算失败、512 节点超时和内部中间结果测试假设错误均保留在本地档案；最终修复验证真实输出，没有删除边界用例或放宽预算。此前曾将其他工作流成功误记为主 CI：37889396659 是增量更新工作流，37889396595/37889895803/37890143305 的主 CI 因同一测试假设失败，已由 c431fe9 修复。

## 重放与接收

候选 `fixtures/parity/`、`fixtures/budget/` 保存原始请求、期望字节和摘要；附带 `verification/` 保存实际重放脚本与来源。执行附带 `verify_diagnostics_fuel.py --text-results` 保持精确字节对照；冷启动用 `verify_diagnostics_cold.py`。独立验证环境不属于产品依赖。源码同步用 `python scripts/sync_trace_plugin.py --check`，构建复用 `scripts/dev.ps1` 和现有 profile/target。

Pi 声明使用冻结字段 `capability`，必须由 Host 单独授权。实际 Studio 安装、更新、回滚、撤权、卸载、Worker 隔离/管道/冷启动以及 Pi Broker 均未验收。旧候选及 Inspect 0.1.1 三轮冷启动失败记录保持原样。

全目录仍为 154 项：41 计算迁移候选、50 等待 Host 接口、63 独立版未完成；完整接受为 0。字符工具的完整范围及资源/复制/保存授权另见 [设计](character-plugin-design.md)。
