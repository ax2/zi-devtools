# Spring Actuator 交付映射与冻结包复核（2026-10-18）

## 问题与改动

Spring Actuator 0.1.1 candidate-02 已有冻结目录和外部收据，但迁移表的操作缺少具体包路径、包版本和收据引用，导致交付总表漏掉它；工具级 candidateDirectory 还指向可变源码目录。现将 health、info、metrics、summary 四项页面操作绑定到原冻结包和具体 View，源码目录单列为 sourceDirectory。

manifest 没有声明 capabilities，因此这四项不能当作 Agent 能力。迁移表使用 candidateViewOperations，包映射使用 viewOperations 与 executionSurface=view；打包器逐包核对能力集合必须与冻结 manifest 完全一致，页面操作必须关联实际声明的 View，并拒绝重复映射。独立工具功能、插件页面、权限与模块字节没有修改，没有修改或发送消息到 Studio 会话。

旧 Actuator 复核脚本把“历史来源哈希必须等于当前工作树”当作冻结包有效性条件，后续任何目录迭代都会误报。现按外部收据核实文件集合、文件大小、摘要、索引和身份；当前来源一致性单独报告 true/false/null，严格来源对比可通过 --require-current-source 显式启用。校验拒绝重复 JSON 键、重复或越界收据路径、链接/junction、包内收据及不实的签名/完整范围声明。不再因为目录旁存在其他正式插件包而拒绝本候选。

## 验证

- scripts/tests 全量 56 项通过；新增回归覆盖历史来源变化、缺失当前来源、篡改、重复/越界路径、索引/身份变化、包内收据与页面操作误算为 Agent 能力。临时目录验证确认复核不修改候选文件时间。
- 冻结 View 的 Node DOM harness：4 项成功、拒绝/profile、6 类错误、9 个输入防护、3 个 Origin 规范化场景、取消等待 commit 最终态通过。这是模拟 Host API，不是真实 Studio 或渲染验收。
- 冻结合同验证：32 个 JSON 文件、22 个 Draft 2020-12 schema、6 项本地引用、21 项 schema 向量、10 项游标、7 项事件终态、8 项确认/过期/重放与副作用、10 项资源向量通过。
- 首次合同复测使用已有 jsonschema 4.25.1 基础环境，缺少 format extra，非法日历日期负向向量失败。随后按包内 requirements 安装 jsonschema[format]==4.26.0 到用户缓存 actuator-verification-python，全部通过。没有放宽日期规则或修改冻结夹具。
- Studio 现有只读 manifest preflight 无 issue；冻结包与最终目录 revision verifier 均 verified=true。七份包收据索引均 matched。
- 工具目录同步、迁移表同步及 Git 差异检查通过。本轮不修改 Rust，实现与版本不变，因此没有重复 Rust 编译/生成构建缓存；未发布、未安装。

## 可追踪交付

- 原包：release/plugin-candidates/spring-actuator-0.1.1-candidate-02，42 文件、130016 字节，原始文件与收据保留。
- 新外部规范化收据：spring-actuator-0.1.1-candidate-02-verification-revision-01.json，仅将文件索引显式记录为 path/size/sha256；索引 89dd2facddf4ead4d234cf902dcdbd1cbf08fde3ffdd9ec3be42b6b68a55d330，收据 SHA-256 b662d2f028ba70f0327fe19ca17753bc18ea5f54a7d4651c7f762399b0786036。
- 最终目录：release/plugin-candidates/devtools-catalog-2026-10-18-5f0572f4e15f565e，12 文件、2371594 字节；外部收据为同名 -verification.json。
- 最终目录索引：91de0e93796d8425cb34e89de3e78c3e6906d5a3b33c39cde03f7912ef7c3bf9。
- 来源工具目录 SHA-256 b179fc23896f4fdc8ba220c547a86994d613fb407d6aa920a6d353fe2c8d5e20 保持不变；迁移表 SHA-256 c97b0a8e55b52597ed22113f3ccc50d587165c31778c5977e1c9afcedb18f209。
- 中间目录 d0623b8ce0a77b47 使用原 Actuator 收据，保留为历史；最终 5f0572f4e15f565e 使用规范化收据，不覆盖旧目录。

## 当前范围与后续

154 个来源工具中有 32 项与 7 个候选包建立映射；实际为 53 项 capability 加 4 项页面操作，不能说 57 项 Agent 能力或 32 个完整工具已接入。仍有 122 项来源工具未有交付操作映射。

Spring Actuator 仍是固定 health/info/metrics 和摘要子集，Bearer 认证、context path、单项指标选择、后台任务、完整结果接力与报告导出未完成。Host 实际网络策略、权限/取消、Pi→Host→View、跨平台和签名安装尚未验收。Host 接受仍为 0/154，JSON Object 保持用户禁用状态。下一步继续核对其他已存在的资源型和 Agent 页面候选，逐项区分可交付范围、缺少 View 与 Host 协议缺口。
