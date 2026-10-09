# 格式与检查插件交付（2026-10-09）

`com.zicode.devtools.inspect/0.1.0` 为独立构建的 unsigned 候选目录，源码修订 `0353c01bbf58b46eb0bde2b0c001293f38014647`。桌面继续固定 `0.82.0-dev.105`；不发布正式 Release、安装包或 ZIP，不覆盖旧候选。

## 输入与操作

使用冻结 `1.0.0-rc.1` 的 `compute.wasi.v1`，只接受 `input.text`。请求 schema 仅特化包 ID，原始契约快照不变。未知/重复字段、非法 UTF-8、命令输入及外来能力严格拒绝。

| 工具 | 完整既有操作 | 能力后缀（前缀 `devtools.inspect.`） |
| --- | --- | --- |
| YAML / JSON | 双向转换，重复键后值覆盖 | `yaml.to_json`、`yaml.from_json` |
| CIDR | IPv4 网络、掩码、范围、地址数，含 /0、/31、/32 | `cidr.inspect` |
| JWT | Header/Payload 解码与对象检查，保留宽整数文本 | `jwt.inspect` |
| Unicode | 码点/UTF-8/常见不可见字符、NFC、NFKC | `unicode.inspect`、`unicode.nfc`、`unicode.nfkc` |

独立版与插件共用 `zi-inspect-core`，不调用其他项目源码、EXE 或 HTTP 服务。JWT 不验证签名、有效期或可信度；NFKC 可能改变语义；常见字符提示不是完整安全检测。

插件输入 8192 UTF-8 字节，请求及完整结果各 48 KiB；模块 2 MiB、内存 64 MiB、guest stack 2 MiB、10M fuel、五秒。Unicode 报告仅对必然超过序列化结果预算的输入提前拒绝。独立版原有 YAML/JSON 1 MiB 输入、JWT 分段 64 KiB、Unicode 10000 码点及 8 MiB 文本结果范围保留。插件不包含文件、服务、截图、网络或模型权限。

## 冻结目录与验证

- 目录 `release/inspect-0.1.0-candidate-01`：493 文件，2139627 字节。
- 外部索引 `release/inspect-0.1.0-candidate-01-verification.json`；目录索引 SHA-256 `5e08499c3c924ea6fd6744bca863adea0fc397295ead36007bd7d44204b34f84`。
- 模块 617010 字节，SHA-256 `d59d513481d45a68541dade32989032f7bb1d49c5bd4376595cdb53e9ab0b294`。
- 94 条独立预期 native/WASI 字节对照覆盖全部 7 操作、解析语义、边界和错误。旧文本/transforms 共 210 条也通过共享协议重构。
- 独立官方 Wasmtime C-API 36.0.2：94 条预算全通过，最大 fuel 6918538、执行 9ms、线性内存 2752512 字节；94 个独立冷启动全通过，最慢 4667.826ms，所属进程全部关闭，未扩大期限。
- 实际 JSON Schema 验证包内目录、94 结果、31 成功请求与输入；493 文件大小和摘要复核通过。
- 注入公开 SDK 并执行实际 WASI 的两主题/窄窗 View 检查通过：搜索、快捷键、逐操作草稿、错误保留、过期响应、空结果、宽整数、复用及离线状态；截图已检查。不是实际 Studio SDK 或原生 GUI 验收。
- 工作区 824 通过、0 失败、34 忽略；格式和严格 Clippy 通过。优化桌面构建完成；新 EXE 隔离服务启停通过，端口释放。

`source.dirty=true` 如实记录七项原有未提交修改，未提交或覆盖它们。包内所需源码逐件记录摘要。初始 YAML 预期错误、Unicode fuel 超限、Clippy 测试模块顺序、SDK 测试替身错误分类及复核默认编码失败均留存本地档案，最终结果绑定实际模块和页面摘要。CI 已加入同步、实际对照与 schema 门禁，远程结果另行记录。

## 接收范围

交付不是签名安装容器或生产 Host 接受。实际 Studio 安装、更新、撤权、回滚、卸载、Worker 管道/冷启动及 Pi Broker 授权由接收方验收。当前 View 复用当前操作的结果，不宣称跨工具、跨场景接力全部实现。

全目录保留 154 项：41 计算候选、50 等待 Host 接口、63 独立版整体未完成，完整接受仍为 0。regex/diff 多输入须先版本化规范；时钟、随机、网络、写入、模型不能隐含加入本轮文本协议。
