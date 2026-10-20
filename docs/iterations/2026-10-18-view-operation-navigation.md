# View 操作定位 0.1.3

## 目标与设计

模型目录、模型生成、Spring health/info/metrics/摘要六项入口接入公开view.operation.directory.v1@1.0.0-preview.1。工具中心搜索可定位具体按钮、端点或摘要；操作目录和capability分开，不自动发起模型/HTTP调用，原逐次确认权限保持。未修改Studio源码，未向其他会话发送消息。

处理器同步返回opened/not_found/unsupported；busy/loading拒绝切换，不排队异步焦点，pagehide注销。未读取模型时生成按钮禁用，因此定位生成区标题。Spring切换目标端点但保留Origin和旧摘要，旧摘要仍明确显示自己的端点，避免误认为新的请求已执行。

## 迭代发现

首次浏览器夹具在加载过程中读取Frame.title得到空标题；随后主世界document.title已有正确标题，证实加载观察存在竞态。给父夹具补UTF-8并等待目标document.title及处理器就绪，再执行页面身份检查。未通过放宽预期标题掩盖问题。

历史application-directory回归改读冻结0.1.2候选，继续验证元数据修订的严格边界，不用新View冒充旧运行时。新打包器校验0.1.2到0.1.3仅增加精确profile与六项映射，拒绝权限、capability、来源范围漂移；浏览器证据逐项绑定当前View和manifest哈希。总目录校验增加manifest声明和映射的一致性检查。

## 验证

- Browser plugin not available；现有Python Playwright和安装的Edge，opaque sandbox iframe，公开SDK模拟，connect-src none。
- https://provider-fixture.invalid/agent-assist-preview 与 /spring-actuator 为route.fulfill提供的本机合成页面，不是外部服务。
- 1180×900浅色、390×844深色：6操作×2视口共12组定位通过。页面身份、非空内容、无错误覆盖层、console/pageerror、焦点入屏与无横向溢出通过。
- 每项均检验零自动调用、保留草稿与旧结果；busy返回unsupported且不修改状态；未知ID返回not_found且焦点不动；pagehide注销；模拟确认前/拒绝后执行计数为0，显式同意后为1。
- 人工查看截图agent-assist-preview-dark-generate.png与spring-actuator-light-metrics.png，目标控件完整可见、窄屏无横向截断，旧health摘要与待选metrics端点区分清楚。其余截图由相同断言采集，未声称全部人工体验。
- 原Agent沙箱完整模型选择/结果交互回归通过（1180×900浅色，无外部页面请求）；Agent 3项边界测试、Spring成功/拒绝/错误/输入/取消最终态回归通过；75项Python测试通过。
- 两包公开manifest profile预检、逐文件外部只读校验、总目录外部校验、sync_tools --check及git diff --check通过。

## 不可变候选与收据

| 候选 | 文件数 | 字节 | 目录索引SHA-256 |
| --- | ---: | ---: | --- |
| agent-assist-preview-0.1.3-candidate-01 | 13 | 79409 | 9b7e7dee63fbdad60b178d597754c05dfd587a824874ae33282352b3ef010c63 |
| spring-actuator-0.1.3-candidate-01 | 49 | 246194 | 74f6bab2ed7c9ddbee2857aeac7cf4d875eda642900afbb0fb4fec31102f97cc |

均位于release/plugin-candidates，外部收据同名加-verification.json。新目录devtools-catalog-2026-10-18-ba45d264d48bf094，15文件2344893字节，目录索引663068653cac38ed6593a678c66fc09317f317f46965e8fd6f8a706969a87422。目录包含10个未签名包，6项View操作仍不算Agent可执行能力。

## 验证边界与后续

这是提供方真实浏览器渲染、模拟公开SDK的证据，不是真实Studio安装/导航传输/跨版本/工作区/停用/超时的验收；这些完整链路由Studio专门会话继续。历史协议向量保留为提案历史，不冒充新运行证据。完整来源工具接受仍0/154，未签名、未发布、未安装。

独立Rust桌面仍保持0.82.0-dev.105。本轮接力滚动修复1340项Rust测试、严格Clippy、release构建成功，详见2026-10-18-handoff-scroll-reset档案；插件HTML修改没有重编桌面Rust。
