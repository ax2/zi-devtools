# 模型辅助插件 0.1.1：模型选择与结果状态（2026-10-18）

## 实际问题与修复

updateModelSelect 每次重建模型选项都会选中第一项；生成开始、拒绝和结束时都会调用它。用户选了第二个模型，当前请求仍携带第二个模型的引用，但界面可能跳回第一个模型，下一次请求也可能因此换模型。现以不透明 modelRef 保留选择；列表读取期间锁定表单，避免旧列表和新的模型选择交叉。生成期间 fieldset 锁定原本已存在，本轮增加真实浏览器断言，不把已有功能当作新实现。

回答与提交时的目标、资料、模型和参数关联。当前草稿变化后提示“对应此前的输入”；新请求处理期间显示“保留上次回答”，拒绝或失败后保留旧回答及提示。提供“查看上次回答”“选择回答文字”“清空回答”；选择用浏览器 Range，用户自行按系统复制快捷键，不调用剪贴板 API。清空释放当前回答并保留草稿，焦点回到目标框。回答继续使用 textContent，HTML 片段只显示文字。

修正深色主题错误/成功提示颜色。没有增加模型自动调用、持久化、MCP、RAG、运行中取消或其他权限。完整 Agent Runner 仍为 await_host / not_delivered。

## 浏览器验证

使用 frontend-testing-debugging 流程。Browser 插件及 browser skill 未提供（Browser plugin not available）；复用仓库既有 Microsoft Edge/CDP 端到端脚本，不安装浏览器依赖。目标流程为：连接本机模拟 Studio sandbox -> 用户读取模型 -> 选择第二项 -> 提交并拒绝 -> 再提交批准 -> 修改目标 -> 再提交拒绝 -> 跳转/选择回答 -> 清空并保留草稿。

环境是临时 127.0.0.1 HTTP 夹具和真实 sandbox=allow-scripts 的 iframe.srcdoc，connect-src none。1180×900 浅色与390×844 深色各执行一次模型目录读取和3次生成请求，所有生成均为本机模拟，不调用真实模型服务。

首轮截图发现夹具外层缺少 viewport meta，导致窄屏缩放桌面布局；补齐 viewport 并验证实际 iframe innerWidth 不大于指定视口后重新运行。这个发现否定了旧截图能证明真正窄屏布局的说法。截图阶段隐藏夹具操作栏、保留原 sandbox 并显示插件视口；分别检查首屏和结果区。

| 检查 | 结果 |
| --- | --- |
| 页面身份、标题和非空主界面 | 通过 |
| 首屏与结果区视觉检查 | 两种主题均检查；真实窄屏不横向溢出，文字及按钮可辨识 |
| 所选模型与实际请求一致 | 第二项 modelRef 进入请求，开始/拒绝/完成后仍选第二项 |
| 表单锁定与旧回答状态 | 通过；输入变化、等待新请求与拒绝保留均覆盖 |
| 结果跳转、选择全文、清空保留草稿 | 通过；焦点和选中文字匹配 |
| 不可信 HTML | img/onerror 仅为文字，没有新增元素 |
| Runtime.exceptionThrown | 0 |
| 网络 | 只有本机页面与可选favicon；主动 CSP 负向探针被拒绝，相关浏览器安全日志属预期 |

Node 3 项输入限制测试通过；脚本全量56项通过。第一次单独执行 unittest 缺少 jsonschema 搜索路径而失败，设置已有用户缓存依赖路径后56项全部通过。首次包校验误在打包收据尚未生成时启动，报告“收据缺失”；等待原打包进程正常退出后重新校验通过，没有重启构建。

## 候选与目录

- 新包：release/agent-assist-preview-0.1.1-candidate-02，6文件、41270字节。
- 原收据为同目录同名 -verification.json；结构化索引收据为 release/plugin-candidates/agent-assist-preview-0.1.1-candidate-02-verification-revision-01.json。
- 包索引 SHA-256：28cef544d9a46c264763d5c3ac0e447adc224bec989df2ccacacc83462af0f3a。
- 目录快照：release/plugin-candidates/devtools-catalog-2026-10-18-794fe75cdcb85480，13文件、2385249字节。
- 目录索引 SHA-256：068ee0b86a459de0e3370822d20e78d270b359d0b62eced4e38a3c4328c8465c。
- 8个包对应33项来源工具，53项Agent capability与6项View操作分开记录；仍有121项来源工具没有交付操作映射。Host接受0/154。
- 旧映射指向已经过时的 candidate-03，现指向新0.1.1包；两项页面操作（模型列表、单轮生成）有明确包路径、版本、收据和View ID。
- 当前独立 Agent 工具版本1.10.0只作为来源版本，原生功能与应用版本没有变化；没有重新编译Rust或发布安装器。
- 候选01为本轮深色提示颜色调整前的已冻结中间版本，保留原字节和独立proof目录；最终使用candidate02。旧0.1.0 candidate04也未修改。
- 打包脚本拒绝用 refresh-verification 覆盖已冻结收据，后续需另建收据修订。
- Studio公开只读脚本验证包、目录与profile均通过，不能替代安装、签名、原生授权和真实模型服务端验收。

## 证据与未完成范围

本目录保存 desktop-proof.json、mobile-proof.json、桌面/窄屏首屏与结果截图，以及包、profile、目录校验结果。原始端到端命令为 python scripts/package_agent_assist_preview.py；包脚本内部依次运行 limits.test.cjs、迁移同步检查、两种视口的 verify_agent_assist_preview_view.py。随后运行 verify_agent_assist_preview_package.py 和 Studio 只读接收校验脚本。

尚未验证真实模型网络、Studio原生确认、权限撤销、签名安装、更新回滚、物理触控及其他浏览器。没有给Studio会话发送消息，没有启用用户禁用的JSON Object。后续继续检查图片与文本资源候选及完整Agent接口缺口。
