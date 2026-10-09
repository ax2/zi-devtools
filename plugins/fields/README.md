# 实验多字段JSON适配（非交付插件）

zi-fields-wasi/0.1.0 仅实现[多字段提案0.1.1](../../proposals/studio-fields/v0.1.1/README.md)的三项JSON操作：json.path、json.diff.ordered、json.diff.unordered，前缀devtools.compare.。明确协议1.1.0-proposal.2，不接受冻结rc.1；regex.matches和text.diff明确返回UNSUPPORTED_OPERATION。**不是完整五操作提案，不是可安装候选，不是Host接受。** 不新增manifest、签名容器或虚构可用入口。

三操作调用完整同仓zi-json-core，独立程序也复用同一核心。请求/序列化结果各48KiB，查询text/query各8192/4096 UTF-8字节，差异left/right各8192字节；场景1–128字节，显式null commandId、具名字段严格拒绝缺失/重复/额外/错误类型。用户JSON文本中的重复键仍保留原算法后值覆盖。输出错误不回显输入。独立版原1MiB/8MiB范围不变，不能将实验预算冒充原工具全部输入范围已迁移。

最终模块275361字节，SHA-256 `d85d48baae4a350d60363f0d7e772be6d5ef565ee3b019ece46aa0e6ad7b851b`。147条实际native/WASI精确输出字节和结果Schema通过，前141条请求及期望摘要原样保留。覆盖字段结构、Unicode字节、JSONPath语义、Pointer、嵌套数组、请求/结果边界、长共同路径放大及30/100层对象内250项数组。独立Wasmtime36.0.2预算147条通过，最大fuel4865328；147次全新进程冷启动通过，最慢1921.914ms，全部子进程关闭。预算仍10M/五秒/64MiB/2MiB栈，没有扩大。

扩展验证发现：每侧不足8192字节的JSON可通过共同6000字节字段名生成超过48KiB的差异报告，原适配耗尽10M fuel。共享核心新增可选结果预算，只对实际差异累计路径UTF-8字节下界；下界已超限时提前返回类型化错误，不构造剩余报告。最终序列化仍检查完整信封；相等输入和无效JSON保留原行为，独立版仍生成完整100项报告且原1MiB/8MiB范围不变。修复后该用例fuel1914968，未缩减输入范围或放宽预算。原失败证明保留在技术档案。

这些只是147条实验验证，不是完整恶意输入覆盖或生产隔离证据。Node只提供功能验证，独立官方引擎另测预算；Python/C-API冷启动不是实际Studio Worker冷启动。还需要更多复杂组合验证、通用元数据和View、明确Host协议协商及真实安装/权限/更新/撤权/卸载/Pi验证，之后才考虑冻结unsigned候选。两工具映射仍not_delivered。

验证：使用scripts/dev.ps1构建native和现有plugin/WASI目标，运行scripts/verify_fields_wasi.py；固定期望可导出为供独立预算重放的fixtures。CI只记录实际native/WASI功能/Schema及工作区检查，不把CI绿灯当作Host支持。根dev105保持；查询源工具1.0.3、JSON对比1.0.4。


深层无序比较原来在每层对象重新规范化整个数组，30/100层用例实际耗尽10M fuel。修复仅在数组节点规范化，达到10000项边界时保留原有无序相等检查；完整报告及边界行为与原算法一致，原始数组顺序和重复值保持。两用例现fuel2371717/4865328；旧失败证据保留。最终41组工作区846通过/0失败/34忽略，全features严格Clippy通过。新原生优化构建及新EXE服务链仍运行，不能用前轮产物替代。
