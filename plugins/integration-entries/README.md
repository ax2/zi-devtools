# 本机工具目录盘点

在 Studio 工具中心搜索「本机工具目录盘点」，切换到「文件夹」，选择并确认工作区内一个文件夹。确认后，只用这个目录的单层名称和类型检查原生工具内置的 13 组 Windows 入口，例如 Java、Node.js、Codex CLI 或 Git。文件名按 Windows 常见规则忽略 ASCII 大小写。同名文件夹和仅出现在子目录中的入口不会算入。

结果只显示已知的候选入口。出现匹配只说明该目录有同名普通文件。可预览完整结果，再确认保存一份新的 UTF-8 `.txt` 报告。不会命中未知工具，也不读取文件内容或记录未匹配的原始文件名。

每次确认一个工作区目录，插件处理最多 128 KiB、2048 项的单层名称快照。既有 `resource.read` 和 `resource.directory.read` 都须授予。Studio 负责目录选择、作用域限定、取消、撤权和保存确认。原生独立版的 PATH 全局发现、软件可运行性、端口和账户状态在此范围外。

## 构建与验证

复用 Cargo 缓存运行 `cargo build --offline --locked -p zi-integration-entries-wasi`，再以 `--profile plugin --target wasm32-wasip1` 构建同一模块。`scripts/verify_integration_entries_plugin.py` 比较真实 Windows Worker 和原生适配器的输出，覆盖原生白名单、普通文件、同名目录、恶劣名称和元数据边界。`scripts/package_integration_entries_plugin.py` 用候选的真实执行证据冻结独立包并生成外部验证文件。
