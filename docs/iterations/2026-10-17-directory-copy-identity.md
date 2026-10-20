# 目录差异复制：绑定目标目录身份（1.2.2）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：目录内容对比 `1.2.2`

## 目标与修改

按用户确认的接口和临时目录测试边界继续目录差异复制。复核发现 1.2.1 只验证目标路径仍指向普通目录；如果预览后原目录被移走，并在同一路径创建另一个普通目录，路径检查会通过，可能把数据写到替换后的目录。

复制计划现在通过 `same-file` 保存目标目录的文件系统身份。每个文件执行前重新读取当前目标身份，与预览时身份比对；若同路径已被替换成不同目录，则逐项报告失败，不创建目标文件。保留已有路径、符号链接、冲突和来源摘要检查。工具目录升至 1.2.2；程序版本保持不变。

## 验证

- `cargo test --offline directory_compare::tests -- --nocapture`：10 项通过，0 项失败。新增测试在预览后以新普通目录替换目标，验证替换目录中的标记文件保留且没有复制目标文件。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- <系统临时目录> 165 directory-copy-smoke`：真实 eframe 窗口接收合成鼠标事件；勾选差异、生成只读预览、确认执行；目标字节与所选左侧来源一致，来源仍保持原样。应用状态和复制夹具仅写入系统临时目录，未触碰用户目录。
- 截图：`screenshots/directory-copy-confirmation-20261017/directory-copy-smoke.png`（1600×1125，浅色主题）。检查了来源/目标、预览摘要、明确执行按钮和复制结果；顶部路径显示受可用宽度限制，悬停可查看完整值。
- 机器证明：`screenshots/directory-copy-confirmation-20261017/directory-copy-smoke.json`。
- `cargo fmt --check`、`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 与 `git diff --check` 均通过。
- `cargo test --offline --locked --features ui-preview`：962 项库测试中 935 通过、27 忽略、0 失败；可运行集成测试通过，需 Ollama、联网或官方 SDK 夹具的测试按标记忽略。目录测试单独复核 10 项全通过。

## 边界与遗留

该身份复核可阻止预览后目标目录被同路径替换的情况，但逐路径操作仍不能消除操作系统层面的所有并发文件系统竞争；复制保持逐文件提交，不构成整个批次的事务。自动化窗口使用合成鼠标事件，物理鼠标、触屏和多显示缩放尚未验收。未向用户目录执行复制。


## 后续复核（2026-10-17）

- 重新运行目录差异 UI 冒烟：`PASS directory copy UI: mouse selection → preview without writes → explicit copy; source preserved`。机器证明中的选择、预览先于写入、确认执行、目标字节匹配和来源保留字段均为 `true`。
- 截图经实际查看后，补充长路径输入框悬停完整路径提示；工具版本更新到 `1.2.3`，程序开发版本仍为 `0.82.0-dev.105`。
- 本次截图与 JSON 保存在 `screenshots/directory-copy-review-20261017/`。本次及此前专用临时目录均包含由冒烟窗口生成的应用状态和测试夹具；递归清理命令被执行策略拒绝，因此保留在系统临时目录，没有尝试绕过限制。
- 本轮 `cargo fmt --check`、目录专项 10 项测试、带 `ui-preview` 的 all-targets Clippy、工具目录同步检查与 Studio 插件映射检查均通过。

## 后续复核（按确认边界重跑）

- `cargo test --offline --locked directory_compare::tests -- --nocapture`：10 项通过，0 项失败。覆盖完整报告限制、目标目录被替换、迟到冲突、来源变化、取消、预览只读以及 UI 明确执行。
- `cargo run --offline --locked --example capture_ui --features ui-preview -- target\\ui-preview-directory-copy-final 165 directory-copy-smoke`：通过；合成鼠标选择差异、先生成只读预览，再确认复制；目标内容与左侧来源一致，来源保持不变。夹具位于项目 `target` 下。
- 更新截图与机器证明：[`directory-copy-smoke.png`](../../screenshots/directory-copy-confirmation-20261017/directory-copy-smoke.png)，1600×1125 浅色主题。重新检查确认截图不再包含 Windows 用户目录；机器证明仅保留测试结论字段。
- `python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check` 和 `git diff --check` 通过。Git 对已有工作区文件报告 LF/CRLF 转换提示。

## 复核记录（2026-10-17，临时目录边界）

- `cargo test --offline --locked directory_compare::tests -- --nocapture`：10 项通过，0 项失败。
- 使用 `target\ui-preview-directory-copy-20261017` 新建独立临时夹具，真实 eframe 合成鼠标交互通过：勾选差异、生成只读预览后才显式复制，目标字节与左侧来源一致，来源保持不变。截图与机器证明仅保存在项目 `target` 临时目录，未触碰用户目录。
- 已检查截图（1600×1125，浅色主题）；路径输入受宽度限制，完整路径可悬停查看。机器证明记录选择、先预览、显式执行、目标匹配和来源保留均为 `true`。

## 再验证（用户确认的接口与临时目录范围）

- 再次运行 `cargo test --offline --locked directory_compare::tests -- --nocapture`：10 项通过，0 项失败。
- 重新运行 `cargo run --offline --locked --example capture_ui --features ui-preview -- <新建的 target 临时目录> 165 directory-copy-smoke`：通过。鼠标选择差异后先生成只读预览，再确认执行；机器证明中的预览先于写入、目标内容匹配所选来源、来源保留均为 `true`。
- 本次截图与机器证明保留在 `target\ui-preview-directory-copy-confirmed-994be4b7c70b4c35960dbd432c94dcb2\`，未写入用户目录；已实际检查 1600×1125 浅色界面截图。
- `cargo fmt --check`、工具目录与 Studio 插件映射同步检查、`git diff --check` 均通过。检查期间 Git 提示工作区现有文件可能发生 LF/CRLF 转换；未改动换行配置。

## 退出时取消复制（1.2.4）

- `State` 销毁时同时请求取消目录扫描与后台复制；复制函数仍会保留已经原子提交的文件，不会回滚或删除。
- 增加状态销毁取消标记测试。复跑目录比较专项测试和真实 eframe 临时目录鼠标流程，验证选择、只读预览、显式执行、目标与来源内容及 UI 状态。
- 工具版本升至 `1.2.4`；应用版本 `0.82.0-dev.105` 保持不变，未发布。
- 最新验证：目录专项 11 项通过；全量 `cargo test --offline --locked --features ui-preview --quiet` 为 983 通过、27 忽略、0 失败，所有可运行集成测试通过；严格 Clippy、fmt、工具目录同步及 Studio 映射检查均通过。
- 最新真实 UI 冒烟截图和机器证明保存在 `target\ui-preview-directory-copy-final-4b5083cff3424cfd956ed53fe51df164\`；该隔离运行使用临时来源、目标和输出目录，未触碰用户文件。
- 仍未验证物理鼠标/触屏、跨显示器缩放与 OS 层面的全部并发文件系统竞态；复制批次按文件提交，非整体事务。

## 复制计划绑定来源目录身份（1.2.5）

- 目标目录在预览后被替换时已有身份校验；复核发现来源根目录仍只按路径重新解析。即使各文件会重新核对元数据和摘要，预览后的来源目录替换仍应直接使计划失效。
- 复制计划现在同时保存来源与目标根目录的文件系统身份。执行每个文件前会核验来源根路径仍指向预览时的目录对象；路径被替换时逐项失败，不向目标写入。
- 新增独立临时目录回归测试：预览后删除来源目录并在同一路径重建同名文件，执行被拒绝，目标文件不存在，替换来源内容保留。
- `cargo test --offline --locked directory_compare::tests -- --nocapture`：12 项通过、0 失败。全量 `cargo test --offline --locked --features ui-preview --quiet`：984 通过、27 忽略、0 失败；可运行集成测试通过。`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check`、工具目录及 Studio 插件映射同步检查通过。
- eframe 真实窗口合成鼠标冒烟在新建 `target` 临时目录通过：选择差异、生成只读预览、明确执行；机器证明中的 `previewCreatedBeforeWrite`、`destinationMatchesSelectedLeftSource`、`sourcePreserved`、`confirmedExecutionReported` 全为 `true`。截图为 1600×1125 浅色主题，已检查预览内容、目标路径、执行按钮和结果；机器证明与截图留在对应 `target/directory-copy-source-identity-*` 临时目录。
- 应用版本保持 `0.82.0-dev.105`，未发布。路径/身份核验仍无法消除所有操作系统级并发竞态；复制按文件原子提交，批次不是整体事务。

## 再验证（用户确认的接口与隔离临时目录）

- `cargo run --offline --locked --example capture_ui --features ui-preview -- <target/debug 新建目录> 0 directory-copy-smoke`：真实 eframe 渲染窗口接收合成鼠标事件，完成差异选择、生成只读预览、再显式执行复制；过程通过。
- 机器证明位于 `target\debug\directory-copy-smoke-e8e476e5cf914e57a2cb3145d3ea2913\directory-copy-smoke.json`，选择、预览先于写入、显式执行、目标与左侧来源匹配、来源保留均为 `true`。独立 SHA-256 复核来源和目标均为 `A2029121B59E4A4F89F18B7D938A8BB8AD0FCC57562044A4FEE3EAA440CCF4F8`。
- 完成态截图 `target\debug\directory-copy-smoke-e8e476e5cf914e57a2cb3145d3ea2913\directory-copy-smoke.png` 已实际查看：1600×1125，浅色主题；能看到只读计划摘要、执行按钮、复制结果和相对路径。完整路径仍通过悬停查看。
- `cargo test --offline --locked directory_compare::tests -- --nocapture`：12 项通过、0 失败；本轮全量测试为 1009 项通过、27 项忽略、0 失败，严格 Clippy、格式检查和工具目录同步检查通过。
- 夹具、应用状态和截图均保留在项目 `target/debug` 下的唯一隔离目录；未读取或写入用户目录中的文件。

## 复制预览单次执行（1.2.6）

- 完成态 UI 复核发现，同一份计划执行后仍会启用“执行复制”，再次点击只会重复报告目标冲突。现在有执行结果时禁用旧计划；取消或部分失败后须重新生成预览，再按最新目标冲突状态重试。
- 状态测试验证新预览可执行、完成后旧预览不可再次执行。工具目录升至 `1.2.6`，同步更新 `docs/tools.json`、生成清单、README 与平台路线摘要；程序版本保持 `0.82.0-dev.105`。
- `cargo test --offline --locked --features ui-preview --quiet`：1009 项通过、27 项忽略、0 项失败；`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings`、`cargo fmt --all -- --check` 与 `python scripts/sync_tools.py --check` 通过。
- eframe 鼠标流程复验通过；更新后的完成态截图与机器证明保存在 `target\debug\directory-copy-once-smoke-c496b3938ff54d0ba9d1fbb16564de85\`。截图已查看，“执行复制”按钮在成功后呈禁用状态，来源/目标与复制结果仍可见。
- `cargo build --offline --locked --release` 通过，用时 8 分 10 秒。构建产物 `target\release\ZiDevTools.exe`：40,599,552 bytes，SHA-256 `E9FE363BA9ECD4F5709613AB0CB4208F4E18DDA1A2F3335A9487C87BA5DC1D30`；这是本机验证构建，程序版本仍为 `0.82.0-dev.105`，未发布。
