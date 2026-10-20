# SQLite 单格操作撤销与重做

日期：2026-10-18

## 目标

补齐上一轮 SQLite 单格事务编辑的当次会话恢复能力。撤销和重做都必须基于数据库身份、结构、主键与当前值核验，并要求用户再次明确确认。

## 实现

- 提交新编辑后维护最多20步进程内逆向操作记录；新的编辑会清空重做历史。
- 工具栏显示可用的撤销/重做计数。点击后展示数据库路径、表、列、主键、新旧值；用户确认后才执行事务。
- 每次确认都重新检查数据库文件身份、schema version、原值和唯一主键；不匹配则拒绝提交、刷新当前页并清空可能失效的历史。
- 应用退出后历史消失，不保存操作正文、授权、路径快照或额外数据库副本。
- SQLite 浏览器版本 `1.3.0`；通用“授权写入与变更预览”版本 `0.2.0`，仍标记为开发中。

## 验证

- `cargo test --offline --locked sqlite_browser::write::tests --no-fail-fast`：6 项通过，验证新值提交、撤销恢复原值、重做恢复新值，以及旧值并发变化、schema 变化、数据库替换、无主键/触发器/外键和非法标量拒绝。
- `cargo test --offline --locked`：库内 1098 项与集成测试全部通过；需要本机 Ollama、外网或外部官方 SDK 的测试按定义忽略。
- `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --check`、`python scripts/sync_tools.py --check` 与 `git diff --check` 通过。
- `cargo build --offline --locked --release` 通过（13m51s），生成 `target/release/ZiDevTools.exe`（41,223,168 bytes）；E 盘构建后保留 7.717 GiB 可用空间。程序开发版号保持 `0.82.0-dev.105`，未清理缓存，也未发布新版本。

## 限制

仅撤销/重做 SQLite 浏览器中的单格标量更新，最多20步且限本次运行；不覆盖其他工具写入、不恢复跨重启状态，不提供数据库备份或批量操作。外部程序对数据库的任何物理文件变化可能使历史失效，即使修改的不是目标单元格。
- `cargo run --offline --locked --features ui-preview --example capture_ui -- target/sqlite-undo-ui 167 single` 与 scene `168` dark 通过；两主题实际渲染 1600×1125 页面，目视确认接力、编辑、撤销、重做合并为一行，分页/CSV操作及数据表均未裁切；撤销/重做在空历史时正确禁用并显示 `0`。
