# 资料库过期搜索取消

日期：2026-10-18\
工具版本：类型化资料与结果 `0.13.1`\
程序版本：`0.82.0-dev.105`（保持不变）

## 问题

资料库在后台扫描当前版本正文。用户改写搜索词、切换筛选或刷新时，界面会忽略旧查询结果，但旧 SQLite 扫描和正文校验仍会继续，直到最多 512 项资料处理完毕。关闭应用时，同一后台线程也可能继续工作。

## 修改

- 将 UI 搜索 generation 同步给后台线程；旧 generation 在 SQLite progress handler 和 Rust 逐项处理边界检查。
- 输入变化和列表刷新会使当前扫描失效；关闭应用时使当前 generation 失效，SQLite 扫描收到取消后后台线程随请求通道关闭而退出。
- 旧查询结果仍按 generation 丢弃；新查询不会等待旧查询扫描完整个资料库。取消不会修改资料库、草稿或索引。
- “类型化资料与结果”更新为 `0.13.1`，同步工具清单与生成文档。

## 验证

- 新增回归用例：generation 已过期时，拒绝执行查询且不创建资料库文件。
- 全量测试、严格 Clippy、格式检查、工具目录同步检查及 Release 构建待本轮运行后记录。

## 限制

SQLite progress handler 按 SQLite 指令批次触发；已经进入单个 Rust 字段处理时，会在该资料处理结束前才检查新 generation。
## 本轮验证结果

- 资料库定向测试：20 项通过；新增过期 generation 与活动 SQLite 查询中断回归测试通过。
- 全量 cargo test --offline --locked --quiet：1,079 项通过、32 项忽略、0 失败。
- cargo clippy --offline --locked --all-targets -- -D warnings、cargo fmt --all -- --check、python scripts/sync_tools.py --check、git diff --check：通过。
- cargo build --offline --locked --release：通过，耗时 10m34s；target/release/ZiDevTools.exe 大小 41333248 bytes，生成时间 2026-10-18 06:26:57。构建后 E 盘可用空间 5256220672 bytes。
- 开发版保持 0.82.0-dev.105；未创建正式发行包。


## 最终验证

- Release 构建通过，耗时 12m36s；target/release/ZiDevTools.exe 为 41333248 bytes，生成时间 2026-10-18 06:43:58；构建后 E 盘可用空间 345116672 bytes。
- 全量测试 1,079 通过、32 忽略；严格 Clippy、格式检查、工具目录同步、差异空白检查与迭代文档控制字符审计通过。
- 开发版保持 0.82.0-dev.105，未创建正式发行包。
