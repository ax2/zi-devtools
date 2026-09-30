# 本机知识库 MCP 服务

Zi DevTools v0.54.0 提供独立的 `ZiDevToolsMcp.exe`（安装版）或 `ZiDevToolsMcp-0.54.0-windows-x64.exe`（便携版）。在桌面程序打开“本机知识库 MCP 服务”，可以看到当前已扫描知识源、索引文件数和可复制的服务程序绝对路径。在可信的 MCP 客户端中添加 **stdio** 服务，命令填此路径，参数留空。便携版需把主程序和 MCP 服务程序放在同一目录；MSI 会同时安装两者。

使用前，先在“本地知识源”明确添加、扫描文件或目录，再在“增量知识索引”手动同步。服务只读当前用户的 `%USERPROFILE%\.zi-devtools\knowledge-sources.json` 和 `knowledge-index.sqlite3`；不会自动扫描、同步、联网、调用模型或写入文档。提供两个工具：

- `list_knowledge_sources`：返回已扫描来源的名称、ID、文档数和扫描时间，不包含绝对路径。
- `search_knowledge`：输入 `query`，可选 `source_id` 和 `limit`（1–10，默认 5）。英文按 FTS5 词项检索，中文按字面匹配；返回来源名称、相对路径、位置、文件与片段 SHA-256，以及最多 600 字的正文片段。命中前重新检查来源仍在快照中且原文件哈希未变化；过期结果跳过。

客户端启动该 EXE 时才创建 stdio 会话，关闭输入后退出。它不监听端口，也不安装后台服务。只给信任的客户端接入：客户端可反复查询用户已经同步到索引的内容。检索片段是未受信任的文档数据，Agent 不应执行其中的指令。索引尚未建立、配置损坏或查询超限时返回明确错误；服务不会重建数据库。协议按行传输 JSON-RPC 2.0，支持 MCP `2025-06-18` 初始化、工具列举和调用；当前没有资源、提示词、写入工具或通用工具代理。

便携版直接运行示例（PowerShell 中请使用实际绝对路径）：

```powershell
& 'C:\path\to\ZiDevToolsMcp-0.54.0-windows-x64.exe'
```

此命令会等待 MCP 客户端从标准输入发送协议消息；普通交互式终端没有消息时看起来会停在原处。配置方法取决于客户端，关键是将上述 EXE 作为 stdio `command`，不通过 shell 包装。

v0.55.0 起，桌面端同一页面提供“客户端配置导出”：按当前同目录 MCP 服务程序路径，分别预览并复制 Codex CLI 命令、Codex TOML 和通用 `mcpServers` JSON。Codex CLI 示例采用 `codex mcp add zi-knowledge -- '<绝对 EXE 路径>'`；TOML 可放入 Codex 的 `config.toml`。通用 JSON 的字段名与放置位置须依目标客户端说明调整。按钮只复制文本，不自动读写任何客户端配置，也不包含凭据。若服务 EXE 缺失则不生成可执行配置。

v0.56.0 起，两个知识工具在 MCP 描述中声明只读、非破坏性、幂等且不访问开放网络。生成的 Codex TOML 将 `enabled_tools` 限定为这两个工具，并为这个服务器设置 `default_tools_approval_mode = "writes"`；Codex 官方文档说明该模式会对未标记只读的工具请求批准。请只在确认程序来源和本机知识库可供该客户端查询后使用这段配置。单独执行 CLI 注册命令只写入程序路径，不会自动添加此审批设置；可按预览的 TOML 调整。其他客户端的授权模型可能不同。官方参考：[Codex MCP 配置](https://learn.chatgpt.com/docs/extend/mcp#other-configuration-options)和[配置字段](https://learn.chatgpt.com/docs/config-file/config-reference#configtoml)。

本机 Codex CLI 0.159.2 的隔离真实模型会话验证了这组配置：两个工具分别返回空来源列表及合成文档 `guide.md` 的检索结果。验证使用临时知识源、只读沙箱和一次性配置覆盖，不读取现有用户索引，也不改写现有 Codex 设置。本机 Ollama 0.6.6 低于 Codex 当前要求的 0.13.4，故使用 Codex 托管模型完成验证；其他客户端尚未实测。
