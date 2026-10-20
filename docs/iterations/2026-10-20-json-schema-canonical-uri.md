# JSON Schema 本机引用路径规范化

日期：2026-10-20  
应用版本：`0.82.0`（发布门禁修正）  
工具版本：JSON Schema `0.2.1`

## 发现

GitHub Windows runner 的临时目录通过 8.3 短路径 `RUNNER~1` 暴露给测试。Schema registry 使用 canonical 长路径注册本机引用，而校验器却用用户传入的短路径构造 base URI；两种 URI 不同，离线 registry 因而拒绝解析同目录的 `address.json`。本机用户目录未使用该别名，所以此前本地测试通过。

## 修改

在 schema 验证入口先规范化用户选择的根文件路径，并将同一个 canonical 路径用于 registry 和 base URI。根目录边界检查、只允许本机 file URI、引用数量/字节上限及离线解析策略不变。工具目录中 JSON Schema 升至 `0.2.1`，新增版本历史；工具仍标记为开发中。

## 验证

- 根文件本机相对引用专项测试通过。
- JSON Schema 14 项专项测试通过。
- 全量 CI 首轮在短路径场景重现失败；修正后的提交需重新执行完整 workspace 测试、严格 Clippy、优化构建和 Explorer 安装矩阵。
- 正式 Release 与下载资产验证待门禁通过后补记。
## 修正提交本地验证补记

- `cargo fmt --all -- --check` 通过。
- `cargo test --workspace --locked` 通过：1,384 passed，0 failed；33 项因交互桌面/设备/外部模型环境缺失而 ignored。
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` 通过。
- 工具目录同步与 `--check` 通过；Studio migration 中 JSON Schema 来源版本同步为 0.2.1。
- Windows GitHub 的精确 SHA 回归、Explorer MSI 五路验收、Release 仍待本修正提交推送后完成。
