# Studio 插件提供方交接

2026-10-07：接收到 Studio 会话的插件交付契约和优先接续要求。本轮正在完成流程只读结果检查器的安全验证检查点；随后优先五项文本能力试点与完整目录迁移映射。

契约：`zicode.devtools-plugin/1.0.0-rc.1`。已一次性复制并逐项核对10件文件，连同索引共11件，位于 `contracts/studio-devtools/v1/`。索引SHA-256：`dd0688c009fa2d0600ab2d5af038a19efb5e1e26910619c1aede84aa668873f9`。此后构建/CI只使用本仓库快照，不读取Studio仓库，不引入跨仓path依赖、EXE启动或共享可变用户数据库。

试点：`com.zicode.devtools.text`，JSON格式化/压缩、Base64编解码、SHA-256五项。提取同一纯Rust核心用于独立egui和WASI command适配；View自包含HTML，按协议输入/结果和16条正反向量验证。compute.wasi.v1：2MiB模块、64MiB内存/10M fuel/5秒限制，文本8192 UTF-8字节，请求/结果48KiB。不把限额或静态样例视为实际Host验收。

交付状态：尚无插件产物或签名。无正式私钥时只交unsigned候选，不借用Studio密钥，不擅自发布。将输出包外verification.json、目录/模块大小和摘要及规范对照；包内不含自身包摘要。Host真实安装/UI/Pi Broker和proc_exit(0)/管道排空由接收方验收，提供方记录实际遇到的问题。resource/service/desktop profile待Host接口，不以路径直读/本地HTTP绕过。

全部 tools.json 条目都将记录可迁移、重叠能力整合、等Host接口或独立版未完成；不把独立版implemented当成插件完成。保持独立EXE及阶段档案，复用现有Cargo缓存，不增设重复target，不生成额外ZIP。

## 全目录首轮映射

[迁移映射](studio-plugin-migration.json)覆盖当前全部152项：32项纯计算迁移候选、58项等待Host接口、62项独立版整体未完成。重叠整合暂为0项，尚未确认Host已有同能力，避免推定。五项试点对应json/base64/sha256三项独立工具，全部pluginStatus仍not_delivered，acceptedCapabilities=0。脚本从tools.json同步工具版本/状态，CI执行 --check 防止漏项或陈旧映射。静态候选不代表整个界面/文件功能可直接迁移，需逐项抽核心和限定输入协议。

快照核验脚本：`python scripts/verify_studio_contract.py`，只依赖本仓库。`.gitattributes`禁用该快照的换行转换，以保存发布方原始摘要。16向量和5 capability数量已核对，实际WASI尚未构建；当前机器缺少wasm32-wasip1 target且未发现wasmtime CLI，下一轮使用共享工具链/缓存补齐，不新建重复target。
