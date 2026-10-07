# Studio 插件提供方交接

2026-10-07：接收到 Studio 会话的插件交付契约和优先接续要求。流程结果检查器检查点已完成；本轮已推进五项文本能力实际候选与完整目录迁移映射。

契约：`zicode.devtools-plugin/1.0.0-rc.1`。已一次性复制并逐项核对10件文件，连同索引共11件，位于 `contracts/studio-devtools/v1/`。索引SHA-256：`dd0688c009fa2d0600ab2d5af038a19efb5e1e26910619c1aede84aa668873f9`。此后构建/CI只使用本仓库快照，不读取Studio仓库，不引入跨仓path依赖、EXE启动或共享可变用户数据库。

试点：`com.zicode.devtools.text`，JSON格式化/压缩、Base64编解码、SHA-256五项。提取同一纯Rust核心用于独立egui和WASI command适配；View自包含HTML，按协议输入/结果和16条正反向量验证。compute.wasi.v1：2MiB模块、64MiB内存/10M fuel/5秒限制，文本8192 UTF-8字节，请求/结果48KiB。不把限额或静态样例视为实际Host验收。

交付状态：尚无插件产物或签名。无正式私钥时只交unsigned候选，不借用Studio密钥，不擅自发布。将输出包外verification.json、目录/模块大小和摘要及规范对照；包内不含自身包摘要。Host真实安装/UI/Pi Broker和proc_exit(0)/管道排空由接收方验收，提供方记录实际遇到的问题。resource/service/desktop profile待Host接口，不以路径直读/本地HTTP绕过。

全部 tools.json 条目都将记录可迁移、重叠能力整合、等Host接口或独立版未完成；不把独立版implemented当成插件完成。保持独立EXE及阶段档案，复用现有Cargo缓存，不增设重复target，不生成额外ZIP。

## 全目录首轮映射

[迁移映射](studio-plugin-migration.json)覆盖当前全部152项：32项纯计算迁移候选、58项等待Host接口、62项独立版整体未完成。重叠整合暂为0项，尚未确认Host已有同能力，避免推定。五项试点对应json/base64/sha256三项独立工具，全部pluginStatus仍not_delivered，acceptedCapabilities=0。脚本从tools.json同步工具版本/状态，CI执行 --check 防止漏项或陈旧映射。静态候选不代表整个界面/文件功能可直接迁移，需逐项抽核心和限定输入协议。

快照核验脚本：`python scripts/verify_studio_contract.py`，只依赖本仓库。`.gitattributes`禁用该快照的换行转换，以保存发布方原始摘要。16向量和5 capability数量已核对，初次核对时尚未构建实际WASI、缺少wasm32-wasip1且未发现wasmtime CLI；后续已复用共享工具链补齐target并构建实际模块，见下方更新。

## 2026-10-07：文本核心与实际模块

wasm32-wasip1现已装入既有工具链，仍复用D盘Cargo缓存和唯一共享target。实现crates/zi-text-core与crates/zi-text-wasi；根工作区default-members保持独立程序。独立JSON/Base64/SHA-256默认走同一算法，另有显式插件兼容模式执行相同有界请求。严格校验缺失/重复/额外字段、唯一处理器、UTF-8文本/请求/结果字节、JSON重复键/64层/安全整数和Base64 padding。

已构建实际WASI模块并通过36组原生/WASI逐字节对照，包括16条契约向量和接近48KiB的结果；已检查模块声明64MiB最大内存。Node只用于本项目功能验证，不是生产沙箱，也不验收fuel。stdout为单一结果，正常proc_exit行为记录在包外verification中，Host仍需正确处理成功退出与并发管道排空。

View位于plugins/text/views/main.html，自包含，无外部CSS/JS/图片。公开SDK测试替身连接实际WASI，桌面1100px浅色/390px窄窗深色通过；空输入、超限、错误、重复点击、Ctrl+Enter、结果重用、输入变化后忽略旧结果及无Host禁用通过。实际Studio桥、安装、撤权、升级/回滚、场景及Pi Broker未验收。无授权正式私钥，交付unsigned目录，不生成伪签名容器。

JSON/Base64/SHA-256源工具版本1.1.0，五项插件候选0.1.0；迁移表三项pluginStatus更新为candidate，acceptedCapabilities仍为0。独立版默认宽整数、重复键旧策略与解码外层空白保留；插件模式用契约严格规则。后续资源/服务/桌面profile仍等待Host接口。

实际最终模块220439字节，SHA-256 abfd0dbe45c064c4f8e40a5945c004694eb356cdf13688f8d7adf6536e64f20a；36组对照最大响应47191字节。当前模块在Node中_start正常返回，exitCode=0，未触发proc_exit；不据此宣称Studio的proc_exit(0)缺口已修复。候选目录计划release/text-plugin-0.1.0-dev57，包外text-plugin-0.1.0-dev57-verification.json，不生成额外ZIP。

## 候选交付完成（独立版源码6b23a71）

候选目录已生成：release/text-plugin-0.1.0-dev57，13件文件，总250054字节；外部release/text-plugin-0.1.0-dev57-verification.json，逐件摘要/大小、源码修订与组件摘要、契约摘要、36条WASI/原生结果和View QA齐全。packageSha256=null，目录摘要不冒充签名包摘要。所有capability/catalog/Pi绑定及源工具1.1.0核对通过。未提供或借用正式私钥；不发布signed容器。

独立GUI/MCP完整运行目录release/stage-82-text-plugin-6b23a71，程序0.82.0-dev.57；673/0/34、严格默认/全特性、8m29s优化构建、实际新EXE隔离服务启停、两主题原生Ctrl+Enter检查通过。Host真实安装、最大输出并发排空/fuel/权限、实际桥/Pi/scenes仍由Studio接收验收。当前模块正常_start返回，未调用proc_exit；不能用本候选推定其他插件的proc_exit(0)处理已修复。

## dev.58 文本便捷性（验证中）

当前源工具版本 JSON/Base64/SHA-256 1.1.1，目录版本 0.82.0-dev.58。原生操作前置、小窗口高度调整、兼容字节反馈与序列化前超限拒绝；View 增加大结果不可整段重用的说明，并实际检查六秒超时恢复。View 使用既有 WASI + SDK 测试替身通过两主题验证；原生最终检查仍在运行。

上一轮 release/text-plugin-0.1.0-dev57 和外部验证文件保留且未覆盖，仍对应源工具1.1.0与固定源码6b23a71。迁移清单 sourceVersion 表示当前源工具版本，candidate 不表示该新版本已重新打包或被 Host 接收；acceptedCapabilities 仍为0。源工具修订、旧候选交付与真实Host验收是分别记录的进展。

## dev.59 空文本结果（验证中）

当前目录0.82.0-dev.59。通用文本结果区增加完成状态，区分未运行/失败与成功空文本；随工具草稿切换保存，失败、清空、兼容模式及启动工具重置。28项共用结果界面的独立版本已同步；JSON/Base64/SHA-256现1.1.2。

原生两主题合成Ctrl+Enter及Base64空输入反馈已检查；模拟点击实际复制按钮并检查CopyText空字符串后拦截，测试不读写真实剪贴板。空结果仍不通过现有文本接力发送；此轮不声明接力/Host空文本支持。旧unsigned候选仍为dev57交付，未覆盖；acceptedCapabilities保持0。
