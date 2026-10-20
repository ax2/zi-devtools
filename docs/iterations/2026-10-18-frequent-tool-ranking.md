# 常用工具排序统一

日期：2026-10-18\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`global-launcher 1.5.2`

## 目标

让主工具库和快捷面板对“常用”视图提供一致的排序，工具数量继续增长时优先展示用户刚刚常用的工具。

## 发现

- 主工具库已经按使用频次降序、最近访问次序破同分。
- 快捷面板仅按频次排序，同频次会退回工具目录顺序，因此两个入口可能呈现不同的排序。
- `Preferences::recent` 在记录访问时将最新工具插入首位，适合作为共享破同分依据。

## 修改

- 抽出共享的使用频次排序函数，同时用于工具库和快捷面板。
- 排序依次使用使用频次降序、最近访问顺序、稳定的原始目录顺序。
- 为同频次排序加入回归测试，并更新 `global-launcher` 工具版本与导航文档。

## 验证

- `cargo fmt --all -- --check`：通过。
- `cargo test --offline --locked frequent_tools_use_recent_visit_to_break_usage_ties`：通过。
- `cargo test --offline --locked`：单元测试 1,058 项通过、32 项忽略；集成测试 52 项通过、9 项忽略；0 失败。
- `cargo clippy --offline --locked --all-targets -- -D warnings`：通过。
- `python scripts/sync_tools.py` 与 `python scripts/sync_tools.py --check`：通过；目录为 92 项已实现、62 项规划/开发中，Studio 迁移映射 154 项，accepted capability 为 0。
- `git diff --check`：通过。
- `cargo build --release --offline --locked`：通过，耗时 10 分 31 秒。EXE 41,067,520 字节，SHA-256 `C8E3B415437F2018AFD1E3201EA97C0D7E80137B8C820B7D636A30BF2C9B4C9B`；构建后 E 盘剩余 7.84 GB。

## 遗留与风险

- 使用频次仍是本地偏好统计；此变更不改变统计方式、数据格式或跨窗口合并行为。
- 真实托盘界面中的排序需在完成自动化检查后再做人工确认。

## 发布

未发布。
