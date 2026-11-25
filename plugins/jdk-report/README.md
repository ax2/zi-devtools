# JDK 环境报告查看

在 ZiDevTools 独立版「JDK / 构建环境诊断」保存 JSON 报告。回到 Studio 工具中心搜索「JDK 环境报告查看」，选择并确认这份本机文件。查看报告中记录的版本、供应商、架构、`javac` 和构建入口状态。若采集时选择了另一个 JDK，会同时显示两个环境和版本字符串是否相同。`JAVA_HOME` 会按文件内的记录显示一致、不一致或未判定。

所有状态均来自历史快照。包验证原生报告的来源标识、版本和可移植指纹结构，按独立版规则核对版本字符串。可预览并确认保存只包含这些识别字段的 `.txt`。结果省略可执行文件路径、`JAVA_HOME` 路径、入口路径、备注和原指纹。版本、供应商和架构字段有独立长度限制，并拒绝路径分隔符和控制字符。

每次一份最多 32 KiB 的 UTF-8 JSON，仅申请 `resource.read`。系统上的 Java 检查、PATH 查询和路径一致性检测继续由独立版原入口处理。

## 构建与验证

运行 `cargo build --offline --locked -p zi-jdk-report-wasi`，再用 `--profile plugin --target wasm32-wasip1` 编译同一模块。`scripts/verify_jdk_report_plugin.py` 对照真实 Windows Worker 和原生适配器的报告字节与限制。脚本 `scripts/package_jdk_report_plugin.py` 将通过的模块和包外收据冻结。
