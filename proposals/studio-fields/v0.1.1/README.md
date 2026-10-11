# 多字段纯计算提案 0.1.1

状态：提供方提案，未冻结、未被Host支持，无通用运行适配。显式协议版本为1.1.0-proposal.2，profile仍为compute.fields.v1。保留[前版](../v0.1.0/README.md)，原冻结rc.1及所有既有候选不变；不能回退到旧input.text隐藏JSON。

本版共五项操作：regex.matches使用text/pattern，text.diff及json.diff.ordered/json.diff.unordered使用left/right，新增json.path使用text/query。差异模式用不同能力明确选择。JSON查询两字段分别8192/4096 UTF-8字节；其余预算保持前版：总输入16384、请求/结果49152字节，模块2MiB、10M fuel、五秒、64MiB内存/2MiB栈。字节限制独立于schema字符计数，不能只用maxLength。

JSON查询保留$、.字段、双引号字段、非负u64索引及[*]，无递归/过滤器/切片；原生完整范围仍为1MiB文本、4096字节查询、128步、10000匹配、8MiB输出。超过u32但未超u64的索引在native64/WASI32均返回未匹配；溢出u64明确失败。原输入JSON重复键后值覆盖、宽整数、对象通配顺序保持。提案外层/具名字段重复必须拒绝；工具text中的JSON重复键语义与外层校验不能混淆。

当前实验适配已调用同仓完整JSON/regex共享核心，四操作219条native/WASI精确输出与原预算通过，另27条编译/长文本及528条旧算法对照通过；完整正则范围预算仍未证明，最新独立冷启动218通过/1期限失败，旧210/9证据保留，仍非Host执行。text.diff完整核心已提取，但本版适配仍不支持；其原500ms单调计时会触发原库近似行为，不能用fuel替代、关闭std后忽略截止或另造TIMEOUT错误。新增时钟约定及五操作实验另见[0.1.2提案](../v0.1.2/README.md)，不改变本版协议。详见[迁移设计](../../../docs/text-diff-plugin-design.md)。

字段接力显式指定text/query或left/right，不从私有草稿、默认值或其他窗口补齐。预览来源、目标版本/场景、全部输入与修订，确认时重新核对授权/预算/版本/修订；取消不提交，不自动执行，旧结果与其他字段保留。query的4096字节限制需要独立检查。未协商本协议时入口应明确不可用，不静默改用旧字段协议。

verify_fields_proposal.py --proposal-version 0.1.1 实际验证请求/结果schema、身份、严格解析及UTF-8字节正反例，包含精确ASCII/Unicode字段与场景边界、旧协议拒绝和重复query。空query及非JSON text可通过输入形状检查，之后仍需运行时算法校验；结构验证不是算法成功。所有工具映射仍按各项真实状态记录，不因提案新增操作标记已交付。
