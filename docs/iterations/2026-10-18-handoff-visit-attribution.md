# 接力成功后的访问记录归因（已修复并验证）

## 源码证据

src/app/handoff.rs的apply_handoff仅在接收成功时调用navigate(page,kind)。src/app.rs的navigate对kind=None使用catalog中该Page的第一个kind=None条目登记visit；它未获得真实Target ID。TextFlow接入Page::Data，Java/Django诊断同类工具共用Page，这种推导可能把访问次数记到通用或同类首个工具。open_entry已有按指定ID记录一次的逻辑，但同时执行各工作台切换动作，不能为了修正计数盲目重复调用而影响刚收到的数据。

## 拟修复

把页面切换与默认访问ID选择分离；普通navigate保留当前规则，接力成功明确传入Target绑定的目录ID，执行一次visit。错误与取消不得记录访问。保持选择、接收和执行三个阶段分开，不因搜索、预览或选择产生历史。

## 拟验证

在隔离偏好目录实际接收CSV后，继续接收到文本流程、Java GC、Django诊断；核对当前Page/目标状态、持久recent和usage，只增加真实目标一次且不增加共享页面首个工具。构造无效数值接收与取消，验证记录不变。保留原有接力草稿、确认、新实例、来源保留验收。

此文仅为源码审查和实现计划，尚无修复或运行通过主张。先等待handoff-access release进程85722结束，再修改生产Rust，以免运行中构建的源码与证据漂移。


## 红测与修复

原生应用fixture实际调用接力接收文本流程后，recent首项为data，预期text-flow，红测失败已归档。修复将普通navigate保留为默认页面入口逻辑，抽出navigate_with_visit；接力成功使用Target绑定目录ID调用该方法，仅执行一次visit，不重复调用open_entry改变刚收到的工作状态。Java GC、Django SQL及错误/取消回归也加入原生fixture的visits模式。完整验证待运行。工具版本1.12.1，应用仍dev.105。


## 原生复验通过

新流水线四项原生阶段均通过：收藏/最近/常用明暗（980×740、980×640逻辑窗口），CSV浅色与TSV深色滚动（980×740）。鼠标切换范围、键盘输入无结果、恢复全部、确认新实例及保留旧工作通过。实际程序接收文本流程、Java GC、Django SQL后各记一次持久访问；无效数值接收与取消均不计数。滚动到500像素以上后重开，在四个连续采样帧保持顶部；新的滚轮和搜索均可正常工作。

截图已检查稳定的深色收藏/空结果与浅色最近范围，确认/取消完整可见、文字清晰。重开瞬间截图含入场渐变，不作为稳定不透明度证据；自动化不能代替人工设备体验。证据见handoff-access-filters档案native-ui-proof.json与screenshots。完整测试、Clippy、新Release仍由session 5861继续，当前不能声明整条流水线通过。


完整Rust工作区1344通过、0失败、43忽略（60组），75项Python测试通过；四组原生阶段通过。严格Clippy仍在运行，后续Release尚未启动；同一session 5861，下一轮继续观察同一handle。当前源码与source-state.json逐项哈希一致。


严格Clippy已通过（24.65秒）。新Release已开始，session 5861仍在运行；不要重启或覆盖此构建。完整验证脚本的最终checks.json会在进程终态写入，当前尚不能称整条流水线完成。


## 1.12.1 最终验证（本节取代上文进行中状态）

同一 session 5861 正常结束，验证脚本全部 10 个阶段通过，包含四组受控原生 UI、1344 项 Rust 测试、严格 Clippy 和 Release；另有 75 项 Python 测试通过。Release 阶段用时 486.12 秒。构建后逐项复核 source-state.json 的全部源码指纹一致。

新 EXE：41574400 字节，SHA-256 `dbd67b078f630e90f36c42c32978bf4f114adfcfcc6f061e34c665a365c38e8a`。见档案 release-proof-1.12.1.json、checks-1.12.1.json 和 release-1.12.1.log。旧 1.12.0 产物记录继续保留为历史，不作为新修复证据。应用仍 0.82.0-dev.105，未发布，也不表示 Studio 插件已验收。

后续键盘审查见 2026-10-18-handoff-keyboard-review.md；其中下拉框 Esc 行为仍待原生复现，尚未修改本次构建的生产源码。
