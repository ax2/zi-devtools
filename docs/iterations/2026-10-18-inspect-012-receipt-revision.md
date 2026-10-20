# Inspect 0.1.2 外部收据修订

日期：2026-10-18

## 目标与边界

修复 `inspect-0.1.2-candidate-01` 包外 verification 收据的目录索引编码声明。保留候选目录及原收据原样；不改候选包文件，不修改 Studio。

## 变更

- 新增 `release/plugin-candidates/inspect-0.1.2-candidate-01-verification-revision-01.json`。该文件仅把 `directoryIndexEncoding` 改为 Studio 能解析的结构化编码对象，并按实际 listed-order JSON 字节重算索引摘要；保留逐文件记录和其他证据。
- 修正 `scripts/verify_inspect_tool_open_plugin.py` 后续收据生成格式，避免以后生成自然语言字符串编码字段。
- 文件清单按斜线规范化的便携相对路径，以区分大小写的 Unicode code point 顺序排序；不折叠大小写，也不执行 Unicode 规范化。边界向量：`A.txt < a.txt < z.txt < é.txt < 中.txt`；逆序必须重排；`e + U+0301` 与预组字符 `é` 保持不同。

## 验证

- 对候选目录与原收据逐文件复核：616 个文件、2,341,254 字节、每个 SHA-256 全部匹配。
- Studio 只读接收脚本对新收据返回 `verified=true`、`directoryIndex.status=matched`、索引摘要 `1d3af04bb9e35fa9594a673ff71058e56ffbb94e516f4086dba275472d1678ee`。
- `check-plugin-manifest-profiles.mjs` 预检无声明问题。
- 验证结果明确仍为 unsigned provider candidate：`signatureVerified=false`、`runtimeAccepted=false`、`fullToolAcceptance=false`。这不是 Studio Host 安装/运行验收或签名正式包。

## 保留与限制

原候选与旧收据均未改写；新收据为单独的新文件。Studio 接口中的 `tool.open.v1` 接收结果是只读包清单/声明预检，不代表 Host 能力或工具的完整验收。
