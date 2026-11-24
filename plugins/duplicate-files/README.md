# 重复文件检查插件

在 Studio 选择并确认目录后，Host 先按文件大小筛选，只读取同大小候选以计算 SHA-256。插件接收确认后的相对路径、逻辑大小与摘要，按大小和摘要分组。显示候选数、实际完成摘要数、重复组与理论重复逻辑大小。大小以十进制字符串传输。硬链接的不同路径按目录项计量，理论重复量仅供人工核对。

能力 `devtools.duplicate-files.scan` 属于 `com.zicode.devtools.duplicate-files`。声明 `resource.read` 和 `resource.directory.duplicates.read`，通过 `resource.wasi.input.draft2` 接收一份 `text/plain` 的 `resource.local-directory.duplicates.draft1`。Host 负责路径选择、逐次确认、显式取消和扫描。插件本身只读取交付的快照，报告中没有绝对路径，也不会修改文件。

快照最多 60 KiB，Host 预算包含 4096 个目录项、16 层、256 个候选、256 MiB 和 20 秒的软截止。报告区分扫描未覆盖与展示裁剪。按理论冗余大小降序并以摘要排序，最多展示 20 组，每组最多 12 个完整相对路径，所有展示路径共用 32 KiB UTF-8 预算。哪怕限制了展示，合计仍覆盖快照中所有已确认的重复组。

运行 `scripts/verify_duplicate_files_plugin.py` 比较固定原生适配器与真实 Windows Worker 向量。通过后用 `scripts/package_duplicate_files_plugin.py` 冻结未签名候选目录和包外校验收据。Studio 工具页和完整 Host 验收见 Studio #29。
