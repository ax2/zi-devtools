# MSI产品归属与升级事务（更新工具0.6.0，开发中）

安装版程序不能绕过Windows Installer直接覆盖EXE。更新准备和便携助手执行前均通过原生`msi.dll`读取当前用户可见的同一UpgradeCode产品、缓存MSI身份和Application组件路径，核对Zi DevTools/ZiCode、产品GUID、已注册版本、每用户安装范围、x64架构和主程序文件名。所有句柄关闭，字符串和产品数量有界；读取MSI不会执行其自定义操作，也不会触发安装或修复。

旧版MSI没有InstallLocation，仍使用组件实际注册路径判断目录。安装版和另一目录的便携版可以共存：安装器拥有的目录拒绝直接替换，真正独立的便携目录可以准备更新。产品缓存缺失、组件无法定位、路径不存在、枚举失败或只有残留安装标记时拒绝操作，不推断为便携版。默认MSI目录的既有额外保护继续保留。尚未提供界面内安装器修复或标记清理。

新的安装器将`ARPINSTALLLOCATION`设为实际`INSTALLFOLDER`，便于系统显示和后续诊断。`MajorUpgrade`移除旧版的时机从事务之外移至`afterInstallInitialize`，使升级失败时Windows Installer能够回滚旧产品移除；旧的已发布MSI没有重写。数值MSI版本限定为三段、最大255.255.65535，不把开发版本号伪装成正式安装器版本。此阶段没有发布新正式MSI。

## 可复核验证

```powershell
cargo run --locked --example inspect_installer -- '实际安装器.msi'
cargo run --locked --example inspect_installer
```

第一个命令只读包身份、架构、组件和升级事务配置；第二个命令只读已注册产品和组件路径。它们不联网、不执行安装、自修复或卸载。检查结果本身不认证发布者签名，不能当作允许执行任意MSI的依据。

`Installer upgrade transactions`工作流仅在一次性GitHub托管Windows runner运行实际安装事务。先拒绝任何已存在的Zi DevTools产品，以真实WiX模板构建无可执行代码的小夹具：旧版缺少InstallLocation且使用旧移除顺序，新版使用本轮模板。在中文、空格、&自定义目录安装旧版；新版故意在InstallFinalize之前失败，核验旧产品注册和两个文件恢复；再成功升级，验证新产品、组件路径、文件内容及安装/便携共存；最后只按本次生成的产品GUID卸载，核验未知用户文件保留。本机只运行`--compile-only`编译及只读检查，不运行这些安装事务。

夹具不是正式程序、签名包或实际用户数据迁移测试。升级失败测试覆盖事务内旧产品移除恢复，不覆盖每一种掉电、占用、权限或重启延迟情况。参考[WiX MajorUpgrade事务时机](https://docs.firegiant.com/wix/schema/wxs/majorupgrade/)。

## 仍待接入

界面内MSI认证下载、二次确认、精确等待退出、Windows Installer调用、结果回执及重启尚未完成。当前用户应主动使用官方MSI升级/修复；不能把只读归属识别和CI夹具升级当作桌面自动升级已经完成。签名增量应用、启动健康握手、正式线上签名升级和右键菜单生命周期仍是后续工作。
