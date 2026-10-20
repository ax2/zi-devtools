# 2026-10-18 Diagnostics 输出与发现契约（进行中）

## 目标与发现

接收方静态检查指出既有 Diagnostics 0.1.3 候选仅声明 output.schema.v1，11能力没有outputSchema/maxResultBytes。当前源码已处于0.1.4开发版本，仍有同样缺口。本轮完善0.1.4源声明，不覆盖旧候选、旧收据或旧目录修订。

## 实现

- 按本机只读取得的公共 output.schema.v1 1.0.0-rc.1 文档快照，为11能力内联严格成功/错误JSON envelope schema，并声明49152字节上限；schema只使用Host支持的oneOf、基础类型及字段约束，不使用ref/pattern。报告字符串内部语义继续由既有运行验证检查。
- 补充tool.directory.v1 1.0.0-preview.1：code分类、来源工具的中英文别名、仅分析报告文本的用途，避免将导入分析能力描述为完整桌面工具。
- 修复catalog中inputSchema路径与打包实际位置不一致；添加outputSchema路径/预算。生成器内容未变时不重写文件，减少不必要的Rust重编译。
- 原生/WASI验证器逐结果执行schema与UTF-8字节校验，证明记录manifest/schema摘要。打包器强制核对该证明，旧运行证明不能代替新manifest的验证。
- 新候选外部收据采用明确的path/size/sha256键序和目录索引编码，拒绝覆盖已有回执；NOTICE版本取自manifest。未发布或修改旧交付。
- 发现操作报告schema的generatedBy引用错误地指向operations/generated-by.schema.json，改为../generated-by.schema.json，并新增11个操作的离线引用解析/缺失来源字段拒绝测试。实际WASI一致性验证也检查报告结构。
- 包内新增独立输出契约验证器：检查封存请求/期望结果摘要、结果外壳和报告schema，明确runtimeExecuted=false，不能以复核旧证据代替运行新模块。

## 已验证

- `PYTHONPATH=C:\Users\ax2\AppData\Local\ZiDevTools\verification-python` 下39项脚本测试通过，包含9项新增诊断输出/发现/离线schema测试：11能力成功及5类错误、非法结构、ASCII/中文/转义后的精确字节边界、缺少声明的拒绝、搜索元数据和报告引用。
- `sync_diagnostics_plugin.py --check` 与Python语法检查通过。
- Studio现有 `check-plugin-manifest-profiles.mjs` 只读检查源码manifest：issues=[]、preflightOnly=true、hostAccepted=false。报告在`target/diagnostics-014-profile-preflight.json`。未改Studio工作区、未向其他会话发消息。
- 公共协议快照与来源摘要位于`contracts/studio-output-schema/1.0.0-rc.1/`。
- 独立Wasmtime验证依赖已安装至C盘用户缓存，官方36.0.2 C API归档SHA-256按既有固定值验证；归档只在内存消费，未保留ZIP。engine-provenance.json保存在本轮技术档案目录。准备环境不计作运行/预算通过。

## 未完成与下一步

录屏release构建会话29483已退出0，用时16分43秒；随后复用C盘用户缓存构建0.1.4原生/WASI适配器，未争抢Cargo构建锁。下一步运行完整一致性、fuel、序列化边界、冷启动与View检查，再生成独立候选和外部收据。迁移表仍指向已存在0.1.3候选，不提前改为未生成的0.1.4包。新声明通过静态检查不代表Host接受；签名、实际安装、授权和生命周期仍待接收方验收。

## OpenAPI 运行预算修复

首次0.1.4实际原生/WASI的258项字节一致性、结果外壳与报告结构检查通过，但独立Wasmtime预算验证中，350个移除端点的OpenAPI用例耗尽1000万fuel，其余257项通过。只增加输出累积预算仍不足以解决，两个失败证据目录budget与budget-optimized均保留。

新增无可达$ref文档的借用路径，避免复制整棵JSON和构造引用路径；保留原有深度、节点、展开文本及截断语义。独立重放原失败用例通过，消耗9,808,019 fuel，余量较小，不能以此单例代替完整回归。增补借用/引用回退与原解析器等价性测试，覆盖普通输入、深度/节点/文本限额和限额外引用；结合输出预算测试，诊断核心20项测试全部通过。

DRF OpenAPI工具版本更新为1.3.1，应用版本保持0.82.0-dev.105。新版本适配器已重建，正在重跑完整一致性、预算、序列化、冷启动与View证据；未生成新候选，未修改既有交付。

完整258项fuel和9项序列化边界在当前模块通过后，冷启动预检拒绝重复用例ID。原因是新增同一操作的成功样本及其衍生边界沿用操作名；现在为第2及以后样本增加稳定序号后缀，保留全部258项请求与语义断言。实际一致性、fuel与冷启动验证器拒绝重复ID，打包器进一步要求三份证明具有完全相同的用例集合及请求/结果摘要，拒绝诊断计时模式、未结束或失败用例。相关正反测试已通过；重放记录使用独立unique文件名，旧证据保留。

## 当前候选与验证结果

- 当前模块834852字节，SHA-256 `2df08c2a21a66a8598c9ec2b73493cf77d3e70c025ff305c79379a6303ec62b0`。
- `runtime-unique-proof.json`：258项原生/WASI逐字节一致，11能力成功/错误外壳及内部报告schema通过。
- `budget-unique/fuel-proof.json`：258项通过；最高9,808,019 fuel，最大内存3,014,656字节，未提高1000万fuel/64MiB/5秒预算。
- `serializer-final/fuel-proof.json`：9项ASCII/UTF-8/转义结果49151/49152/49153字节边界通过。
- `cold-unique-proof.json`：258项全新独立进程通过，无剩余/跳过/失败；最大1650.132ms。计时包含进程调度与引擎编译，仅代表本机独立C API运行，不是Studio启动时间承诺。
- `view-final-proof.json`：实际Edge浅色1180×900、深色390×844，各执行11能力，共22次真实WASI调用，无页面错误。已查看两张截图：宽屏输入/结果并排，窄屏纵向排列，文字、按钮及结果对比度可读；窄屏结果需纵向滚动。不是Studio真实桥接或权限验收。
- `release/plugin-candidates/diagnostics-0.1.4-candidate-01`：651文件、5,444,825字节，unsigned，目录索引SHA-256 `9755b9f55db6691feb4a461ebd4f108d183705dac081b958c5a2d7760c0146ff`。外部收据为同名`-verification.json`。Studio只读逐文件/索引检查verified=true，源码及包内manifest静态profile检查issues=[]。包内输出契约复核258+9项通过，明确runtimeExecuted=false。
- 新目录修订`devtools-catalog-2026-10-18-d084f541be736aed`：154工具、6包、31个候选来源工具，11文件、2,256,397字节；目录索引SHA-256 `61ce727c193bd204f4fb6ffbb376c0e7a4e01a2583c5f8c064784d2a41a1fa25`。接收方只读语义验证verified=true，accepted仍0。迁移表11个诊断操作改为新0.1.4目录与确切收据，并明确三个requiredProfiles；旧候选/目录修订未改。

源目录证明：catalog SHA-256 `b179fc23896f4fdc8ba220c547a86994d613fb407d6aa920a6d353fe2c8d5e20`；migration SHA-256 `c626522e043ffb4445c9be696d2049b75c4f16e3656b35b89063a2a9fb802c14`。

桌面全工作区测试、严格Clippy和release构建已按顺序启动，复用C盘唯一用户缓存；当前结果待收集。不发布、不安装、不发送其他会话消息，应用版本保持不变。

## 桌面回归补充

全工作区`dt-test`完成，60组结果合计1336通过、0失败、43忽略。第一次严格Clippy发现新增测试模块位于生产函数之前，触发items_after_test_module；已把完整cfg(test)模块移到文件末尾，未改变测试内容或生产行为。失败日志workspace-clippy.log保留，重新执行核心测试、全工作区Clippy及release构建，结果待收集。

候选01及目录修订已冻结，来源摘要保留打包时快照；随后仅调整了Rust测试模块的位置。该候选的258项运行证明仍绑定封存模块，不据此宣称任意后续重新编译二进制已验证。未覆写候选源码摘要或历史证明。

后续复查：核心20项测试通过，严格全工作区Clippy通过（18.73秒）；格式及目录同步检查通过。release构建已开始，当前仍在编译，不能视为完成；日志release-build.log，构建会话69406。

桌面构建结果补记：会话69406退出0，release构建7分36秒成功。ZiDevTools.exe为41,438,208字节，SHA-256 `6e66f889beb9027f956adeb725264f96a4e9cf250cc988e9065a17b8c0a9e546`，位于C盘既有用户构建缓存的release目录；未打安装包或发布。
