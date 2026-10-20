# 2026-10-18 插件目录能力补齐

## 目标与现状

继上一轮修复Trace迁移表后，现场确认JSON Object 2项、Transforms 23项能力缺少tool.directory.v1声明。用户希望工具可分类、搜索、便捷访问；本轮补齐这25项提供方目录信息，不更改接收方设置、权限或JSON Object停用状态。

## 修改

- 保持公共目录schema不变，扩展提供方capabilities映射，从17项增至42项（另外Diagnostics 11项由其生成器独立维护）。分类为text/data/web等既定类别，补中英文别名及明确用途/输入范围，不把文本转换说成文件操作或联网操作。
- Transforms和JSON Object包版本改为0.1.1，仅目录修订；独立工具算法/版本和应用0.82.0-dev.105不变。
- apply_manifest合并而非覆盖requiredProfiles，保留JSON Object的output.schema.v1与result.json.v1；冲突的目录profile版本直接拒绝。
- 新增JSON Object同步脚本，目录来源统一；生成校验和全量47项Python测试通过。
- 增加目录修订打包：先验证基线包逐文件收据/明确索引编码，拒绝符号链接、junction和越界路径；只允许版本、manifest schema及目录信息变化，源工具版本、计算路由、输入输出schema、权限和模块保持一致。保留基线receipt/manifest/catalog和运行证明，记录所有改变文件的前后摘要。已有候选不覆盖。

## 真实验证与发现

新包静态预检及逐文件/索引检查通过。JSON Object第一份元数据候选的旧复核脚本仍要求精确旧profile字典，不能识别新增directory声明；新的candidate-02修复profile兼容后重放10项成功，但检查脚本发现其会回写封存夹具，并在无native适配器时误打印native/WASI一致性。

最终candidate-03修复了上述复核行为：包内模式仅读取已有夹具，报告nativeCompared=false并打印WASI packaged-result replay；拒绝将证明输出写入候选自身。实际重放10项通过；43个包内文件修改时间全部保持不变；包内输出负向测试被拒绝。之前candidate-02重放回写的是完全相同的请求/期望字节，未改变内容摘要；前序候选仍留作历史，不作为当前映射。

Transforms在新的包目录中重放全部174项原封存请求，Wasmtime 36.0.2实际输出与期望字节一致，预算10M fuel/5秒/64MiB保持原值，failures=[]。本轮未重新编译Rust或复制构建缓存；原有native/WASI、cold和View证明按模块/页面字节完全相同继承，明确不是再次执行这些历史检查。

## 最终交付

- JSON Object：`release/plugin-candidates/json-object-0.1.1-candidate-03`，43文件、371860字节；索引SHA-256 `ea76508d7e0987ae914730bcaeabdf1988570687cc5227be2a30e377635981cc`。
- Transforms：`release/plugin-candidates/transforms-0.1.1-candidate-01`，804文件、4421575字节；索引SHA-256 `4f2c7cc1c3b70ba0ae05344bf0eb83b54d17f8f7e661d4ae808a35da934e25b3`。
- 两包外部收据分别为同名`-verification.json`，原始基线及所有旧收据保持原样。
- 目录修订：`release/plugin-candidates/devtools-catalog-2026-10-18-d4e432843b27474e`，11文件、2271695字节；索引SHA-256 `b219264ee0b5d429aab61e9673fdd143ebecd9d3838d365d7984a291dabd40af`。接收方只读语义验证verified=true。
- 源catalog SHA-256 `b179fc23896f4fdc8ba220c547a86994d613fb407d6aa920a6d353fe2c8d5e20`保持不变；migration SHA-256 `5a909e9f64b3a538a5a1c7e1f2f083b36e9fcaf00622957ce8b3c92b4ee84daf`。
- 六包53项能力逐项目录结构检查及Host静态profile预检全部通过，issues=[]。对应31个不同来源工具，不能把53项能力当作53个完整工具。

## 证据与边界

本迭代技术档案包含json-object-final-replay.json、json-object-replay-readonly.json、transforms-replay/fuel-proof.json、all-mapped-directory-check.json及接收方目录核验结果。47项脚本测试覆盖目录profile保留/冲突、元数据覆盖、UTF-8长度、别名规范化，以及禁止runtime/权限/schema/源版本改变与路径越界。

没有签名、发布或安装插件；没有启用JSON Object，没有发送其他会话消息。完整Host接受仍0/154，尚有123项来源工具缺少交付操作映射；下一步逐项核对可迁移计算范围、Host接口缺口和未实现范围，并单独核对Spring Actuator候选的边界。生产Host/Worker/Pi、结果接力、跨平台与签名验收不能由提供方静态检查替代。
