# 扫描 PDF 本机 OCR（2026-10-17）

## 目标

在现有 PDF 文本提取基础上，支持用户手动预览和同步扫描 PDF 的页面文字；保留页码，使用当前会话的 Windows OCR 语言，不引入外部运行时或网络服务。

## 发现

- 项目已有 `lopdf` 文本层提取与 Windows OCR 常驻 MTA 工作线程；图片预览和索引同步共享会话语言。
- 当前 Rust MSRV 为 1.88。采用 Windows 自带 `Windows.Data.Pdf` 可避免额外 PDFium/Poppler 部署依赖。
- 实测 Windows PDF 渲染的较大页面图像会使同一 OCR 引擎返回空文本；把合成扫描页缩至约 1200 像素后识别成功。因此 PDF OCR 的长边设为 1200 像素，而不是直接使用引擎公布的最大尺寸。

## 修改

- `Windows.Data.Pdf` 在内存中加载同一份已校验 PDF；仅对 `lopdf` 提取为空的页面渲染并 OCR，已有文本页不重复识别。
- 比对两个 PDF 解析器的页数，使用 PDF 页码与零基页面索引映射结果；缺页、重复或不完整 OCR 结果均报错。
- 单文件仍为 16 MiB、PDF 最多 200 页；每份最多 OCR 50 个空文本页。单页渲染长边最多 1200 像素、最多 800 万像素，渲染图缓冲最多 40 MiB，OCR 返回文本总量最多 4 MiB，最终正文/分块继续受现有资源预算限制。
- OCR 失败会让文档预览报错；索引同步遵循已有事务回滚，保留之前的索引。页面渲染和 OCR 运行期间不提供中断，完成后按现有取消逻辑检查。
- 多轮模型工作台的 PDF 摘录共用解析器，更新至 `0.5.0`；摘录仍仅在用户明确运行模型请求时发出。
- 更新 `rag-ingestion` 至 `1.3.0`、`rag-index` 至 `1.4.0`、开发中的 `ocr-documents` 至 `0.3.0`，更新用户文档。程序版本保持 `0.82.0-dev.105`，未发布。

## 验证

- 单元测试以合成三页 PDF 验证：文本页不调用 OCR；两张空文本页传入正确页码/渲染索引；识别文本按页码回填；空白页没有文字时跳过。
- 51 张空白/扫描页在启动 OCR 前被拒绝；测试回调确认未被执行。
- 在当前 Windows 主机显式运行忽略测试 `windows_ocr_recognizes_a_synthetic_scanned_pdf_page`，动态生成高对比度 JPEG 扫描 PDF，指定 `en-US`；图片 OCR 预检与完整 PDF 渲染、OCR、页码回填均通过，识别包含 `HELLO`。
- 运行环境此前可用缓存已复用；没有新增产品运行时可执行依赖。

## 限制与待办

- 只实测高对比度英语合成页；中文、多语言混排、倾斜、低对比度、照片阴影、复杂版式和大型真实 PDF 尚未验收。
- 使用系统用户首选或已安装 OCR 语言；不下载语言包、不访问网络、不输出文字坐标，也不能编辑或独立导出 OCR 结果。
- 加密 PDF 不支持。超过 50 个无文字页面需由用户拆分；页面图像 OCR 调用期间不能取消。
- 自动把 PDF 页面图像写入 SQLite 仅发生于用户明确启动的索引同步；扫描快照仍只记录文件元数据，不自动启动渲染或 OCR。

## 追加验证：中文 OCR 空格归一化

- 实机测试发现 Windows `zh-Hans-CN` OCR 会把高对比度合成图 `本地 OCR 测试成功` 返回为 `本 地 OCR 测 试 成 功`，直接影响中文短语字面匹配。
- 在 OCR 共用结果边界合并连续汉字之间的 ASCII 空格；中英文之间的空格及换行保持不变，并新增单元测试覆盖混排与换行。
- 将扫描 PDF fixture 构造拆成可复用 JPEG→PDF 助手，新增简体中文合成扫描 PDF 实测：从 PNG 编码 JPEG、创建扫描 PDF、Windows 原生渲染、`zh-Hans-CN` OCR、页码回填，断言归一化结果包含“测试”。测试通过。
- 运行命令：`cargo test --offline --locked --features ui-preview windows_ocr_recognizes_a_configured_chinese_scanned_pdf_page -- --ignored --nocapture`；环境变量只指向本机 target 下合成 PNG，并指定 `zh-Hans-CN`/`测试`，未访问网络。
- 本轮质量检查：`cargo fmt --check`、`git diff --check`、`python scripts/sync_tools.py --check` 通过；完整 `cargo test --offline --locked --features ui-preview` 为 1021 passed、31 ignored、0 failed；`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings` 通过；`cargo build --offline --locked --release` 通过（8m23s）。
- 文档解析与分块由 `1.3.0` 更新为 `1.3.1`，增量知识索引由 `1.4.0` 更新为 `1.4.1`，开发中的本机 OCR 文档助手由 `0.3.0` 更新为 `0.3.1`；程序版本仍为 `0.82.0-dev.105`。
- 证据只覆盖本机高对比度合成中文图片和扫描 PDF；真实扫描件、复杂排版、混合语言、低对比度或倾斜页面仍未验收，OCR 工具状态继续为开发中。

## 追加功能：导出完整解析文本

- 文档解析预览新增“导出完整文本…”；按页码/段落输出 Markdown，写入知识源与索引仍需用户另外操作。
- 导出按解析器生成的 800 字符窗口与 100 字符重叠规则重组正文，跨页/段落单独保留标题，避免导出重复重叠文本。
- 使用 `local_files::save_new_moved` 在同目录写入暂存后无覆盖发布；目标冲突时保留原文件，发布完成后不自动打开目标。
- 专项测试覆盖重组后的正文逐字一致、位置分区、空预览拒绝、成功写入和目标已存在时拒绝覆盖；`cargo test --offline --locked --features ui-preview markdown_export -- --nocapture` 通过（2项）。
- 工具版本更新：文档解析与分块 `1.4.0`，本机 OCR 文档助手 `0.4.0`；增量知识索引保持 `1.4.1`。工具目录生成与 `--check` 通过。
- 完整验证：`cargo fmt --check`、`git diff --check` 通过；`cargo test --offline --locked --features ui-preview --quiet` 为 1023 passed、31 ignored、0 failed；`cargo clippy --offline --locked --all-targets --features ui-preview -- -D warnings` 通过；`cargo build --offline --locked --release` 通过（7m52s）。程序仍为 `0.82.0-dev.105`，未发布；目标版本及中文复杂版式/真实扫描件验收仍未完成。
