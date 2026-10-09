# 实验多字段适配（非交付插件）

当前zi-fields-wasi/0.1.0实现proposal.2四操作：json.path、json.diff.ordered、json.diff.unordered、regex.matches，前缀devtools.compare.。text.diff未接入，仍返回UNSUPPORTED_OPERATION。不接受冻结rc.1，无View、manifest、安装候选、签名包或Host接受。

JSON与正则分别共用完整zi-json-core/zi-regex-core；原生输入范围不减。请求/最终结果48KiB，text/left/right8192 UTF8字节，query/pattern4096；场景1–128字节，显式null commandId，严格拒绝缺失/重复/额外/类型错误字段。原生正则保留2MiB文本、捕获组、Unicode字节位置、100匹配和120字符展示。

当前模块1425085字节，SHA-256 `8aee2ae20f295b0b95c1602be236e7ed17a2e4db4765323b6ca1d649a07887ac`。197条固定native/WASI精确字节与结果Schema通过；原186条期望保留，新增11条空捕获嵌套/命名/flags/死捕获编号/交替/断言/非空回退及100匹配边界。它们不是完整正则语义证明。

独立Wasmtime36.0.2在原10M/五秒/64MiB/2MiB栈预算下196通过、1失败：`a{1000000}`在返回原引擎程序大小拒绝前耗尽fuel。1000个空捕获的输出放大已通过精确语法识别与HIR Empty/Capture/Concat路径优化降至fuel3019147，结果仍为INPUT_TOO_LARGE；其他语法继续原引擎。初始通用HIR路径仍耗尽fuel的v2记录保留，最终平铺空组识别无需大型解析器记账。

另有真实语义失败：独立脚本verify_regex_program_limits.py在`a{400000}`发现native64返回INVALID_INPUT、WASI32却返回没有匹配；200000/700000两侧一致。原因与引擎编译程序内存限额/指针宽度有关，不能仅靠197条绿向量声称全范围可移植。该诊断真实exit1，源码和失败证明保留；冻结前必须解决，不改成接受不同结果。未做当前最终模块冷启动验收，不借旧三JSON模块的成功。

当前核心5组与适配4组回归通过，全features严格Clippy通过；完整43组工作区851通过/0失败/34忽略，最新优化构建/新EXE服务链仍运行。正则工具1.0.4、JSON查询1.0.3、JSON对比1.0.4，根dev105保持，所有相关插件仍not_delivered、Host接受0。

先解决大重复编译budget与跨架构程序限额，再验证最终预算/冷启动、完整恶意输入、View/通用元数据与真实Host安装/授权/更新/撤权/卸载/Pi。CI固定功能矩阵不是提供方交付或Host接受。所有历史失败保留，未放宽预算、删除失败或缩减语法。

历史186条阶段模块1413434B曾预算184通过/2失败；该版优化构建/服务及主CI37940068942均已通过，但不替代本轮源码和模块证明。

## 历史三JSON模块验证

以下证据只适用于加入正则前的模块与源码69ceb19，不代表当前四操作模块接受：

# 实验多字段JSON适配（非交付插件）

zi-fields-wasi/0.1.0 仅实现[多字段提案0.1.1](../../proposals/studio-fields/v0.1.1/README.md)的三项JSON操作：json.path、json.diff.ordered、json.diff.unordered，前缀devtools.compare.。明确协议1.1.0-proposal.2，不接受冻结rc.1；regex.matches和text.diff明确返回UNSUPPORTED_OPERATION。**不是完整五操作提案，不是可安装候选，不是Host接受。** 不新增manifest、签名容器或虚构可用入口。

三操作调用完整同仓zi-json-core，独立程序也复用同一核心。请求/序列化结果各48KiB，查询text/query各8192/4096 UTF-8字节，差异left/right各8192字节；场景1–128字节，显式null commandId、具名字段严格拒绝缺失/重复/额外/错误类型。用户JSON文本中的重复键仍保留原算法后值覆盖。输出错误不回显输入。独立版原1MiB/8MiB范围不变，不能将实验预算冒充原工具全部输入范围已迁移。

最终模块275361字节，SHA-256 `d85d48baae4a350d60363f0d7e772be6d5ef565ee3b019ece46aa0e6ad7b851b`。147条实际native/WASI精确输出字节和结果Schema通过，前141条请求及期望摘要原样保留。覆盖字段结构、Unicode字节、JSONPath语义、Pointer、嵌套数组、请求/结果边界、长共同路径放大及30/100层对象内250项数组。独立Wasmtime36.0.2预算147条通过，最大fuel4865328；147次全新进程冷启动通过，最慢1921.914ms，全部子进程关闭。预算仍10M/五秒/64MiB/2MiB栈，没有扩大。

扩展验证发现：每侧不足8192字节的JSON可通过共同6000字节字段名生成超过48KiB的差异报告，原适配耗尽10M fuel。共享核心新增可选结果预算，只对实际差异累计路径UTF-8字节下界；下界已超限时提前返回类型化错误，不构造剩余报告。最终序列化仍检查完整信封；相等输入和无效JSON保留原行为，独立版仍生成完整100项报告且原1MiB/8MiB范围不变。修复后该用例fuel1914968，未缩减输入范围或放宽预算。原失败证明保留在技术档案。

这些只是147条实验验证，不是完整恶意输入覆盖或生产隔离证据。Node只提供功能验证，独立官方引擎另测预算；Python/C-API冷启动不是实际Studio Worker冷启动。还需要更多复杂组合验证、通用元数据和View、明确Host协议协商及真实安装/权限/更新/撤权/卸载/Pi验证，之后才考虑冻结unsigned候选。两工具映射仍not_delivered。

验证：使用scripts/dev.ps1构建native和现有plugin/WASI目标，运行scripts/verify_fields_wasi.py；固定期望可导出为供独立预算重放的fixtures。CI只记录实际native/WASI功能/Schema及工作区检查，不把CI绿灯当作Host支持。根dev105保持；查询源工具1.0.3、JSON对比1.0.4。


深层无序比较原来在每层对象重新规范化整个数组，30/100层用例实际耗尽10M fuel。修复仅在数组节点规范化，达到10000项边界时保留原有无序相等检查；完整报告及边界行为与原算法一致，原始数组顺序和重复值保持。两用例现fuel2371717/4865328；旧失败证据保留。最终41组工作区846通过/0失败/34忽略，全features严格Clippy通过。新原生优化构建及新EXE服务链仍运行，不能用前轮产物替代。
