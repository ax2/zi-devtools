当前交互修订为0.1.1，模型选择保留、旧回答状态与结果操作详见[本轮记录](iterations/2026-10-18-agent-result-state.md)。

# Studio Agent 模型辅助 View 候选

**状态：未签名候选；Agent Runner 整体仍为 `await_host` / `not_delivered`。** 本 View 只用 Studio 暴露的模型目录和单轮生成能力提供一个小型文字辅助入口，不改变 DevTools 原生 Agent 工作台。

当前候选包位于 `release/agent-assist-preview-0.1.1-candidate-02/`，验证清单在同级 `agent-assist-preview-0.1.1-candidate-02-verification.json`；源代码位于 `plugins/agent-assist-preview/`。包根目录直接包含 `plugin.json`，符合 Studio 本地插件目录的结构。它要求 `model.catalog.read.v1 = 1.0.0-preview.1` 和 `model.generate.v1 = 1.0.0-preview.1`，并声明 `model.catalog.read`、`model.generate` 两项权限。View 是单一自包含 HTML；脚本与样式均内联，符合 Studio 将页面装入 sandboxed `iframe.srcdoc` 的加载方式，不依赖相对资源、网络、Worker、文件系统或外部进程。

此前 candidate-01 和 candidate-02 把插件根目录错误地嵌套在 `plugins/agent-assist-preview/` 子目录中，根目录校验会拒绝它们。candidate-03 已保留作历史审查；其生成按钮使用表单提交，在 `sandbox="allow-scripts"` 中不能可靠触发生成。candidate-04 改为普通按钮事件，不依赖表单提交权限。

模型目录只在用户点击后读取。用户明确提交生成时，Studio Host 应展示完整待发送目标、资料、模型名称及目标域名，并要求逐次确认；View 自身不声称验证资料来源。输入仅来自当前页面手动填写的目标和文字资料，最多 32 项、资料累计 32 KiB，目标最多 2 KiB UTF-8。资料和回答不保存到磁盘。

它不包含 MCP 发现、授权或调用，RAG 与可核验引用，文件/剪贴板输入，多步计划，批准后工具执行，任务中心、恢复或运行中取消。来源标签是提供方自报，不能作为来源证明。Studio 当前生成请求不支持运行中取消；请求发出后关闭 View 不能证明远端计算已经停止。

## 验证边界

- `node --test plugins/agent-assist-preview/tests/limits.test.cjs`：3 项通过，覆盖 UTF-8 目标边界、资料合计边界、数量、重复 ID、来源标签与输出/截止参数范围。
- `python scripts/verify_agent_assist_preview_view.py` 在本机 Edge 中运行实际 View，宿主夹具使用 `iframe.srcdoc`、`sandbox="allow-scripts"` 和完整 CSP。桌面浅色 1180×900、窄屏深色 390×844 均通过；验证用户触发模型读取、字节预算、完整生成请求、拒绝不显示结果、批准后纯文本安全渲染及 CSP 阻止网络请求。图片证据和哈希 proof 保存到 `target/agent-assist-sandbox-011-c02/`，不进入候选包。
- 浏览器夹具和模型都是本机模拟，不会调用模型网络；不能替代 Studio 桌面端原生确认对话框、权限撤销、profile 协商、安装签名和模型服务端端到端验证。该候选不代表 Studio 已接受。
