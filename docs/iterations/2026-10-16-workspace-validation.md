# 全工作区验证（2026-10-16）

## 范围

对当前 `main` 工作树的所有 crate、主程序、MCP 可执行文件和集成测试执行项目验证门槛。验证期间保留已有未提交改动；不发布、不安装 MSI、不递增程序版本。

## 结果

- `cargo test --offline --locked`：全工作区 **967 passed / 0 failed / 36 ignored**。忽略项需本地 Ollama 模型、显式 MCP SDK fixture 或外网 HTTPS 等外部条件；对应测试在测试名中声明依赖。
- `cargo clippy --offline --all-targets --locked -- -D warnings`：通过，0 warning。
- `cargo build --offline --release --locked`：通过，5分23秒；启用仓库 Release LTO/单代码生成单元配置。
- `cargo fmt --check`、`python scripts/sync_tools.py --check`、`git diff --check`：通过。

## 构建产物核对

程序版本仍为 `0.82.0-dev.105`。本次构建输出仅留在可重建的 `target/release`：

- `ZiDevTools.exe`：34,785,280 bytes，SHA-256 `70573BF01A45440BA2DB84E13FCB9454A8A924C24B7F9FC7929240B38B38928D`
- `ZiDevToolsMcp.exe`：2,194,432 bytes，SHA-256 `16274D6959373FC9DACC7972EF60A62DBD6F9083FB467281327552B3E34EEEC8`

以上不是正式发行目录或发布包。当前证据不覆盖需要真实 Ollama/SDK/外网的忽略测试、开发机 MSI 安装或真实 Explorer 内容导入验收。
