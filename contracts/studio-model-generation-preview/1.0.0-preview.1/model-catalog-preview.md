# 插件模型目录读取预览契约

状态：`model.catalog.read.v1 = 1.0.0-preview.1`。这是一个独立的只读 Host profile，**不代表** `service.v1`、MCP 或任意网络访问已支持。需要生成时另行协商 [model.generate.v1 预览](model-generation-preview.md)。

插件清单须使用 schemaVersion 2，在 `requiredProfiles` 声明 `"model.catalog.read.v1":"=1.0.0-preview.1"`，并在 required 或 optional 权限中声明 `model.catalog.read` 及用途。用户启用插件并授予该权限后，插件 View 可调用 `window.zicode.services.models.list(cursor?)`。不支持从 WASI Worker、Pi 扩展或普通网页直接调用。目录查询不触发模型请求、不读取凭据、不执行工具。

View 的请求只带可选分页游标。可信父页面从当前 Host 运行目标注入 runtimeId/sessionId 和安装的插件身份；Host 再核对桌面令牌、活动运行时、当前场景的插件启用状态、profile 与权限。插件请求中的伪造 pluginId、sceneId、workspaceId 不参与授权。切换运行目标后旧 View 请求失效；禁用插件或撤权后新请求被拒绝。

Host 返回 `{"schemaVersion":1,"operation":"models.list","result":{"models":[...]}}`。每条模型只有 `modelRef`、`displayName`、`providerKind: "host_managed"` 和 `supportsCancellation: false`。每页最多 128 条，可通过 `nextCursor` 继续；结果不含 provider ID、模型原始 ID、连接地址、密钥或配置文件路径。`modelRef` 是按 Host 进程密钥、场景、插件与运行实例生成的不透明引用，**不是**模型调用授权；只有另行获得生成权限并经用户逐次确认，才可作为生成请求的模型选择。当前分页游标是偏移量，配置在翻页期间变化时应重新加载目录，稳定快照游标留待正式版。

边界：模型配置仍使用 Studio 现有场景配置读取器；此预览不会使外部插件访问配置文件。模型生成有独立预览 profile；需要长任务取消、来源证明或更多 Host 服务的 DevTools 工具仍等待 `service.v1` 完整协议。提供方不能把这个 profile 计为完整 `service.v1` 支持。
