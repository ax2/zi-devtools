# ZiDevTools URL 文本资源接收候选 0.1.1

这是 `devtools.transforms.url.encode`（独立工具版本 `1.0.2`）的 Studio 资源接收适配器。它读取当前调用中用户明确授权的一项 `text/plain` 资源，并通过 `zi-text-core` 复用独立版相同的 URL 编码实现。它只处理 UTF-8 文本，接受 1–8192 个源字节；结果包含编码文本和源字节数。

插件声明 `resource.wasi.input.draft2` 与 `resource.input.types.v1@=1.0.0-preview.1`，同时声明 `tool.directory.v1@=1.0.0-preview.1` 与 `output.schema.v1@=1.0.0-rc.1`。只允许 `text/plain`，只请求 `resource.read`。guest 使用 `zicode_resource_input_draft1` 的 `size`/`read` 导入读取 Host 已授权的资源；WASI 标准启动还会出现 `environ_get`/`environ_sizes_get` 导入，但适配器不读取环境值，也不导入文件路径或网络 API。代码不访问文件系统、环境变量、网络或 Pi；不保存结果、不自动调用其他工具。Studio 用户可在插件执行后单独预览或保存结果。

这里的 `resource.wasi.input.draft2` 是 Host 侧的可信模块快照 framing profile；guest 内的受限资源读取函数仍使用现有 `zicode_resource_input_draft1` ABI。两者分别描述 Worker 输入帧与模块导入，不是版本冲突。

候选包 id 为 `com.zicode.devtools.url-text-resource`，其中 capability id 保持为已有 URL 编码操作的精确 id。该 capability id 与完整 transforms 插件中的 URL 编码 capability 相同；在隔离 Studio profile 中验证时，请仅启用其中一个提供方，避免重复注册。候选版本不代表 Studio Host 已接受该 profile 或该插件。

`fixtures/success-vectors.json` 与 `fixtures/runtime-negative-vectors.json` 用于验证编码和 guest 侧拒绝行为。`fixtures/host-negative-cases.json` 列出撤权、取消和过期授权的 Host 端负例；DevTools 提供方不能独立伪造这些 Host 状态，这些用例留给 Studio 隔离会话验证。候选为未签名目录包，不包含可安装签名容器。

本次0.1.1保留0.1.0 candidate-02的WASM与依赖许可证字节，补齐Host必需声明和中英文发现别名。输出schema保留成功分支并新增封闭的失败分支，覆盖7种稳定错误码；13项实际WASI用例验证结果schema、最坏百分号膨胀、UTF-8字节边界，以及错误请求在读取资源前拒绝。独立运行器使用10M fuel、5秒执行期限和64MiB内存；不计模块编译时间，不能替代生产Host资源授权、撤权与端到端验收。
