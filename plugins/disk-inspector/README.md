# 目录空间分析插件

在 Studio 选择工作区内的文件夹并确认共享后，插件读取目录用量快照，计算文件逻辑字节、直属子目录的累计大小及最大文件。两张排行各显示前 20 条，相同大小按相对路径排序。逻辑大小按目录项计量，文件与汇总数以十进制字符串保持整数精度。

首次交付的接收方是 com.zicode.devtools.disk-inspector，能力为 devtools.disk-inspector.scan。运行时使用 resource.wasi.input.draft2 接收一份 text/plain 的 resource.local-directory.usage.draft1 快照，并使用 Studio 的 resource.read 和 resource.directory.usage.read 两项授权。快照只含相对名称、条目类型和逻辑文件长度。

Host 的快照最多 4,096 条目、16 层、2 MiB。候选 Worker 在现有预算内分析最多 64 KiB。超过分析预算时，汇总只计入已读前缀内的完整条目，并明确标记部分结果及已读范围。Host 的部分结果也保留；路径展示超过 512 UTF-8 字节时会标记截短。逻辑字节与磁盘物理占用是不同的数值。

使用 scripts/verify_disk_inspector_plugin.py 运行固定原生与生产 Worker 向量。用带对应 WASM 和实测回执的 scripts/package_disk_inspector_plugin.py 打包。候选冻结为未签名目录和包外校验收据。工作区外选址、Explorer 入口和 Studio 工具页实际验收归 Studio #29。
