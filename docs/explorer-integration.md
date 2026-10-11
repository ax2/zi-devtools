# 资源管理器统一入口：实现约束与验收

更新：2026-10-11。状态：文件接收1.2.0、经典菜单维护与Shell调用修正0.5.0。设置页可预览并确认手动登记、修复、切换、迁移和移除；`explorer-menu`仍为开发中，实际Explorer窗口显示/导入、MSI自动维护与完整多选尚未验收。

## 已有基础及缺口

`src/main.rs`的`--open-files`接收1–64条末尾字面路径，不与工具直达/配置导入/服务夹具混用，不自动恢复服务。`DevToolsApp::open_startup_files`与拖放共用确认入口；只做路径排队与扩展名推荐，用户选择工具后主动导入。CSV/TSV/JSON表进入新实例，其他现有文本工具明确替换草稿，多个文件可交给文件校验。文本2 MiB/UTF-8及64文件上限保留。

现有快捷方式允许各工具独立窗口和任务栏身份，不能为了Shell接入强制所有工具变成单实例。`packaging/windows.wxs`目前是perUser MSI，只维护程序、许可证、开始菜单及Installed标记；没有右键安装选项或菜单组件。不得把现有MSI升级夹具通过当成菜单生命周期通过。

## 入口和命令共用

“Zi DevTools”作为一个顶层级联入口；文件、目录和目录背景分别列适用动作。动作定义携带稳定ID、工具ID/版本、选择类型、扩展名集合、数量限制、草稿接收器及写入能力。工具库、快捷键与Shell读取同一动作定义；未接入接收器的工具不显示可执行动作。文件打开、菜单展开均不做内容识别、递归扫描、哈希或图片解码。

第一组实际接收器是文件导入/文件校验，随后是图片转换/压缩、编码检查、目录分析、SQLite与文档入库；每个接收器分别证明预览、取消、错误反馈和写入确认。目录背景需要显式目录草稿接收器，不能把目录塞进目前只接受普通文件的入口后宣称支持。多选中有不支持项时给出完整清单和理由，不能静默丢弃。

经典菜单优先实现与验证；Windows11现代菜单另做轻量IExplorerCommand/身份部署，不在Shell进程加载egui、模型或工具算法。只验证经典菜单时明确其出现位置，不宣称现代菜单已完成。

## 多选与传输

注册表参数模板中的`%1`只作单选原型；不能假定MultiSelectModel=Player或`%*`自动把完整选区变成argv数组。完整多选必须通过能取得Shell数据对象/IShellItemArray的薄接收器，再将结构化路径列表交给程序。Microsoft说明IDropTarget支持数据对象且避免命令行缓冲限制，选择模型还影响项目数和窗口行为：[数据对象接收](https://learn.microsoft.com/en-us/windows/win32/shell/context-menu-handlers)、[选择模型](https://learn.microsoft.com/en-us/windows/win32/shell/how-to-employ-the-verb-selection-model)。实际本项目接收器仍待实现，不能只登记Player就标多选验收。

传输契约需要独立版本、请求ID、明确动作及路径数组；不允许传入可执行shell文本或任意程序。保持路径原值，按接收时文件状态复核；不把路径拼成PowerShell/cmd命令。帧大小/路径数有界，超量返回可见错误，不截断成功。

已有主窗口通过同一用户、同一配置身份的本机会话通道接收Shell请求；正常`--tool`仍可独立窗口。通道必须有用户/会话访问控制，拒绝远程连接，收发超时明确，收到完整请求才确认接收。没有服务时才创建主窗口；确认响应丢失用请求ID去重，不能悄悄把相同请求再排两次。路径不写入常规日志。

已有导入/编辑忙碌时新请求进入有界待处理队列，不替换草稿、不抢当前录屏/对话焦点；用户可查看、切换和丢弃。队列满返回“未接收”，发送侧显示恢复方式。退出清空会话队列；持久恢复需单独主动开启，不能借菜单默认保存用户文件路径。

## 安装、升级和卸载

纯Rust生成注册计划，执行层负责读取、预览、应用和复核。默认当前用户作用域；每项记录产品所有权、安装身份、程序规范位置、命令及菜单协议版本。已有同名非本产品键须拒绝接管。不能整段递归删除未核对所有权的Classes子树。

经典级联可以使用HKCU Software/Classes下的本产品动作定义并复用，避免要求全局CommandStore写权限。Microsoft明确说明ExtendedSubCommandsKey可在当前用户Classes下登记并跨关联复用：[级联菜单注册](https://learn.microsoft.com/en-us/windows/win32/shell/how-to-create-cascading-menus-with-the-extendedsubcommandskey-registry-entry)。具体值、键布局和Explorer可见性仍需实际验证。

便携EXE提供状态/预览/安装/修复/移除，写入前由用户主动确认；移动目录后显示失效并允许重建。MSI提供明确安装选项，由稳定组件维护自身注册，并验证升级/修复/卸载/回滚；不能用安装时随意执行主程序代替Windows Installer事务。

便携/MSI不互相覆盖同一所有权记录。共享一个可见入口需要明确的活动安装选择及可恢复更新：一方卸载只释放自己，仍有另一份有效安装时入口重新指向它；没有有效安装才移除本产品入口。所有权方案先通过两份安装、升级失败和卸载次序测试，才接入真实注册写入。

## 完成门槛

| 范围 | 必须证明的行为 |
| --- | --- |
| 文件接收 | 中文、空格、字面参数名、长路径、消失文件、二进制、64/65边界，确认前无内容读取/工具执行 |
| 多选 | 真实Explorer中1/2/64项准确进入同一请求；混合类型与超限可见，不丢项、不生成大量窗口 |
| 窗口复用 | 主窗口可见/托盘隐藏/忙碌/无实例/响应丢失/退出竞争；草稿保留、请求去重、选择正确配置 |
| 作用域与权限 | 当前用户注册，其他用户/会话不能转发；非产品注册值保留；不需管理员权限 |
| EXE/MSI | 首装、重复安装、移动后修复、升级、修复、失败回滚、共存及两种卸载次序，无悬空或误删入口 |
| 系统体验 | Windows10经典、Windows11更多选项与现代菜单分别记录；菜单展开不加载工具、不阻塞Explorer |
| 工具操作 | 每个已登记动作均有实际UI接收器；读取/修改/覆盖分别符合用户授权，结果与部分失败可追踪 |

这些门槛都通过后才将explorer-menu改为已实现。现有文件接收基础、CLI测试、受控截图或普通MSI事务测试均不能单独替代此验收。

## 开发版提供方基础0.1.0

11种接收动作已集中到`commands/file_actions.rs`，导入页/读取校验/外部`--file-action`和注册预览共用；`intake::Target`保持兼容导出。设置页只生成当前用户注册计划和JSON复制，顶层Zi DevTools、选择工具及11个精确接收动作；每个候选Shell动作显式Single，原文件校验64条CLI能力不冒充Shell多选。无注册查询或写入、无安装/修复/卸载/回滚。Windows CommandLineToArgvW对生成命令逐条拆分后交给真实CLI解析函数，验证中文/空格/符号路径和目标动作，不等同实际Shell展开/调用验收。

## 当前用户状态检查0.2.0

设置页可主动后台读取固定两处候选入口；不存在、一致、部分/内容变化、所有权或未知项冲突、其他协议分别显示。未知内容保留，不自动接管或修复。报告只有计数及状态，不保存、复制或日志记录现有注册原值。检查绑定候选计划指纹，程序或已加载配置改变后旧结果标过期。

仅KEY_READ逐段以REG_OPTION_OPEN_LINK打开，发现REG_LINK立即拒绝；不跟随注册链接。每次快照限制128键、1024值、递归深度8、每值128 KiB、数据合计1 MiB，错误/超限不截断为成功。连续两次快照不一致则拒绝结论；这不是事务快照，安装前仍需互斥及再次复核，不能把一致报告当成授权写入。

Windows读取方式依据[RegOpenKeyExW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regopenkeyexw)与[RegEnumValueW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regenumvaluew)。当前原生测试只在独立Tests/ExplorerAudit-UUID命名空间创建合成数据并验证读取前后一致、超限失败和完整移除；不创建真实Shell菜单。真实菜单显示/调用、系统安装身份、EXE/MSI事务和完整多选仍待实现。

实际Windows将HKCU/Software/Classes映射到当前用户的独立Classes hive。检查器从进程令牌确认用户，直接只读打开HKEY_USERS下该用户的SID_Classes，绕过系统别名而不跟随任意注册链接；其下产品路径仍逐段拒绝REG_LINK，不读混合机器数据的HKCR，也不记录SID。另有实际用户Classes锚点只读测试。映射依据：[Microsoft registry shared keys](https://learn.microsoft.com/en-us/windows/win32/winprog64/shared-registry-keys)。

## 安装账本事务基础0.3.0

`explorer/lifecycle.rs`提供独立于UI和安装包的安装记录及迁移规划。便携安装使用持久UUID，MSI使用固定UpgradeCode家族身份；每次更新生成新代次UUID，即使应用版本未变。更新/移除必须匹配当前代次，提交必须匹配完整旧账本。修订号递增，最后一份移除后仍保留空账本修订号，防止旧预览重放。活动安装从调用方本次核验过的完整记录中选择，主动切换到未核验目标直接拒绝；移除活动安装时优先选剩余最新可用记录，没有可用记录则无活动项。模型不自己确认文件存在、真实性或菜单可调用；接入层必须核验当前文件，不能用历史可用性缓存充当证据。

Windows存储API固定在HKCU/Software/ZiCode/ZiDevTools/ExplorerInstallations，单个State值包含严格协议/所有权JSON，最多32项，UTF-16注册值最多256 KiB。逐段拒绝注册链接，发现未知值/子键、外部所有权、协议、编码或资源超限即拒绝；不清理未知内容。当前使用本机KTM注册事务，在同一事务内读取完整旧状态、写入并复核，再提交；失败回滚，不降级成部分写入。提交未确认时要求重新检查。依据[RegCreateKeyTransactedW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-regcreatekeytransactedw)，事务提交/回滚后须重新打开事务句柄，当前每次操作使用独立事务。

真实原生测试只在Tests/ExplorerLedger-UUID命名空间写入合成安装记录，覆盖两种安装和卸载顺序、首次安装及更新回滚、陈旧预览、并发单赢家和未知值保留，并核实移除测试目录。未连接真实账本的UI自动调用，不写生产账本或Shell菜单。账本事务不是Windows Installer事务，也尚未与共享菜单原子提交；MSI安装选项、文件核验、身份持久化、用户确认及菜单共存/升级/卸载仍按前述门槛待完成。

## 确认式联合维护0.4.0

设置页后台预览本程序的经典单文件菜单，列出当前和目标程序、配置位置及变更数量；勾选同意后才能应用。可刷新安装记录、切换已登记程序、将便携身份迁移到本程序位置或移除登记。移除不删除程序、配置或用户文件；剩余安装重新核验后接管，没有可用项才移除共享入口。MSI记录的更新和移除仍须安装器接入，界面不冒充Windows Installer维护。

活动程序记录包含SHA-256。预览核验绝对规范文件位置、祖先重解析点、基本MZ头及有界文件内容摘要；配置只核验位置和文件存在，不读取内容。应用前重新核验活动程序，持有拒绝写入/删除的读取句柄直到提交。这是文件一致性检查，不是发布者数字签名验证或完整PE验证。

账本与两处固定Classes菜单树在同一KTM事务中重新读取、对照预览、写入和复核，再共同提交。菜单协议2记录活动安装身份、代次及文件摘要；只修复确认属于当前记录的缺失项，所有权标记缺失、未知内容、注册链接或预览后变化均拒绝覆盖。旧的纯账本提交API拒绝带菜单摘要的记录，避免绕过联合提交。成功后通知Shell刷新；状态检查可区分入口属于另一份已登记程序/配置。应用后清除旧检查结果及等待中的旧回复。

隔离Tests/ExplorerJoint-UUID夹具覆盖两个注册hive的共同提交与回滚、两种登记/移除顺序、更新回滚、陈旧预览、未知值保留和缺失命令修复；不会登记真实文件右键入口。受控截图的确认按钮禁用，后端也拒绝应用；截图前后核对三处产品注册根未变化。上述证据不替代真实Explorer调用及MSI安装、升级、卸载和失败回滚验收。

## 原生Shell调用修正0.5.0

真实Shell调用发现规范文件系统路径的`\\?\`前缀用于可执行命令时返回拒绝访问。现在安装记录仍保存规范路径用于身份/摘要核验，只有菜单命令和图标模板转换为普通驱动器或UNC路径；无法转换的扩展设备路径在预览阶段拒绝。已有登记可通过“预览安装 / 修复此程序”确认修复命令。UNC转换有单元覆盖，未验证真实网络共享或超长路径的Shell执行。

`explorer_shell_probe`通过实际IShellItem/IContextMenu构建级联、发送子菜单初始化消息，并逐项InvokeCommand启动独立接收进程；复用产品Plan生成器。普通与规范路径输入各12项调用均核对完整argv，覆盖中文、空格、符号、字面`%1`和类似参数的文件名。夹具只登记随机独立文件类型/ProgID/动作树，不登记产品`*`入口；结束后复核删除关联项，移除合成输入和参数回执。CI运行同一探针并保存非敏感证明。

运行：先用`scripts/dev.ps1 build --example explorer_shell_probe --features ui-preview --locked`构建，再将独立空的本地记录目录作为参数传给`target/debug/examples/explorer_shell_probe.exe`；第二次加`--extended-paths`覆盖安装记录的规范路径。记录目录拒绝重解析点。该证据证明Shell COM宿主解析和实际进程参数传递，不证明Explorer窗口渲染、真实应用导入确认、多选、现代菜单或MSI生命周期。参考：[BindToHandler](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitem-bindtohandler)、[InvokeCommand](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu-invokecommand)、[菜单消息初始化](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-icontextmenu2-handlemenumsg)。
