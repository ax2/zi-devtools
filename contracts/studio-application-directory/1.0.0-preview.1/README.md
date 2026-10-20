# 应用目录公开契约快照

application.directory.v1@=1.0.0-preview.1。upstream-contract.md为只读获取的公开提供方协议说明，摘要见provenance.json；applications.json为本仓库提供方映射，不是Host预置工具表。构建和测试仅使用本快照，不导入Studio代码。

仅主activity声明目录；12分类与tool.directory.v1一致。别名最多16个、各128 UTF-8字节，NFKC/小写/去空白去重；描述最多512 UTF-8字节。不得含控制字符和额外字段。整包应用发现不会自动获得权限或成为可执行capability。
