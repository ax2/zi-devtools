# 实验多字段JSON适配（非交付插件）

zi-fields-wasi/0.1.0 仅实现[多字段提案0.1.1](../../proposals/studio-fields/v0.1.1/README.md)的三项JSON操作：json.path、json.diff.ordered、json.diff.unordered，前缀devtools.compare.。明确协议1.1.0-proposal.2，不接受冻结rc.1；regex.matches和text.diff明确返回UNSUPPORTED_OPERATION。**不是完整五操作提案，不是可安装候选，不是Host接受。** 不新增manifest、签名容器或虚构可用入口。

三操作调用完整同仓zi-json-core，独立程序也复用同一核心。请求/序列化结果各48KiB，查询text/query各8192/4096 UTF-8字节，差异left/right各8192字节；场景1–128字节，显式null commandId、具名字段严格拒绝缺失/重复/额外/错误类型。用户JSON文本中的重复键仍保留原算法后值覆盖。输出错误不回显输入。独立版原1MiB/8MiB范围不变，不能将实验预算冒充原工具全部输入范围已迁移。

模块274389字节，SHA-256 `8a405ce0a2d1814bd3a61cf0fdf3c95b4e4c06675b8046563c22b6f2d23674b5`。19条实际native/WASI精确输出字节和结果Schema通过，包含三操作、u32以上/u64最大索引、宽整数、重复值、身份/旧协议/UTF8拒绝、精确字段/请求/结果字节边界及128/129路径步。独立Wasmtime36.0.2预算19条通过：最大fuel7086467、执行2ms、内存2752512字节；19次全新进程冷启动通过，最慢657.382ms，全部子进程关闭。预算仍10M/五秒/64MiB/2MiB栈，没有扩大。

这些只是19条实验验证，不是完整恶意输入覆盖或生产隔离证据。Node只提供功能验证，独立官方引擎另测预算；Python/C-API冷启动不是实际Studio Worker冷启动。还需要扩展完整语义/恶意向量、通用元数据和View、明确Host协议协商及真实安装/权限/更新/撤权/卸载/Pi验证，之后才考虑冻结unsigned候选。两工具映射仍not_delivered。

验证：使用scripts/dev.ps1构建native和现有plugin/WASI目标，运行scripts/verify_fields_wasi.py；固定期望可导出为供独立预算重放的fixtures。CI只记录实际native/WASI功能/Schema及工作区检查，不把CI绿灯当作Host支持。根dev105和源工具1.0.3保持。
