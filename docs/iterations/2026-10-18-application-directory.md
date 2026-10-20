# 模型辅助与Spring Actuator应用发现 0.1.2

## 目标与实际缺口

工具数量增多后，应用入口也需要分类、搜索、收藏和最近访问。当前Agent Assist的2项和Spring Actuator的4项是View里的操作；它们不是可调用capability，也没有已支持的单操作定位协议。0.1.1候选缺少已有application.directory.v1的整应用目录元数据。

## 已实现

两包均单独升至0.1.2：在主activity增加application.directory.v1@=1.0.0-preview.1。模型辅助使用ai分类、模型辅助/AI问答/Agent Assist/model assistant别名；Spring使用system分类、Spring健康检查/Actuator诊断/Spring health/service diagnostics别名。用途明确逐次确认和有限功能范围。

新包仅更新manifest、catalog、README及审计附件。View HTML、其他执行与schema内容保持冻结0.1.1基线字节；权限、路由、输入输出、确认流程不变。增加独立元数据修订打包器，拒绝虚假capability、未实现profile、来源范围漂移、权限与入口变化及覆盖冻结目录。公开应用契约保存在contracts/studio-application-directory/1.0.0-preview.1，仅依赖本仓库快照。

## 六项操作提案

proposals/studio-view-operation-directory/2026-10-18包含六项稳定ID、标题、类别、别名、用途、主View与拟定位目标。每项给出已定位、未知ID、慢回调、停用、版本变化五种情况，共30条向量。输入不携带用户资料、origin或凭据；定位不自动调用HTTP或模型，也不授予权限。

特别约束：迟到回复不仅应被Host忽略，View改变焦点前也须校验请求仍有效。具体取消/有效期机制需Host确定；未临时创造SDK回调，未声明view.operation.directory.v1。此提案结构通过检查，但30条向量尚未执行，不是运行证据。

## 验证

- 70项Python回归通过，包括目录别名规范化/长度/控制字符、元数据唯一变更、伪capability/权限/来源漂移拒绝及提案交叉引用检查。
- Agent的3项Node限制测试通过；Actuator View验证4项成功、拒绝/profile、6项错误、9项输入防护、3项origin及取消等待commit终态通过。
- Actuator合同验证：32 JSON、22 schemas、6本地引用、21 schema向量、10 cursor、7 event、8 confirmation、10 resource向量通过。
- 两包profile静态预检无issues；两个包与总目录均通过Studio只读完整性校验。
- 工具目录同步及check通过。Rust/UI执行代码没有变更，未重复全量编译；上一轮已有1337项Rust测试、fmt、严格Clippy与release通过证据，见图片输入契约迭代。

## 冻结交付

| 候选 | 文件/字节 | 目录索引SHA-256 |
| --- | --- | --- |
| agent-assist-preview-0.1.2-candidate-01 | 8 / 51043 | 06e6179b69db5c2ea443c1a9c9835df7cf4fb98603643b7fed15d88d982db910 |
| spring-actuator-0.1.2-candidate-01 | 44 / 211064 | e0f4f37138c0b8ce2ae50cecb6f68f0a9333904ab49ed302176cffe7fdeb6425 |
| devtools-catalog-2026-10-18-eee5a2fbeb430f20 | 15 / 2340703 | 39422c2677bb56d1d6c026f85d0309ab48503d49cd6382ad815547063fad6264 |

目录均在release/plugin-candidates，收据在相邻目录外。10包覆盖34项来源的部分范围，55项能力声明/54唯一ID（URL为替代提供方），6项View操作，120项暂无候选；完整Host接受仍0/154。View操作数量不代表可独立搜索、收藏、最近或Pi调用的数量。

## 边界与后续

本轮只改善2个整应用入口，真实Studio安装、启用、应用目录渲染和原生交互仍未由提供方证明。历史0.1.1浏览器交互证据仅作为相同HTML的基线，不转写为0.1.2原生验收。待Host支持单操作合同后再实现六项定位并执行全部向量。

独立DevTools应用保持0.82.0-dev.105，没有发布、安装、签名、清缓存或改动Studio源码。JSON Object继续禁用。没有向Studio会话发送消息。
