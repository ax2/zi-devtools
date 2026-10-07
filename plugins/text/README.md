# ZiDevTools Text 0.1.0 unsigned candidate

五项纯文本能力：JSON 格式化/压缩、Base64 编码/解码、SHA-256。使用本仓库 Rust 核心和 WASI Preview 1 command；View 为自包含 HTML，无外部资源。

这是提供方开发候选目录，不是签名发行包，也不能代表 Studio 已接入。没有正式签名私钥时不生成 `.zicode-plugin`，不要求用户关闭宿主签名校验。正式签名、首次安装、升级/回滚、撤权及卸载需接收方按公开契约验收。

纯计算不保存数据，因此卸载没有用户文本数据迁移。独立 ZiDevTools 的配置和数据库不进入插件。View 编辑状态仅用于关闭前提示；重开不会恢复文本。Pi 注册是 optional 权限，由用户在 Studio 中显式授权，View 不调用模型。

独立版 JSON/Base64/SHA-256 增加可选的「插件兼容模式」：输入最多8192 UTF-8字节；JSON深度64、拒绝重复键和不安全整数；Base64严格标准编码。默认独立版保留原有宽整数和解码外层空白行为。

构建（复用现有 Cargo target）：

```powershell
cargo build -p zi-text-wasi --release --target wasm32-wasip1 --config 'target.wasm32-wasip1.rustflags=["-C","link-arg=--max-memory=67108864","-C","link-arg=-zstack-size=2097152"]'
cargo build -p zi-text-wasi
python scripts/verify_text_wasi.py --wasm <target>/wasm32-wasip1/release/zi-text-wasi.wasm --native <target>/debug/zi-text-wasi.exe --output <archive>/wasi-parity.json
python scripts/package_text_plugin.py --wasm <target>/wasm32-wasip1/release/zi-text-wasi.wasm --proof <archive>/wasi-parity.json --view-proof <archive>/view-qa.json --output release/<new-candidate>
```

校验只用仓库内契约快照，不读取 Studio checkout。Node WASI 仅验证本项目模块的功能，不作为生产沙箱或10M fuel验收。签名后的最终容器摘要应写入包外verification.json；当前 unsigned 目录仅提供有序文件清单摘要，packageSha256为null，不以目录摘要冒充包摘要。
