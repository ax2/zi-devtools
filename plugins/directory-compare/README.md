# 目录内容对比插件

分别选择并确认左、右目录。Studio 按精确相对路径清点文件和文件夹，随后只为等长候选文件计算 SHA-256。插件收到相对路径、类型、逻辑字节和已有摘要，输出单侧、类型不同、大小不同、内容不同、未校验、检查中变化与读取失败。文件大小以十进制字符串传输。只在双方清点完整时确认单侧存在；内容哈希预算耗尽时，也能保留先前已确认的单侧结论。

能力 `devtools.directory-compare.scan` 属于 `com.zicode.devtools.directory-compare`。声明 `resource.read` 和 `resource.directory.compare.read`，通过 `resource.wasi.input.draft2` 读取一份 `text/plain` 的 `resource.local-directory.compare.draft1`。Host 负责选址、确认、取消、清点和读取边界。插件报告不包含绝对路径和文件原文，功能为只读对照。

快照最多 60 KiB、256 个路径对。报告保留快照中的全部分类计数，相同项只计数。结果展示超过 48 KiB 容量时按相对路径顺序省略整条差异行并给出裁剪标记。扫描覆盖和报告展示分别标记。独立版的 CSV 导出、复制计划及文件写入是另行交付的范围。

用 `scripts/verify_directory_compare_plugin.py` 运行固定原生与生产 Windows Worker 向量。用 `scripts/package_directory_compare_plugin.py` 冻结未签名候选和外部收据。Studio 双目录正常入口和授权验收仍在 Studio #29。
