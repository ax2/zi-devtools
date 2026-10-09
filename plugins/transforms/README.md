# ZiDevTools 文本转换插件候选

独立分组入口包含 9 个工具的 23 个操作。算法来自独立桌面版的共享 Rust 核心；插件不调用桌面 EXE、HTTP 服务或其他项目源码。桌面版原有输入范围和行为保留，插件采用有界文本协议。

| 独立版工具 | 插件操作 | 范围与语义 |
| --- | --- | --- |
| URL 编解码 | `url.encode`、`url.decode` | UTF-8 百分号编码；解码保留 `+`，无效百分号片段按既有行为保留，无效 UTF-8 报错 |
| HTML 转义 | `html.escape`、`html.unescape` | 五种既有实体：`& < > " '`；不是通用 HTML 实体解析器 |
| 字符串转义 | `string.escape`、`string.unescape` | JSON 字符串文本；还原输入必须是合法 JSON 字符串，拒绝对象、孤立代理码点 |
| 命名转换 | `case.camel`、`case.pascal`、`case.snake`、`case.kebab` | 按非字母数字字符分词；延续既有行为，已有 camelCase 不额外拆词 |
| 文本统计 | `stats.inspect` | UTF-8 字节、Unicode 标量字符、行、空白分隔词统计；字符不是字素簇 |
| 行处理 | `lines.deduplicate`、`lines.ascending`、`lines.descending`、`lines.nonempty`、`lines.trim` | 稳定去重、排序、去空行、去行首尾空白；输出行尾统一 LF |
| 进制转换 | `number.from2`、`number.from8`、`number.from10`、`number.from16` | 有符号 128 位整数；各输入进制均输出 2/8/10/16 进制，保留符号、前缀和下划线输入行为 |
| Hex | `hex.encode`、`hex.decode` | UTF-8 字节和大写空格分隔 Hex；还原接受 ASCII 空白，拒绝无效 Hex 和 UTF-8 |
| URL 拆解 | `url.inspect` | 绝对 URL 结构与重复查询项；凭据仅提供存在标记，不输出用户名和密码 |

上述覆盖指这些独立版工具的全部既有转换操作在插件预算内可用，不代表整个工具目录已迁移，也不代表 Host 已接受。

## 使用

通过一个活动入口访问所有操作，按编码、文本、数值和结构分组。支持名称/工具 ID/操作搜索；`Ctrl+K` 聚焦搜索，`Ctrl+Enter` 执行。每个操作在当前页面内保留输入和结果；修改输入标记旧结果，失败保留内容，过期响应不会覆盖新输入。

“选中结果”后可用 `Ctrl+C` 复制。“作为下一步输入”在当前操作中复用结果，随后切换操作仍保留各自草稿；超过输入预算的结果不能整段复用。页面不会自动读取或写入系统剪贴板。关闭页面后草稿不恢复。

## 契约与限制

- `com.zicode.devtools.transforms`，能力前缀 `devtools.transforms.`，初始候选版本 `0.1.0`。
- 冻结 `1.0.0-rc.1` 外层和 `input.text`。输入最多 8192 **UTF-8 字节**；请求及序列化结果各最多 48 KiB。JSON Schema 字符长度限制不能替代运行时字节校验。
- `sceneId` 长度 1–128。错误枚举沿用 `INVALID_INPUT`、`INPUT_TOO_LARGE`、`INVALID_ENCODING`、`UNSUPPORTED_OPERATION`、`INTERNAL_ERROR`，不自行扩展契约。
- 输出膨胀超过预算返回已有 `INPUT_TOO_LARGE`。进制和统计报告装在文本结果中，不将 128 位整数转成不精确的 JavaScript 数值。
- 运行预算：模块 ≤2 MiB，10,000,000 fuel、5 秒、64 MiB 内存、2 MiB guest stack。页面等待上限不是 Host 取消保证。
- 无文件、网络、命令、时钟、随机数、数据库或模型访问。可选 `pi.tools.register` 需 Host 明确授权；清单声明不等于 Pi 或授权生命周期已验收。
- URL 查询、片段等用户输入可能含敏感内容；结构拆解仅省略 URL 的用户名/密码字段，不保证对查询内容自动脱敏。
- 交付为无签名候选目录及外部校验索引，不是签名安装容器，不承诺生产 Host 安装、撤权、更新、回滚、卸载和 Pi 验收。

## 可重复验证

`scripts/sync_transforms_plugin.py --check` 校验共享核心注册表与清单、目录、页面、Pi 声明一致。`scripts/verify_transforms_wasi.py` 使用独立预期向量和实际 native/Node-WASI 适配器，输出可移植原始请求、预期结果及 SHA-256，供独立预算运行器重放。Node 的一致性验证不等于安全沙箱或生产 Host 验收。

本地开发复用既有 Cargo 配置与缓存，不因每轮开发自动修改桌面版本或创建正式 Release。
