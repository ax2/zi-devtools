# Text 插件 JSON fuel 与冷启动复验

日期：2026-10-17\
范围：为已重建的 Text 0.4.0 WASI 模块补做官方 Wasmtime 预算，修复实际耗尽的 JSON 路径并生成 0.4.1 unsigned 候选。应用版本保持 `0.82.0-dev.105`。

## 发现与决策

首轮官方 Wasmtime 36.0.2 C-API 测量发现 41/43 通过，但大型嵌套 JSON 格式化与必然输出扩张超限两项耗尽 10M fuel。问题来自旧严格解析器先用 RawValue 验证，再递归重新解析每个子容器，导致重复扫描。未把失败写成通过，也未增大燃料上限。

将 `strict_json` 改为一个 `DeserializeSeed`/Visitor 单次构建 `serde_json::Value`。容器遍历时直接检测重复键；保持64层限制；整数与浮点仍拒绝超出安全范围、非有限值及解析后已被舍入的大整数。桌面 JSON 默认核心不变，插件兼容模式继续复用原规则。

## 验证

- `cargo test --offline --locked -p zi-text-core`：20通过、0失败；`cargo clippy --offline --locked -p zi-text-wasi -- -D warnings` 通过。
- 重新构建 `wasm32-wasip1/release` 和本机 native 适配器；43条 native/WASI 字节一致及 View 两主题七操作检查通过。
- 官方 Wasmtime 36.0.2 独立 fuel 检查43/43通过，限额仍为10M fuel、5秒、64MiB。最大使用4,099,756，来自拒绝输出扩张用例；大型合法格式化用例使用2,185,948。
- 43个新进程冷启动全部通过，全部关闭；最慢681.396ms。预算及冷启动均绑定新模块SHA-256 `7cfcde63aaaa68353067ca1f7a99c8c6e4a6926166f541a6de03c7c814803767`。
- 当前候选 `release/text-plugin-0.4.1-candidate-02`：112文件、1,032,175字节，逐文件清单/目录摘要重算相符；含 raw requests、expected bytes、fuel/cold proofs、engine provenance、重放脚本、目录快照和迁移说明。签名字段为空值并明确 unsigned，没有 `.zicode-plugin`。同版 `candidate-01` 的较早收据与目录保留原样。
- 工具目录映射同步到 JSON `1.2.1`、包 `0.4.1`。全154项迁移映射仍为 `acceptedCapabilities=0`。

## 未完成与风险

官方 Wasmtime C-API 只证明此模块在指定引擎和预算下的功能/资源上限，不替代 Studio Worker、Host pipe drain、权限、Pi Broker、安装/更新/撤权/卸载、真实界面截图或其他操作系统验收。候选仍未签名。Inspect 和 Trace 的目录元数据已进入源清单，它们的新候选包尚未生成。
