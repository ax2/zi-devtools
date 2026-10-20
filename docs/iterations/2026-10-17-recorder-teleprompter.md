# 录屏本机提词器（0.10.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未修改）\
工具版本：教程录屏辅助 `0.10.0`（开发中）

## 目标

给录屏讲解增加简易本机提词浮窗，稿件只留在内存；尽可能避免提词文字进入本机 Windows 录屏采集结果，并在系统无法确认排除时隐藏正文。

## 实现

- 录屏页“教程辅助”新增提词器入口，弹出独立置顶窗口。
- 支持编辑稿件、预览、自动滚动、暂停、速度/字号调整和回到开头；长度限制为12000字符。
- 不将稿件写入持久化设置、录屏帧、日志或其它文件；关闭浮窗或程序后随进程内存销毁。
- Windows 浮窗标题包含当前进程号；查找窗口后校验 HWND 属于当前进程，再设置 `WDA_EXCLUDEFROMCAPTURE` 并读取回验。成功后本次窗口不重复调用设置 API；关闭后再次打开会重新设置。创建/回验失败时隐藏正文并显示原因。
- 文档说明外部采集、旧系统及摄像机可能不遵循该标志，不将它描述为安全或隐私保证。

## 验证与证据

- `capture_ui` synthetic fixture 在本机确认 `SetWindowDisplayAffinity` 和 `GetWindowDisplayAffinity` 对本进程窗口均成功并返回 `WDA_EXCLUDEFROMCAPTURE (0x11)`；证明 API 状态，不证明具体录屏输出的像素效果。
- 提词器滚动边界单测通过；全套 `cargo test --offline --locked` 成功，主库测试运行1058项，环境依赖测试依既有配置忽略；集成测试均通过。录屏相关 UI 测试5通过/5忽略（忽略项需解锁桌面、编码器或麦克风）。
- `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check`、`git diff --check` 全部通过；`cargo build --offline --locked --release` 成功，生成 `target/release/ZiDevTools.exe`，40,780,288 字节。
- UI preview 条件编译及 recorder UI 测试目标成功；`capture_ui` 两主题截图 1600×1125，Windows 显示亲和性设置/读取 smoke 通过。真实物理交互与最终录屏文件验证仍未覆盖。
- synthetic 主窗口截图：`../../screenshots/recorder-teleprompter-20261017/recorder-teleprompter-dark.png`（SHA-256 `DC9C0818B3969376447229B9847A3065CF8C34B15EAEBB0A0D2881AA60D9405A`）、`recorder-teleprompter-light.png`（SHA-256 `FCAD41045ADE523F991B82604065419D2FB1E28EB004A97A7746F2516FE31689`），为合成内容，不包含真实桌面或真实讲稿；输出路径是固定示例路径，不含本机用户名。此截图展示录屏页布局，不含子浮窗画面。
- 浮窗截图事件在自动化子 viewport 中未及时返回，因此停止依赖该截图作为验收条件；改为有界等待原生 API 设置与回读证明文件，避免挂起。未生成或留存不完整的提词窗截图。

## 未完成与风险

- 未通过真实录屏成片逐帧确认提词器窗口一定被排除；正式使用前需用无敏感内容测试稿自检。
- Windows 10 2004+ 才具备完整排除语义；Windows 旧版本及其它采集 API、共享软件或外部拍摄的结果未验收。
- 尚未通过真实键鼠操作检查窗口拖动/缩放、不同 DPI、多显示器布局、极长文本性能和长时间滚动。
- 因上述限制工具状态保持 `in-progress`。
