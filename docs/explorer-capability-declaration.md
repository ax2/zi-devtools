# 桌面菜单能力声明0.24.0候选

目录和更多接收工具扩展前，需要区分每个目标程序支持的动作。能力声明现已用于选择性安装目录菜单：只有明确声明 `choose-folders` 的当前程序安装目录项及目录背景入口，旧文件能力快照维持原有两个文件菜单根。

## 二进制格式

仅桌面main含`.zicap`不可写/不可执行PE数据节，MCP不声明桌面UI能力。构建脚本从同一`Target::ALL`生成，不另维护动作清单，不依靠长期不变的dev版本判断。

节内容为8字节ASCII `ZICAP001`、4字节小端声明长度、UTF-8声明。声明上限4096字节，固定头`ZiDevTools/ShellCapabilities/1`和`choose-files`各占一行；可选的`choose-folders`声明独立文件夹接收能力，其后每行一个已知文件动作ID，必须LF结尾。允许仅选择工具而无具名接收器；未知协议、动作、重复项、空行、非UTF-8或非规范换行拒绝。动作顺序保留。旧声明缺少`choose-folders`时只登记既有文件菜单，不推断目录支持。

PE读取仅使用已有File的Read/Seek，不LoadLibrary、不启动EXE。大小上限256MiB，PE头偏移不超过1MiB，最多96节，按需读取且单次分配不超过4096字节。检查节唯一性、原始字节范围、重叠、只读数据权限、包络与声明长度。没有声明节返回None，表示旧程序能力未知；声明存在但损坏返回错误，不能退化为旧版或全支持。

## 实际验证与未完成事项

新增测试覆盖当前和子集动作集合、未知/重复/过大/坏编码/截断，以及合成PE偏移、权限、重复/重叠节和每个截断位置。本地四个新增测试和严格静态检查通过。

独立`zi-shell-core`例程`capability_inspect`在同一只读共享锁文件上解析并流式计算SHA256。实际debug桌面具有11动作声明；debug MCP和上轮优化桌面无声明。输出明确`executableStarted=false`、`ledgerUpdated=false`、`publisherAuthenticated=false`。本轮只读检查不代表目标程序运行验收、菜单能力过滤或发布者认证；摘要与声明必须在后续维护事务中绑定同一文件锁和预览票据。

本机原有注册表事务测试出现系统错误6801；独立只读打开既有HKCU Software的事务调用同样返回6801，未修改注册表。新增纯解析测试通过，完整本机回归不能宣称通过，托管Windows完整工作区/实际安装声明检查另行记录。未自动重置系统事务日志或重启。

后续按[多版本共存设计](explorer-capability-compatibility.md)实现账本/日志协议迁移、旧程序未知能力策略、预览展示和失败回滚，再启用新目录动作。应用版本保持0.82.0-dev.105。

## f4fdb6c最终验证

[CI38216992126](https://github.com/ax2/zi-devtools/actions/runs/38216992126)五路全部成功，production-upgrade完整工作区944 passed / 0 failed / 35 ignored（52组含文档测试）。实际安装桌面文件声明与11动作一致，MCP无桌面声明；公开旧版升级、键盘操作、12项Shell解析、完整UI卸载和用户文件保留继续通过。此结果不消除本机6801问题，托管原生事务通过与本机回归失败分别记录。

单次本地优化桌面构建4m25s成功，实际只读检查保留11动作声明（32368128字节，SHA256 `de3e090e6eb33854e6fde1d254c9a7c194dff9b66bb0dffde68165e1cb664c8e`）。该优化EXE一次性服务健康检查、正常停止、退出0及端口释放通过；未发布、未复制EXE、未为证据文案重复优化构建。最终证据在技术档案`iterations/explorer-capability-declaration-2026-10-12`。

## 文件夹接收能力（2026-10-13，待托管 Explorer 复验）

新增可选`choose-folders`能力位；MSI的有界能力属性与EXE内声明都保留旧格式的默认语义。只有明确带该能力的活动程序才会登记文件夹菜单。Explorer在选中文件夹和文件夹空白处分别注册统一的`Zi DevTools`入口，并各自指向独立子命令别名，避免把`%1`与`%V`混用。完整六个Classes键由同一所有权审计和事务回滚覆盖，旧能力记录仍只有文件菜单。

独立`--open-folder`接收器只接受一个目录，路由到目录空间分析页并立即执行现有有界只读扫描；不走文件导入页、不读取文件内容、不删除数据。当前纯Rust工作区测试已经覆盖能力旧版兼容、六根注册表事务、Windows命令行参数往返与实际扫描夹具。托管Windows Explorer中对选中目录和空白处显示/点击级联、候选MSI升级/卸载恢复还未复验；本机没有安装候选MSI或写入注册表。应用版本保持`0.82.0-dev.105`，工具版本在工具清单独立记录。

## 0.25.0声明驱动计划候选

规划层新增`explorer::plan_declared`，只为传入声明中的接收动作生成菜单。保留选择工具入口；子集不会自动带入维护程序新增的动作。现有`plan`包装仍使用当前全量声明，输出与之前逐字段等价，生产安装/恢复尚未改用逐安装能力快照。

只读检查例程支持`capability_inspect.exe executable --plan config-path`。从同一受写删锁保护的文件读取声明和摘要后生成预览；缺失声明拒绝生成计划。config-path只用于计划，不读取或创建配置，不登记菜单。实际优化桌面的11动作、合成PE的两动作子集和仅选择工具都通过检查，MCP缺失声明拒绝；合成PE不是可运行的旧版本安装证据。

原生安装探针使用同一规划器生成待核对标签，继续检查实际Shell菜单；对应托管结果单独记录。这不表示安装生命周期已依据声明过滤，目录动作仍待账本/恢复协议迁移后开发。

99ed9e5的[CI38355789529](https://github.com/ax2/zi-devtools/actions/runs/38355789529)五路全部通过；完整工作区945 passed / 0 failed / 35 ignored（52组）。实际安装菜单proof确认`declaredPlanMatchesShell=true`，同时桌面声明匹配、MCP无桌面声明、四键盘场景、完整UI卸载及用户文件保留继续通过。子集语义通过合成PE只读计划和独立规划测试验证，尚不代表两个实际不同能力发行版的接管/回滚已验收。

## 文件夹能力的实际注册与 Explorer 显示（08fb27a）

[CI38404005828](https://github.com/ax2/zi-devtools/actions/runs/38404005828)七路全通过。MSI事务矩阵确认旧/合成文件-only声明仅有两个文件菜单根，当前真实桌面载荷带 `choose-folders` 时使用六个根；不支持的根必须为空。Explorer 可见作业在隔离目录中实际显示文件夹项和背景菜单，两个级联都包含“分析目录空间…”。已查看归档的六张截图；未执行接收动作或扫描。生产升级作业另外通过公开 v0.81.0 升级、安装选项/修复/卸载及用户数据保留。

这验证了能力位驱动菜单注册、文件夹选择和空白处显示，不代表接收器启动或扫描 UI 的端到端体验，也不覆盖 Windows 11 现代菜单、多选及正式发布验收。proof与脱敏日志、截图归档于 `iterations/explorer-folder-contexts-2026-10-13/ci-08fb27a-visible-menu/` 和 `ci-08fb27a-production-upgrade/`。

## 0.25.0 文件夹菜单托管复验补充（20d3046）

[Explorer/MSI 工作流 38415199930](https://github.com/ax2/zi-devtools/actions/runs/38415199930) 和同提交常规 [CI 38415199761](https://github.com/ax2/zi-devtools/actions/runs/38415199761) 通过。当前带 choose-folders 的实际桌面载荷在 Windows Server 2022 的 Explorer 中显示选中目录与目录背景入口及“分析目录空间…”级联项；六张 Explorer 截图已检查。文件夹命令参数展开及目标路径匹配通过。

实际接收器窗口未通过托管 runner 验收：启动尝试遇到 egui_glow requires opengl 2.0+，且 nestedMenuActivationVerified=false。因此本轮只确认菜单显示与参数构造，不确认子菜单真实点击、目录扫描或结果页面。应在具备 OpenGL 2.0 的交互式 Windows 桌面继续验证；Windows 11 现代菜单和多选也仍待验证。

## 本机窗口启动补充（2026-10-13）

当前桌面构建在本机通过 --no-restore 和 --open-folder 启动后实际显示 Zi DevTools — disk-inspector 窗口且进程响应正常。窗口创建已在本机验证；没有屏幕截图或扫描完成结果，不能据此宣称结果页验收。详细摘要见技术档案 ci-20d3046-open-gl-limit/local-runtime-proof.json。

扫描逻辑的完成态由现有精确回归测试验证（1 passed）：等待后台任务结束并断言结果根目录及总字节数。它与本机窗口启动记录共同覆盖接收器入口和扫描状态，但没有截图或人工检查完成后的结果页。

## 本机真实应用路由与完成结果页（2026-10-13）

ui-preview 捕获场景通过真实参数解析和 DevToolsApp.open_startup_arguments 路由，扫描只含两份合成文件的隔离目录；断言 Complete、2 files、13 bytes，实际渲染结果页并保存截图。图像经人工检查可读。截图发现长路径/项目名被截断，工具现为路径及结果列表增加悬停全文提示。截图未覆盖提示层，实际Explorer菜单点击与Windows 11菜单仍待验证。proof和截图见技术档案 ci-20d3046-open-gl-limit/local-ui-scan-result/。
