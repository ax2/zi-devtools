# 插件模型生成预览契约

状态：`model.generate.v1 = 1.0.0-preview.1`，Studio 原生桌面 View 专用。它依赖 `model.catalog.read.v1 = 1.0.0-preview.1`，不等于通用 `service.v1`。提供方不能把该预览当成 DevTools 模型工具已完成接入。

插件清单需用 schemaVersion 2，在 `requiredProfiles` 同时声明上述两个精确版本，并声明 `model.catalog.read`、`model.generate` 权限及用途。两个权限都授予且插件在当前场景启用时，View 可先调用 `window.zicode.services.models.list()` 取得本次运行的不透明 `modelRef`，再调用 `window.zicode.services.models.generate(input)`。WASI Worker、Pi 扩展、普通网页和 LAN/公网客户端不能调用。

`input` 为 `{modelRef,goal,materials,maxOutputTokens,deadlineMs}`。`goal` 必须非空且最多 2048 UTF-8 字节；`materials` 最多 32 项，文本合计最多 32768 UTF-8 字节。每项含 `sourceId`、`displayName`、`sourceKind`、`trust`、`text`；其来源标签来自插件，**Host 不把标签当作可信的文件/结果来源证明**。`maxOutputTokens` 为 1–4096，`deadlineMs` 为 1000–120000。此预览仅接受 Studio 场景配置中明确列出的 OpenAI Completions 兼容模型，不能指定网络地址、凭据或任意模型 ID。

可信父界面从活动 View 注入插件、运行时和会话身份。Host 校验桌面令牌、当前运行目标、插件启用、profile 与权限，匹配 `modelRef` 后固定本次目标，发给父界面一次性 `grantId`、模型名称、目标域名与**实际待发送的完整 JSON 文本**。Studio 使用可滚动的本机选择对话框显示完整文本；用户选择发送后，父界面提交单次授权。拒绝即消费授权且不发网络请求。授权 120 秒过期，最多 8 个待确认；插件 View 从不收到授权 ID 或凭据。

执行由 Host 使用当前场景的凭据代理完成。授权对话框在待确认授权过期时自动关闭；插件 View 的调用等待预算覆盖最长确认期和最长模型请求期。禁用/卸载/重载插件或变更插件权限会清空待确认授权并推进撤权代数；调用结束后再次核对运行目标、授权和代数，失效结果不返回插件。请求禁止 HTTP 重定向，模型响应限 4 MiB，返回的文本限 24576 UTF-8 字节；错误只给固定代码，日志不记录待发送文本或凭据。

当前模型请求底层为阻塞 HTTP 调用，有超时但**不支持运行中取消**；View 关闭、用户撤权可阻止结果返回，不能保证立即停止已发出的网络请求。模型目录因此仍标 `supportsCancellation:false`。提供方 `service.v1` 草案中的 `consentId`、长任务事件、MCP、数据库、网络探针及跨工具来源证明尚未实现。DevTools 若要使用此预览，需由其独立签名插件 View 调用 SDK，不能从 WASI 伪造授权；完整模型工具验收还需实际提供方包与原生运行证据。
