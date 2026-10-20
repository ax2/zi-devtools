# JSON 校验插件能力（候选 0.2.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（保持不变）\
JSON 工具版本：`1.2.0`\
文本插件候选：`com.zicode.devtools.text` `0.2.0`

## 目标与修改

补齐 JSON 工具在 WASI 插件映射中的第三项操作。桌面 JSON 工具已有格式化、压缩、校验，插件原候选只声明前两项，且插件兼容模式曾将校验请求映射到格式化能力。本轮在共享 `zi-text-core` 增加独立 `devtools.text.json.validate` 能力，沿用严格 JSON 规则；独立应用原有行为不变。工具映射、插件 View、候选清单与 Studio 提供方迁移表现已同步。

界面回归还发现 Base64 解码示例与编码示例不匹配，已修正预置 Base64 字符串。没有提升程序版本号。

## 验证与候选

- `cargo test -p zi-text-core`：12 项通过；`cargo test plugin_mode_preserves_legacy_and_tool_drafts --lib`：1 项通过。
- `python scripts/verify_text_wasi.py ...`：41 组 native/WASI 输出逐字节一致；WASM 549,533 字节，声明最大内存 64 MiB，最大结果 47,191 字节。
- `python scripts/verify_text_plugin_view.py ...`：浅色桌面 1180×900、深色窄屏 390×844 两种视口通过；真实加载 WASI 模块，校验有效 JSON 并拒绝重复键，页面无控制台错误。截图保存在 `target/text-plugin-020/`。
- `cargo test --lib`：961 通过、0 失败、27 忽略。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、目录及 Studio 插件映射 `--check`、Python 编译检查均通过。
- `cargo build --release` 通过（本机 Release 配置耗时 5 分 33 秒，复用现有 Cargo 缓存）。
- 未签名候选：`release/text-plugin-0.2.0-candidate-04`；源文件摘要覆盖适配器、迁移映射和打包脚本。包外收据 `release/text-plugin-0.2.0-candidate-04-verification.json` 记录文件清单、WASI parity 与 View 证明。`candidate-01/02/03` 作为早期收据保留，不覆盖；本版迁移表明确 Node parity 不构成 Host fuel 证明。

## 边界

这是提供方未签名候选，不代表 Studio 已接入或接受；迁移表 `acceptedCapabilities` 仍为 0。没有生成 `.zicode-plugin` 容器或伪造签名。Studio 的安装、授权、Pi Broker、更新/回滚/撤权/卸载，Host 的 fuel、超时、进程隔离与输出管道行为仍待接收方验证。本机没有 Python `wasmtime` 模块，因此未执行独立 Wasmtime fuel 证明；Node WASI 回归只证明本候选模块的功能与字节一致性。
