# 安装器菜单选项：候选0.23.0

状态：原生选项页已实现，d32b8a6对应CI38140649205的production-upgrade作业通过五次实际原生点击、取消无安装副作用和公开旧版升级。五张549×345原生窗口截图已逐张查看，文字和控件可读、无明显截断。同一提交五路现全部成功；ba87fb8的CI38141305431另通过完整UI卸载且不显示选项页，已启用菜单清理正确；057e0b7的CI38143119357五路全部成功，补齐四个真实键盘场景及焦点截图检查。缩放、其他键盘路径和真实Explorer未验收。

## 用户流程

完整安装界面在开始安装前提供三种选择：

- 保留当前设置：首次安装保持关闭，修复或更新继承已有菜单设置。
- 启用右键菜单：只为当前用户添加统一入口。
- 关闭右键菜单：移除本次MSI登记；还有便携版登记时继续使用便携版入口。

默认选择保留当前设置，不需要输入命令。页面展示简短范围说明、开始和取消按钮。取消应在安装脚本执行前退出，不创建菜单、维护日志或程序文件。卸载不弹出新增选项页。

## 与现有后端的连接

继续复用Rust菜单动作与共享explorer.wxi，不在界面单独写注册表。原生MSI选项页只设置ZIMENU，实际菜单操作仍由现有Rust后端核验并执行。

UI内部属性与实际ZIMENU分开。启用映射为1，关闭映射为0，保留设置清除显式覆盖。若命令行已有合法ZIMENU，则对应选项应预选；未指定时预选保留。非法命令行参数仍由严格后端拒绝，不得被UI自动清空而绕过校验。

静默或基本UI不显示选项页，不改现有参数或继承行为；卸载跳过。外层事务的CommitReady/Committed规则不变，UI点击开始不是成功凭据。

## 验收证据

1. 仅编译时查询最终MSI的Dialog、Control、RadioButton、ControlEvent与InstallUISequence，确认三种映射、默认值和完整/静默/卸载显示条件。
2. 只在一次性托管Windows运行完整UI：实际选择三种状态、取消，检查最终菜单、登记、文件和维护日志；不能用静默参数测试代替点击事件测试。
3. 保存实际原生窗口截图，检查文本、按钮、焦点、键盘和缩放；未经查看不能宣称界面验收。
4. 静默五路矩阵保持通过，特别是公开v0.81.0升级、继承修复及两种安装移除顺序。

WiX原生控件事件与序列依据：[Publish](https://docs.firegiant.com/wix/schema/wxs/publish/)、[Show](https://docs.firegiant.com/wix/schema/wxs/show/)、[SetProperty](https://docs.firegiant.com/wix/schema/wxs/setproperty/)。属性事件顺序与显示条件需以最终MSI和实际点击结果复核。

## 完整UI卸载补充候选
新增第六次原生点击，先恢复启用状态，再以FULL模式执行卸载。窗口探针在原生调用结束前持续观察本测试进程的菜单选项窗口，出现则拒绝；调用完成后核对菜单、账本项、产品和两程序文件均清理，用户标记保留。只有对应CI成功才将“完整UI卸载未验证”移出限制列表，目前仍为候选。版本仍0.23.0，不因验证代码变化递增应用或工具版本。

## ba87fb8完整UI卸载实际结果
CI38141305431五路全部成功，production-upgrade proof确认fullUiUninstallSkipsMenuOptions及fullUiUninstallRemovesEnabledMenu为true，userDataPreserved为true。第六张ui-enable-before-uninstall.png已实际查看，控件/文字可读。完整UI模式不展示菜单选项并正常移除已启用菜单、产品/账本项/两EXE，用户文件保留。键盘、缩放、Explorer窗口及公开旧版升级失败回滚仍未验收。

## 057e0b7键盘操作实际结果

[CI38143119357](https://github.com/ax2/zi-devtools/actions/runs/38143119357)的production-upgrade作业通过四个真实键盘场景：Esc取消、首次继承后Enter、预选关闭后上键启用、预选启用后下键关闭。每次初始焦点均在已选单选项；Tab依次到开始/取消，Shift+Tab返回开始/已选项。读取全部三项原生选中状态并核对实际安装结果，不用按钮点击替代键盘操作；取消返回1602且文件、菜单、账本及日志保持不变。

只在托管Windows、当前测试进程所属前台窗口发送SendInput；失去前台或焦点不符会失败。proof保存四段键序列与focusAndSelectionVerified/sendInput=true、buttonClickUsed=false。四张对应截图已查看，单选项焦点虚线清楚、文字和按钮可读，无明显截断。保留其他两次原生鼠标按钮操作、实际安装Shell解析及完整UI卸载验证；缩放、其他键盘路径、真实Explorer窗口和公开旧版失败回滚仍未验收。此轮只补验证，应用和工具版本不变。
