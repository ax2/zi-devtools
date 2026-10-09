# 实验多字段适配（非交付插件）

当前 zi-fields-wasi/0.1.0 实现[多字段提案0.1.1](../../proposals/studio-fields/v0.1.1/README.md)的四项完整操作：json.path、json.diff.ordered、json.diff.unordered、regex.matches，前缀devtools.compare.。明确协议1.1.0-proposal.2；text.diff仍返回UNSUPPORTED_OPERATION。不接受冻结rc.1，不是完整五操作提案，没有View、manifest、签名容器或安装入口，未被Host接受。

JSON共用zi-json-core，正则共用zi-regex-core，独立原生版复用相同完整算法。请求/最终结果各48KiB，text/left/right各8192 UTF8字节，query/pattern各4096字节；场景1–128字节，显式null commandId，具名字段拒绝缺失、重复、未知与错误类型。正则保留原捕获组、Unicode字节位置、100匹配与120字符展示省略规则，独立版仍支持2MiB输入。原生工具的完整范围不等于插件输入预算。

当前模块1,413,434字节，SHA-256 `a3f0e82a243b5b909e9faac7ff7fc2bba7831a795c91f6c5519d92ec72589a1f`。186条实际native/WASI精确结果字节与结果Schema通过；新增正则语法、捕获、零宽、Unicode、无效表达式、字段结构及精确字节边界。原unsupported-regex用例因操作已实现，明确改为unsupported-text-diff；其余JSON用例保留。

独立Wasmtime36.0.2在10M fuel/五秒/64MiB/2MiB栈预算下184通过、2失败：`a{1000000}`编译至原生引擎大小拒绝前已耗尽fuel；1000个空捕获组的输出放大也耗尽fuel。功能期望分别为INVALID_INPUT和INPUT_TOO_LARGE，实际预算执行却trap；不能把功能正确或CI绿色视为预算通过。原失败证明保留，不扩大预算、不删除用例、不改为ASCII子集。本最终模块尚未进行冷启动完整验收；不会套用旧三JSON模块的冷启动成功。

正则核心4组回归和适配4组回归通过，全features严格Clippy通过；完整43组工作区850项通过、0失败、34忽略。原生优化构建/新EXE服务链仍在运行。根dev105不变；正则1.0.3、查询1.0.3、JSON对比1.0.4。两类工具均not_delivered，Host接受0。

还需处理编译预算及大量捕获组问题、扩展恶意组合、最终预算和冷启动、通用元数据/View、Host协议协商与实际安装/权限/更新/撤权/卸载/Pi。验证脚本verify_fields_wasi.py只做实际功能/Schema，Node不是fuel或生产隔离证据。

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
