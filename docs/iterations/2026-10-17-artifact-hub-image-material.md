# 本机资料库图片材料

日期：2026-10-17\
程序版本：`0.82.0-dev.105`（未更改、未发布）\
工具版本：`artifact-hub 0.13.0`

## 目标

让图片流程结果能够进入本机资料库，明确预览后以版本化材料保存，并能在重开资料库后通过统一接力返回文本操作流程。

## 修改

- `artifact-hub` 增加 PNG 图片类型。SQLite 仍用既有 BLOB 保存每个版本；不增加表结构迁移。
- 接收图片时保留为内存草稿，不自动写文件。保存准备阶段将像素编码为 PNG，限制单版 16 MiB、12,000 像素边长、1,600 万像素以及 192 MiB 解码内存；保存确认仍是单独动作。
- 打开版本时校验版本哈希、PNG 头、尺寸和解码限制。资料库显示缩略图、宽高和像素数；图片内容不做 OCR 或文本索引，名称、来源和类型筛选仍可用。
- 图片版本不提供逐行文本差异。旧格式文本及其版本保持原有编码、搜索和比较路径。
- 图片可从图片工具送进资料库临时草稿；选中的资料库图片可重新进入文本操作流程，接收不会运行流程或更改 SQLite。
- 更新 `docs/tools.json` 与生成目录，工具版本推进到 `0.13.0`；主程序开发版本不变。

## 验证

- `cargo test --offline --locked artifact_hub::tests -- --nocapture`：18 项通过。
- `cargo test --offline --locked image_result_handoff_creates_a_new_text_flow_without_encoding_or_saving_pixels -- --nocapture`：通过，覆盖图片目标兼容性。
- 图片资料测试确认草稿和保存准备均不写数据库；确认后写入 SQLite，重新打开后 RGBA 透明度及像素一致；来源可搜索；图片版本拒绝逐行差异；非 PNG 和超限尺寸被拒绝。
- `cargo test --offline --locked`：1,047 项通过、0 失败、32 项忽略。
- `cargo clippy --offline --locked --all-targets -- -D warnings` 与启用 `ui-preview` 的 all-targets Clippy：通过。
- `cargo test --offline --locked --features ui-preview artifact_hub::tests`：18 项通过。
- `cargo build --release --offline --locked`：通过；产物 `target/release/ZiDevTools.exe` 为 41,012,224 字节，SHA-256 `748711bfbc23f23d2f8d7ccbb18f164cd153e9a7a556f77c9ef141920d5ebb9b2`，未更改程序版本或发布版本。
- `cargo fmt --all -- --check`、`git diff --check`、`python scripts/sync_tools.py --check`、`python scripts/sync_studio_plugin_map.py --check`：通过。

## 遗留与边界

- 本轮未做 OCR、感知哈希或以图搜图；图片仅按名称、来源和类型查找。
- PNG 编码发生在保存准备动作中。单版与像素上限能约束输入，但接近上限的复杂大图仍会占用编码时间；后续应将编码移入后台任务并反馈进度。
- 像素来源字段跨重开保留为保存时的工具来源标记；图片内部的完整 `Origin` 来源链尚未写入版本元数据。
- 完整工作区测试、Clippy 与 Release 构建已通过；当前编码仍发生在保存准备动作中，接近上限的大图可能占用较长时间，后续可把编码移入后台任务并反馈进度。
