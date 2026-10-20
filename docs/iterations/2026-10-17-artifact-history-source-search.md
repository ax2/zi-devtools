# 资料库历史来源检索（0.12.0）

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：类型化资料与结果 `0.12.0`

## 修改

- 搜索工具来源 ID/版本时，除当前版本外也检查该资料的历史版本来源；来源比较大小写不敏感。
- 历史来源命中时，搜索结果标明具体版本号和来源字符串。正文检索仍只针对当前版本，避免误把旧内容当作当前材料。
- 搜索提示和目录范围说明区分“历史来源检索”与“当前版正文检索”；不改变回收站隔离、类型过滤和版本内容完整性检查。

## 验证

- 新增回归用例：资料从 `agent-runner@1.8.0` 追加为其他来源后，按旧来源查询仍命中，并返回历史版本 `v1`；当前来源检索仍正常。
- `cargo test --offline --locked --features ui-preview --quiet`：984 通过、27 忽略、0 失败；资料库全部13项测试通过。
- `cargo clippy --offline --locked --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`python scripts/sync_tools.py --check`：通过。
- 深色桌面资料库实际截图确认搜索提示在窄栏中完整可读，当前来源可直接辨认；截图保存在 `target/artifact-history-source-preview/artifact-version-diff.png`。
- `cargo build --release --offline --locked`：通过，耗时 6 分 25 秒。

## 限制

搜索只检索版本来源元数据和当前版正文；不会扫描历史版本正文、外部附件或源文件。来源检索不是来源身份认证。
