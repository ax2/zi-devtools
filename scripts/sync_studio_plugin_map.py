"""Keep all standalone catalog items mapped, without claiming plugin acceptance."""
from pathlib import Path
import argparse
import json
import re

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--check", action="store_true")
args = parser.parse_args()
catalog = json.loads((ROOT / "docs/tools.json").read_text(encoding="utf-8"))
compute = set("json base64 url sha256 jwt regex number ascii-codes symbol-library ascii-art qr color text-stats html-escape text-escape case-convert yaml hex lines url-inspect cidr diff json-path json-diff data-schema data-transform csv-merge unicode java-trace django-trace java-thread-dump java-gc-log sql-format".split())
report_scopes = {
    "java-thread-dump": "jstack/jcmd导入文本分析；进程采集另需Host",
    "java-dependencies": "Maven/Gradle依赖导入文本分析；构建执行另需Host",
    "java-gc-log": "GC日志导入文本分析；文件获取/进程采集另需Host",
    "java-jfr": "JFR导入JSON分析；二进制读取/导出另需Host",
    "spring-config": "两份JSON/YAML配置差异与脱敏；在线Actuator另需Host",
    "django-migrations": "迁移计划/SQL导入文本分析；数据库/迁移执行另需Host",
    "django-sql": "SQL查询日志JSON数组归一化与重复分析；执行另需Host",
    "django-urls": "URL导入列表检索；项目加载另需Host",
    "django-drf": "两份OpenAPI JSON差异；联网另需Host",
    "django-checks": "检查输出导入分析；manage.py执行另需Host",
    "celery-diagnostics": "Celery任务日志导入分析；Broker/Worker操作另需Host",
}
core_source = (ROOT / "crates/zi-diagnostics-core/src/lib.rs").read_text(encoding="utf-8")
report_actions = {source: action for action, source in re.findall(
    r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"', core_source)}
assert set(report_actions) == set(report_scopes), "Report migration must match the actual core registry"
compute.update(report_scopes)
pilot = {
    "json": ["devtools.text.json.format", "devtools.text.json.minify", "devtools.text.json.validate"],
    "base64": ["devtools.text.base64.encode", "devtools.text.base64.decode"],
    "sha256": ["devtools.text.sha256"],
    "sql-format": ["devtools.text.sql.format"],
}
transform_source = (ROOT / "crates/zi-text-core/src/transforms.rs").read_text(encoding="utf-8")
transform_actions = {}
for operation, source, title in re.findall(
    r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"', transform_source):
    transform_actions.setdefault(source, []).append((operation, title))
assert sum(map(len, transform_actions.values())) == 23 and len(transform_actions) == 9
inspect_source = (ROOT / "crates/zi-inspect-core/src/lib.rs").read_text(encoding="utf-8")
inspect_actions = {}
for operation, source, title in re.findall(
    r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"', inspect_source):
    inspect_actions.setdefault(source, []).append((operation, title))
assert sum(map(len, inspect_actions.values())) == 8 and len(inspect_actions) == 5
trace_source = (ROOT / "crates/zi-trace-core/src/lib.rs").read_text(encoding="utf-8")
trace_actions = re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"', trace_source)
assert len(trace_actions) == 2
candidate_ids = {source for _, source, _ in trace_actions} | set(pilot) | set(report_scopes) | set(transform_actions) | set(inspect_actions) | {"spring-actuator"}
ids = {tool["id"] for tool in catalog["tools"]}
assert compute <= ids
rows = []
for tool in catalog["tools"]:
    if tool["status"] != "implemented":
        category = "standalone_incomplete"
        reason = "独立版整体尚未完成；已有开发子集不等于该功能的插件交付。"
    elif tool["id"] in compute:
        category = "compute_candidate"
        reason = "文本/数据纯计算迁移候选；需抽离核心、确定有界协议并实际构建验收，尚未接入。"
    else:
        category = "await_host"
        reason = "需资源、服务、桌面或通用扩展Host接口；不得用路径直读、独立EXE或本地HTTP绕过。"
    rows.append(dict(sourceToolId=tool["id"], title=tool["name"], sourceVersion=tool["tool_version"],
                     sourceStatus=tool["status"], migration=category, pluginStatus="candidate" if tool["id"] in candidate_ids else "not_delivered",
                     candidateCapabilities=pilot.get(tool["id"], []) + (["devtools.diagnostics."+report_actions[tool["id"]]] if tool["id"] in report_scopes else []) + ["devtools.transforms." + op for op, _ in transform_actions.get(tool["id"], [])] + ["devtools.inspect." + op for op, _ in inspect_actions.get(tool["id"], [])],
                     pilotCapabilities=pilot.get(tool["id"], []),
                     operationScopes=([dict(operationId=report_actions[tool["id"]], scopeKind="completed_imported_report_subset", scope=report_scopes[tool["id"]],
                         sourceVersion=tool["tool_version"], pluginStatus="candidate",
                         core="zi-diagnostics-core/0.1.4", adapter="zi-diagnostics-wasi/0.1.4",
                         packageId="com.zicode.devtools.diagnostics", packageVersion="0.1.4",
                         hostProfile="compute.wasi.v1@=1.0.0-rc.1", diagnosticsProfile="diagnostics.compute.wasi.v1.proposal",
                         diagnosticsSchemaVersion="2.0.0-rc.1", requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1","output.schema.v1":"=1.0.0-rc.1","tool.directory.v1":"=1.0.0-preview.1"},
                         candidateDirectory="release/plugin-candidates/diagnostics-0.1.4-candidate-02",
                         verificationReceipt="release/plugin-candidates/diagnostics-0.1.4-candidate-02-verification.json",
                         runtimeVerification="native_wasi_byte_parity_and_independent_wasmtime_budget_fixtures",
                         viewVerification="public_sdk_fixture_actual_wasi_11_actions_two_viewports",
                         blockingReason="unsigned提供方候选目录；诊断schema/profile/错误码及完整Host生命周期待验收")]
                         if tool["id"] in report_scopes else [dict(operationId=op, title=title,
                             scopeKind="complete_transform_operation_within_plugin_budget", scope=tool["scope"],
                             sourceVersion=tool["tool_version"], pluginStatus="candidate",
                             core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
                             packageId="com.zicode.devtools.transforms", packageVersion="0.1.1",
                             requiredProfiles={"tool.directory.v1":"=1.0.0-preview.1"},
                             limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
                             candidateDirectory="release/plugin-candidates/transforms-0.1.1-candidate-01",
                             verificationReceipt="release/plugin-candidates/transforms-0.1.1-candidate-01-verification.json",
                             runtimeVerification="174_independent_expected_vectors_native_wasi_parity_and_wasmtime_36_0_2_budget",
                             viewVerification="public_sdk_fixture_actual_wasi_23_actions_desktop_mobile_light_dark",
                             blockingReason="unsigned提供方候选；完整Host安装/权限/生命周期/Pi待验收")
                             for op, title in transform_actions[tool["id"]]] if tool["id"] in transform_actions else [dict(operationId=op, title=title,
                             scopeKind="complete_inspection_operation_within_plugin_budget", scope=tool["scope"],
                             sourceVersion=tool["tool_version"], pluginStatus="candidate",
                             core="zi-inspect-core/0.1.1", adapter="zi-inspect-wasi/0.1.1",
                             packageId="com.zicode.devtools.inspect", packageVersion="0.1.2",
                             limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
                             requiredProfiles={"tool.directory.v1": "=1.0.0-preview.1", "tool.open.v1": "=1.0.0-preview.1"},
                             candidateDirectory="release/plugin-candidates/inspect-0.1.2-candidate-01",
                             verificationReceipt="release/plugin-candidates/inspect-0.1.2-candidate-01-verification-revision-01.json",
                             runtimeVerification="121_native_wasi_byte_parity_wasmtime_36_0_2_budget_and_fresh_process_cold_pass",
                             viewVerification="tool_open_v1_positive_unknown_slow_callback_actual_browser_light_dark; 8 actions map to selectors only",
                             blockingReason="unsigned 0.1.2新候选已完成提供方验证；tool.open.v1的Host安装/撤权/切场景仍待接收方验收，0.1.1冻结保留")
                             for op, title in inspect_actions[tool["id"]]] if tool["id"] in inspect_actions else [dict(scopeKind="declared_standalone_scope",
                             scope=tool["scope"], sourceStatus=tool["status"], sourceVersion=tool["tool_version"],
                             pluginStatus="candidate" if tool["id"] in pilot else "not_delivered",
                             blockingReason=reason)]), reason=reason))
for row in rows:
    if row["sourceToolId"] in inspect_actions:
        row["reason"] = "共享纯Rust检查核心的全部既有操作均有界迁移；unsigned 0.1.2新候选增加tool.open.v1只定位UI、不执行能力，Studio Host/Pi待接收方验收。"
        row["packageId"] = "com.zicode.devtools.inspect"
        row["packageVersion"] = "0.1.2"
        row["requiredProfiles"] = {"tool.directory.v1": "=1.0.0-preview.1", "tool.open.v1": "=1.0.0-preview.1"}
        row["candidateDirectory"] = "release/plugin-candidates/inspect-0.1.2-candidate-01"
        row["verificationReceipt"] = "release/plugin-candidates/inspect-0.1.2-candidate-01-verification-revision-01.json"
disk_row = next(row for row in rows if row["sourceToolId"] == "disk-inspector")
disk_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.disk-inspector.scan"],
    packageId="com.zicode.devtools.disk-inspector", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/disk-inspector-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/disk-inspector-0.1.0-candidate-01-verification.json",
    reason="目录用量只读候选已通过真实Windows Worker与原生对照；Studio工具页和完整Host验收在Studio #29跟踪。",
)
disk_row["operationScopes"].append(dict(
    operationId="devtools.disk-inspector.scan", scopeKind="bounded_confirmed_directory_usage",
    scope="消费Host确认的相对名称、类型和逻辑长度；汇总逻辑字节、直属子目录与大文件前20，并标出部分结果。",
    sourceVersion=disk_row["sourceVersion"], pluginStatus="candidate",
    hostProfile="resource.local-directory.usage.draft1",
    analysisLimits=dict(snapshotBytes=2 * 1024 * 1024, analyzedBytes=65536, reportBytes=49152),
    runtimeVerification="16_fixed_native_and_production_windows_worker_vectors",
    blockingReason="独立未签名候选；正常Studio工具页、工作区外选址和Explorer入口继续验收。",
))

duplicates_row = next(row for row in rows if row["sourceToolId"] == "duplicate-files")
duplicates_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.duplicate-files.scan"],
    packageId="com.zicode.devtools.duplicate-files", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/duplicate-files-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/duplicate-files-0.1.0-candidate-01-verification.json",
    reason="重复文件只读报告候选通过生产Windows Worker与原生适配器对照；Studio正常工具页和完整Host验收在Studio #29跟踪。",
)
duplicates_row["operationScopes"].append(dict(
    operationId="devtools.duplicate-files.scan", scopeKind="bounded_confirmed_directory_digests",
    scope="消费Host确认的相对路径、逻辑长度和SHA-256；统计同大小同摘要文件组和理论冗余量，扫描覆盖与展示裁剪分别标记。",
    sourceVersion=duplicates_row["sourceVersion"], pluginStatus="candidate",
    hostProfile="resource.local-directory.duplicates.draft1",
    analysisLimits=dict(snapshotBytes=61440, hashedFiles=256, hashedBytes=268435456, reportBytes=49152),
    runtimeVerification="24_fixed_native_and_production_windows_worker_vectors",
    blockingReason="独立未签名候选；Studio确认选址、授权、取消、UI和主程序分发继续验收。",
))

compare_row = next(row for row in rows if row["sourceToolId"] == "file-compare")
compare_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.directory-compare.scan"],
    packageId="com.zicode.devtools.directory-compare", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/directory-compare-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/directory-compare-0.1.0-candidate-01-verification.json",
    reason="只读目录对比候选通过生产Windows Worker与原生适配器对照；双目录授权和工具页由Studio #29验收，复制等完整独立版能力另行跟踪。",
)
compare_row["operationScopes"].append(dict(
    operationId="devtools.directory-compare.scan", scopeKind="bounded_confirmed_directory_comparison",
    scope="按精确相对路径对比两侧类型、逻辑大小和SHA-256；清点不足时不推断单侧缺失；不写入文件。",
    sourceVersion=compare_row["sourceVersion"], pluginStatus="candidate",
    hostProfile="resource.local-directory.compare.draft1",
    analysisLimits=dict(snapshotBytes=61440, comparedPaths=256, reportBytes=49152),
    runtimeVerification="27_fixed_native_and_production_windows_worker_vectors",
    blockingReason="独立未签名候选；双目录选址、Host授权、取消、UI、CSV导出和复制范围继续跟踪。",
))

merge_row = next(row for row in rows if row["sourceToolId"] == "csv-merge")
merge_ids = ["devtools.table-merge." + mode for mode in ("left", "inner", "append", "columns")]
merge_row.update(
    pluginStatus="candidate", candidateCapabilities=merge_ids, scopeComplete=False,
    packageId="com.zicode.devtools.table-merge", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/table-merge-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/table-merge-0.1.0-candidate-01-verification.json",
    reason="四种两表合并共享原生解析和关联核心，逐侧选择 CSV、TSV 或 JSON；原 Compute 冻结包保留。",
)
for operation in merge_ids:
    merge_row["operationScopes"].append(dict(operationId=operation,
        scopeKind="bounded_independent_table_formats",
        scope="两表各 8 KiB、分别解析，零资源权限，按类型匹配并保留有效的 64 位整数。返回分页预览所需表结构和 CSV。",
        sourceVersion=merge_row["sourceVersion"], pluginStatus="candidate",
        limits=dict(inputUtf8BytesPerTable=8192, serializedRequestAndResultBytes=49152),
        runtimeVerification="independent_native_and_windows_production_worker_vectors",
        blockingReason="独立版更大表格、应用到原始草稿及其余窗口流程由 Studio #28 跟踪。",
    ))

schema_row = next(row for row in rows if row["sourceToolId"] == "json-schema")
schema_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.json-schema.validate"], scopeComplete=False,
    packageId="com.zicode.devtools.json-schema", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/json-schema-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/json-schema-0.1.0-candidate-01-verification.json",
    reason="纯离线粘贴候选已通过真实 Windows Worker。现代 Draft 和根文件引用由 Studio #29 继续跟踪。",
)
schema_row["operationScopes"].append(dict(
    operationId="devtools.json-schema.validate", scopeKind="bounded_offline_schema_validation",
    scope="Draft 4/6/7（默认7），实例和Schema各8KiB，仅支持文档内引用。最多100条脱敏错误，长报告提前截断。",
    sourceVersion=schema_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputUtf8BytesPerField=8192, serializedRequestAndResultBytes=49152, maxDisplayedErrors=100),
    runtimeVerification="23_independent_windows_production_worker_vectors",
    blockingReason="2019-09与2020-12受现有fuel限制；根文件、外部引用和原生容量继续追踪。",
))

crop_row = next(row for row in rows if row["sourceToolId"] == "image-crop-annotate")
crop_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.image.crop"], scopeComplete=False,
    packageId="com.zicode.devtools.image-crop", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/image-crop-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/image-crop-0.1.0-candidate-01-verification.json",
    reason="原生图片核心的单张像素裁剪候选已通过 Windows Worker。标注和可视选区继续在 Studio #29 跟踪。",
)
crop_row["operationScopes"].append(dict(
    operationId="devtools.image.crop", scopeKind="bounded_confirmed_pixel_crop",
    scope="一张已确认的静态 PNG/JPEG/WebP，按原始像素坐标产生无损 PNG，不改原件。",
    sourceVersion=crop_row["sourceVersion"], pluginStatus="candidate",
    hostProfile="resource.wasi.image.crop.v1@=1.0.0-preview.1",
    limits=dict(inputBytes=32 * 1024 * 1024, outputBytes=32 * 1024 * 1024, edge=4096, pixels=16_000_000),
    runtimeVerification="fixed_synthetic_windows_production_worker_vectors",
    blockingReason="单张裁剪子集，交互选区、标注、批量和原生界面仍待。",
))

log_row = next(row for row in rows if row["sourceToolId"] == "log-viewer")
log_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.log-viewer.filter", "devtools.log-viewer.tail"], scopeComplete=False,
    packageId="com.zicode.devtools.log-viewer", packageVersion="0.2.0",
    candidateDirectory="release/plugin-candidates/log-viewer-0.2.0-candidate-01",
    verificationReceipt="release/plugin-candidates/log-viewer-0.2.0-candidate-01-verification.json",
    reason="单份确认 UTF-8 日志的多行事件、级别、字面筛选和有界末尾窗口已通过 Windows Worker。",
)
log_row["operationScopes"].append(dict(
    operationId="devtools.log-viewer.filter", scopeKind="confirmed_local_log_event_preview",
    scope="按原生事件规则识别多行事件，在本机按级别和区分大小写的字面正文筛选。报告总匹配数及前12条。",
    sourceVersion=log_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=16384, queryBytes=256, previewBytes=512, displayedEvents=12),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="独立版的大日志流式索引、分页、时间定位与轮转尚未接入。",
))

log_row["operationScopes"].append(dict(
    operationId="devtools.log-viewer.tail", scopeKind="bounded_log_tail_snapshot",
    scope="只分析已确认日志快照的末尾 16 KiB，并用前一字节核对整行。报告窗口、有效分析范围与边界裁切。",
    sourceVersion=log_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(confirmedSnapshotBytes=32 * 1024 * 1024, windowBytes=16384, lookbehindBytes=1,
                queryBytes=256, eventPreviewBytes=512, displayedEvents=12),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="整份快照仍由 Host 接收；无全文件分析、实时轮转、流式索引、分页和新的文件权限。",
))

pdf_row = next(row for row in rows if row["sourceToolId"] == "pdf-text")
pdf_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.pdf-text.page"], scopeComplete=False,
    packageId="com.zicode.devtools.pdf-text", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/pdf-text-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/pdf-text-0.1.0-candidate-01-verification.json",
    reason="复用原生文档解析的 PDF 库，已确认单份标准 PDF 单页嵌入文字候选。",
)
pdf_row["operationScopes"].append(dict(
    operationId="devtools.pdf-text.page", scopeKind="bounded_pdf_embedded_page_text",
    scope="确认一份本机 PDF，显式选择一页并检查适用的嵌入文本。区分空白、图像和无可提取内容。",
    sourceVersion=pdf_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(confirmedPdfBytes=131072, pages=32, previewBytes=6144),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="候选只包含单页嵌入文本，保留独立版 PDF 专用 UI、OCR、全文检索和结果另存。",
))

ingest_row = next(row for row in rows if row["sourceToolId"] == "rag-ingestion")
ingest_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.docx.preview.chunks"], scopeComplete=False,
    packageId="com.zicode.devtools.docx-preview", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/docx-preview-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/docx-preview-0.1.0-candidate-01-verification.json",
    reason="复用原生文档解析与分块核心，提供确认的 DOCX 正文有界只读预览。",
)
ingest_row["operationScopes"].append(dict(
    operationId="devtools.docx.preview.chunks", scopeKind="bounded_confirmed_docx_chunks",
    scope="确认一份本机 DOCX，仅解析 word/document.xml。保留原生段落位置、800字分块和100字重叠。",
    sourceVersion=ingest_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(confirmedDocxBytes=131072, extractedXmlBytes=131072, zipEntries=256,
                previewRows=8, previewTextBytes=6144),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅 DOCX 正文分块预览，不接宏或媒体、其它格式、OCR、知识源索引和完整导出。",
))

calendar_row = next(row for row in rows if row["sourceToolId"] == "calendar-planner")
calendar_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.calendar.ics.preview"], scopeComplete=False,
    packageId="com.zicode.devtools.ics-preview", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/ics-preview-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/ics-preview-0.1.0-candidate-01-verification.json",
    reason="复用原生日历的属性及折行解析，提供经确认的 ICS 只读日程预览。",
)
calendar_row["operationScopes"].append(dict(
    operationId="devtools.calendar.ics.preview", scopeKind="bounded_confirmed_ics_preview",
    scope="确认一份本机 ICS，保留全天/浮动/UTC/命名时区、原始起止和 RRULE，展示有限日程。",
    sourceVersion=calendar_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(confirmedFileBytes=16384, previewEvents=8, previewTextBytes=6144),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅 ICS 文件预览，尚不含日程导入/导出、同步、时区换算、重复展开或提醒。",
))

flow_row = next(row for row in rows if row["sourceToolId"] == "text-flow")
flow_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.flow.recipe.inspect"], scopeComplete=False,
    packageId="com.zicode.devtools.flow-recipe", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/flow-recipe-0.1.0-candidate-02",
    verificationReceipt="release/plugin-candidates/flow-recipe-0.1.0-candidate-02-verification.json",
    reason="复用原生格式 1–4 与动作签名校验，提供经确认的文本流程配方只读检查。",
)
flow_row["operationScopes"].append(dict(
    operationId="devtools.flow.recipe.inspect", scopeKind="bounded_confirmed_flow_recipe_inspection",
    scope="确认一份本机配方 JSON，校验 16 步以内的注册动作、版本和材料类型衔接，展示操作影响和结果端口。",
    sourceVersion=flow_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(confirmedFileBytes=65536, recipeSteps=16, serializedResultBytes=16384),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅检查配方；运行、编辑、保存、命名实例、配方库及结果接力另行跟踪。",
))

rename_row = next(row for row in rows if row["sourceToolId"] == "file-rename")
rename_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.files.rename.preview"], scopeComplete=False,
    packageId="com.zicode.devtools.rename-preview", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/rename-preview-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/rename-preview-0.1.0-candidate-01-verification.json",
    reason="复用原生字面替换与文件名检查，接收显式确认的一层目录名称快照。",
)
rename_row["operationScopes"].append(dict(
    operationId="devtools.files.rename.preview", scopeKind="bounded_confirmed_rename_preview",
    scope="对确认的一层目录名称预览改名，检测未改变、仅大小写、相同提议、已有名称和非法名称。",
    sourceVersion=rename_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(snapshotBytes=65536, entries=64, sourceNameBytes=12288, targetNameBytes=16384,
                ruleFieldUtf8Bytes=128, serializedResultBytes=49152),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅目录名称快照的预览；原生多选、实际变更、竞态复核和撤销未接入。",
))

sources_row = next(row for row in rows if row["sourceToolId"] == "rag-sources")
sources_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.knowledge.source.preflight"], scopeComplete=False,
    packageId="com.zicode.devtools.knowledge-preflight", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/knowledge-preflight-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/knowledge-preflight-0.1.0-candidate-01-verification.json",
    reason="复用原生文档格式和排除规则，预检已确认的一层目录名称快照。",
)
sources_row["operationScopes"].append(dict(
    operationId="devtools.knowledge.source.preflight", scopeKind="bounded_confirmed_knowledge_source_preflight",
    scope="按文件名、类型与排除规则统计可能纳入知识源的文档，标明未扫描的子目录。",
    sourceVersion=sources_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(snapshotBytes=65536, entries=64, nameBytes=12288, exclusionRules=8,
                exclusionUtf8Bytes=960, previewCandidates=24, serializedResultBytes=24576),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅目录名称预检；内容、文件大小、递归、指纹、变更历史、保存知识源和重建索引未接入。",
))

date_row = next(row for row in rows if row["sourceToolId"] == "advanced-calculator")
date_ops = {
    "difference": "公历日期差，返回带符号天数。",
    "offset": "公历按天、月或年偏移，月末规则可明确选择。",
    "workday_count": "周一至周五计数，结束日包括在范围内，可包含开始日。",
    "workday_offset": "周一至周五按正负工作日偏移，跳过周末。",
}
date_row.update(
    pluginStatus="candidate", candidateCapabilities=["devtools.date." + name for name in date_ops],
    scopeComplete=False, packageId="com.zicode.devtools.date-calculator", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/date-calculator-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/date-calculator-0.1.0-candidate-01-verification.json",
    reason="复用原生日期与工作日计算规则，提供公历日期四项离线操作。",
)
for action, scope in date_ops.items():
    date_row["operationScopes"].append(dict(
        operationId="devtools.date." + action, scopeKind="bounded_offline_civil_date_calculation",
        scope=scope, sourceVersion=date_row["sourceVersion"], pluginStatus="candidate",
        limits=dict(minYear=1,maxYear=9999,maxAbsoluteOffset=4000000,requestBytes=4096,serializedResultBytes=2048),
        runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
        blockingReason="仅公历日期和工作日计算，其他计算器功能、工作表和日程接力未迁移。",
    ))

matrix_ops = {
    "add": "同样尺寸的实数矩阵加法。",
    "subtract": "同样尺寸的实数矩阵减法。",
    "multiply": "维度相容的实数矩阵乘法。",
    "transpose": "实数矩阵转置。",
    "determinant": "实数方阵行列式，奇异时返回零。",
    "inverse": "实数可逆方阵求逆。",
    "solve": "求解 AX=B，A 必须可逆，B 可有多个右端项。",
}
matrix_package = dict(
    packageId="com.zicode.devtools.matrix-calculator", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/matrix-calculator-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/matrix-calculator-0.1.0-candidate-01-verification.json",
)
date_row["candidatePackages"] = [
    dict(packageId=date_row["packageId"], packageVersion=date_row["packageVersion"],
         candidateDirectory=date_row["candidateDirectory"], verificationReceipt=date_row["verificationReceipt"],
         capabilities=date_row["candidateCapabilities"].copy()),
    dict(**matrix_package, capabilities=["devtools.matrix." + name for name in matrix_ops]),
]
date_row["candidateCapabilities"] += ["devtools.matrix." + name for name in matrix_ops]
date_row["reason"] = "日期候选保留原四项离线操作，矩阵候选另提供七项实数计算。独立版计算器范围尚未完整迁移。"
for action, scope in matrix_ops.items():
    date_row["operationScopes"].append(dict(
        operationId="devtools.matrix." + action, scopeKind="bounded_offline_real_matrix_calculation",
        scope=scope, sourceVersion=date_row["sourceVersion"], pluginStatus="candidate",
        **matrix_package,
        limits=dict(matrixRows=8, matrixColumns=8, matrixBytes=4096, cellBytes=96,
                    requestBytes=16384, serializedResultBytes=12288),
        runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
        blockingReason="仅七项实数 TSV 运算。原生网格、变量与工作表未迁移；Studio Host 安装及工具中心尚待验收。",
    ))

statistics_ops = {
    "sum": "实数数列求和。",
    "mean": "实数数列算术平均值。",
    "median": "实数数列中位数。",
    "population_variance": "总体方差（分母 n）。",
    "sample_variance": "样本方差（分母 n−1，至少两个值）。",
    "population_standard_deviation": "总体方差的近似平方根。",
    "sample_standard_deviation": "样本方差的近似平方根（至少两个值）。",
}
statistics_package = dict(
    packageId="com.zicode.devtools.numeric-statistics", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/numeric-statistics-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/numeric-statistics-0.1.0-candidate-01-verification.json",
)
statistics_capabilities = ["devtools.statistics." + name for name in statistics_ops]
date_row["candidatePackages"].append(dict(**statistics_package, capabilities=statistics_capabilities))
date_row["candidateCapabilities"] += statistics_capabilities
date_row["reason"] = "日期四项、矩阵七项和一维数值统计七项使用独立离线候选。独立版计算器范围尚未完整迁移。"
for action, scope in statistics_ops.items():
    date_row["operationScopes"].append(dict(
        operationId="devtools.statistics." + action, scopeKind="bounded_offline_real_statistics",
        scope=scope, sourceVersion=date_row["sourceVersion"], pluginStatus="candidate",
        **statistics_package,
        limits=dict(maxValues=64, inputUtf8Bytes=4096, valueUtf8Bytes=96,
                    minValues=2 if action.startswith("sample_") else 1,
                    requestBytes=16384, serializedResultBytes=1024),
        runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
        blockingReason="仅实数一维数列统计，变量、工作表和图表未迁移；Studio 安装及工具中心待验收。",
    ))

root_capability = "devtools.roots.bisect"
root_package = dict(
    packageId="com.zicode.devtools.root-solver", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/root-solver-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/root-solver-0.1.0-candidate-01-verification.json",
)
date_row["candidatePackages"].append(dict(**root_package, capabilities=[root_capability]))
date_row["candidateCapabilities"].append(root_capability)
date_row["reason"] = "日期四项、矩阵与数值统计各七项、区间二分求根作为独立离线候选。独立版计算器范围尚未完整迁移。"
date_row["operationScopes"].append(dict(
    operationId=root_capability, scopeKind="bounded_real_bisection",
    scope="在给定区间用原生二分法估算一个实函数的候选点，报告残差、迭代和收敛状态。",
    sourceVersion=date_row["sourceVersion"], pluginStatus="candidate", **root_package,
    limits=dict(formulaUtf8Bytes=2048, formulaTokens=512, expressionDepth=64, maxAbsEndpoint=1_000_000_000_000,
                minTolerance=1e-12, maxTolerance=1e-2, minIterations=1, maxIterations=200, explicitConstants=64,
                guestWorkUnits=22000, requestBytes=16384, serializedResultBytes=1024),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅用户提供区间的一个候选点。复杂度预算、连续性未证明、极点及偶重根限制均明示。变量会话、绘图、工作表和数值接力未迁移。",
))

formula_capability = "devtools.calculator.evaluate"
formula_package = dict(
    packageId="com.zicode.devtools.expression-calculator", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/expression-calculator-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/expression-calculator-0.1.0-candidate-01-verification.json",
)
date_row["candidatePackages"].append(dict(**formula_package, capabilities=[formula_capability]))
date_row["candidateCapabilities"].append(formula_capability)
date_row["reason"] = "日期、矩阵、数值统计、区间二分求根和单次公式计算已有独立离线候选。原生会话和工作表范围尚未完整迁移。"
date_row["operationScopes"].append(dict(
    operationId=formula_capability, scopeKind="bounded_stateless_numeric_expression",
    scope="使用原生表达式计算精确有理数、近似和复数、位运算、单位换算及排列组合。",
    sourceVersion=date_row["sourceVersion"], pluginStatus="candidate", **formula_package,
    limits=dict(formulaUtf8Bytes=2048, formulaTokens=512, expressionDepth=64, functionArguments=64,
                explicitConstants=64, constantUtf8Bytes=64, requestBytes=16384, serializedResultBytes=1024),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅一次无状态公式。变量和 ans 会话、绘图、工作表、图像或数值接力仍由独立版提供。",
))

agent_record_row = next(row for row in rows if row["sourceToolId"] == "agent-replay")
agent_record_capability = "devtools.agent-record.inspect"
agent_record_row.update(
    pluginStatus="candidate", candidateCapabilities=[agent_record_capability], scopeComplete=False,
    packageId="com.zicode.devtools.agent-record", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/agent-record-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/agent-record-0.1.0-candidate-01-verification.json",
    reason="共用独立版 Agent JSON 运行记录校验，提供一份经确认快照的只读元数据时间线。",
)
agent_record_row["operationScopes"].append(dict(
    operationId=agent_record_capability, scopeKind="bounded_agent_export_metadata_inspection",
    scope="检查导出记录 v1–v5，报告状态、步骤、耗时、响应字节、记录用量、来源和引用数量。隐藏文本、路径和任意名称。",
    sourceVersion=agent_record_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=65536, maxCalls=4, maxAllowlist=8, maxSourceDeclarations=40, serializedResultBytes=4096),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="仅结构校验。独立版 1 MiB、目录浏览、比较与原文展示尚未迁移。",
))

local_html_row = next(row for row in rows if row["sourceToolId"] == "browser-clipper")
local_html_capability = "devtools.html.local.preview"
local_html_row.update(
    pluginStatus="candidate", candidateCapabilities=[local_html_capability], scopeComplete=False,
    packageId="com.zicode.devtools.local-html", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/local-html-0.1.0-candidate-02",
    verificationReceipt="release/plugin-candidates/local-html-0.1.0-candidate-02-verification.json",
    reason="复用原生网页文字提取，提供逐次确认的一份本机 UTF-8 HTML 只读预览。",
)
local_html_row["operationScopes"].append(dict(
    operationId=local_html_capability, scopeKind="bounded_confirmed_local_html_text",
    scope="确认一份本机 UTF-8 HTML，查看标题、可提取正文的前 6 KiB 和省略提示。",
    sourceVersion=local_html_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=16384, previewTextBytes=6144, markupMarkers=256,
                characterReferenceMarkers=512, tagSeparators=128, documentSeparators=768,
                serializedResultBytes=16384),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="保留独立版 2 MiB 的导入、粘贴和公开 HTTPS 抓取、文本接力、知识源目录保存与去重。"
                   "本候选使用静态正文规则，单独验收 Windows 安装与资料生命周期。",
))

ics_count_row = next(row for row in rows if row["sourceToolId"] == "planner-interop")
ics_count_capability = "devtools.calendar.ics.count_cutoff"
ics_count_row.update(
    pluginStatus="candidate", candidateCapabilities=[ics_count_capability], scopeComplete=False,
    packageId="com.zicode.devtools.ics-count", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/ics-count-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/ics-count-0.1.0-candidate-01-verification.json",
    reason="复用独立版 COUNT 截止计算，逐次确认一份本机 ICS，显示末次开始和可用的 UNTIL。",
)
ics_count_row["operationScopes"].append(dict(
    operationId=ics_count_capability, scopeKind="bounded_confirmed_ics_count_cutoff",
    scope="单日程 ICS，支持全天和浮动整分钟 DTSTART。显示日、周、月、年规则的包含末次；COUNT=1 显示单次。",
    sourceVersion=ics_count_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=16384, maxPhysicalLines=512, maxEvents=1, maxRuleBytes=512,
                firstYear=1901, lastYear=2099, serializedResultBytes=2048),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="UTC、TZID、重复例外、超出原生支持的规则、多事件和写入或同步不在本候选范围。"
                   "单独验收 Windows 安装与资料生命周期。",
))

memo_text_row = next(row for row in rows if row["sourceToolId"] == "memo-library")
memo_text_capability = "devtools.memo.file.preview"
memo_text_row.update(
    pluginStatus="candidate", candidateCapabilities=[memo_text_capability], scopeComplete=False,
    packageId="com.zicode.devtools.memo-text", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/memo-text-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/memo-text-0.1.0-candidate-01-verification.json",
    reason="共享独立版逐条备忘文本准备，提供一份显式确认文件的只读导入预览。",
)
memo_text_row["operationScopes"].append(dict(
    operationId=memo_text_capability, scopeKind="bounded_confirmed_memo_file_preview",
    scope="从 Markdown/UTF-8 文本预览建议标题、准确字节范围和原样正文。保留 CRLF、制表和空格。",
    sourceVersion=memo_text_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(files=1, bodyUtf8Bytes=131072, extraBomBytes=3, previewBytes=6144,
                titleCharacters=120, serializedResultBytes=49152),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="源批量事务提交、标签、导出和知识接力继续由独立版提供。"
                   "本候选只提供单文件预览，Windows 安装与资料生命周期另行接收。",
))

prompt_template_row = next(row for row in rows if row["sourceToolId"] == "prompt-library")
prompt_operations = [
    ("devtools.prompt.variables", "prompt_variable_validation",
     "检查模板语法，列出已去重、排序的变量名，保持四花括号字面语义。"),
    ("devtools.prompt.expand", "single_pass_prompt_expansion_preview",
     "按原生规则展开一次。重复占位符共用同一变量文本，变量值中的花括号保留。"),
]
prompt_template_row.update(
    pluginStatus="candidate", candidateCapabilities=[operation[0] for operation in prompt_operations],
    scopeComplete=False, packageId="com.zicode.devtools.prompt-expansion", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/prompt-expansion-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/prompt-expansion-0.1.0-candidate-01-verification.json",
    reason="共享独立版提示词变量语法和一次性展开，提供两项无权限的本机预览。",
)
for operation, kind, scope in prompt_operations:
    prompt_template_row["operationScopes"].append(dict(
        operationId=operation, scopeKind=kind, scope=scope,
        sourceVersion=prompt_template_row["sourceVersion"], pluginStatus="candidate",
        limits=dict(templateUtf8Bytes=16384, variableNames=12, serializedRequestBytes=49152,
                    **({"expandedUtf8Bytes":131072,"previewUtf8Bytes":16384,"serializedResultBytes":49152}
                       if kind=="single_pass_prompt_expansion_preview" else {})),
        runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
        blockingReason="原生模板名、修订、标签、跨工具填充、模型配置与请求继续由独立版提供。"
                       "本候选的超长展开仅提供有界预览。",
    ))

tool_collections_row = next(row for row in rows if row["sourceToolId"] == "command-collections")
tool_collections_capability = "devtools.collections.preview"
tool_collections_row.update(
    pluginStatus="candidate", candidateCapabilities=[tool_collections_capability], scopeComplete=False,
    packageId="com.zicode.devtools.tool-collections", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/tool-collections-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/tool-collections-0.1.0-candidate-01-verification.json",
    reason="复用版本 1 的原生导入计划，只读查看集合名称、显式比较得到的改名与有序工具 ID。",
)
tool_collections_row["operationScopes"].append(dict(
    operationId=tool_collections_capability, scopeKind="bounded_confirmed_collection_import_plan",
    scope="校验本机版本 1 工具集合 JSON，保留集合和工具 ID 顺序，按明确填写的名称计算冲突改名。"
          "可按集合、工具起始序号查看后续项目，未知 ID 作为原文保留。",
    sourceVersion=tool_collections_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=131072, combinedCollections=128, toolsPerCollection=512,
                collectionPageSize=16, memberPageSize=16, memberPageEncodedBytes=4096,
                serializedResultBytes=24576),
    runtimeVerification="fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="只读版本 1，实际写入、现有偏好读取和跨机器同步保留给原生功能。"
                   "独立版 1 MiB，本候选验证 128 KiB。",
))

rag_eval_row = next(row for row in rows if row["sourceToolId"] == "rag-eval")
rag_eval_capability = "devtools.rag.report.review"
rag_eval_row.update(
    pluginStatus="candidate", candidateCapabilities=[rag_eval_capability], scopeComplete=False,
    packageId="com.zicode.devtools.retrieval-review", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/retrieval-review-0.1.0-candidate-01",
    verificationReceipt="release/plugin-candidates/retrieval-review-0.1.0-candidate-01-verification.json",
    reason="复用原生资料校验与命中计算，复核已确认的仅检索 v2 和双路对比 v1 导出。",
)
rag_eval_row["operationScopes"].append(dict(
    operationId=rag_eval_capability, scopeKind="bounded_confirmed_retrieval_report_review",
    scope="单路用有序证据重算首名与命中，并推导 MRR。双路用记录名次复算配对统计和 MRR。"
          "分母和失败处理按原生报告定义，逐题结果只输出序号和统计状态。",
    sourceVersion=rag_eval_row["sourceVersion"], pluginStatus="candidate",
    limits=dict(inputBytes=65536, cases=10, topK=[3, 5, 8], serializedResultBytes=8192),
    runtimeVerification="47_fixed_synthetic_native_windows_production_worker_parity",
    blockingReason="固定文件报告离线复核。原生数据集编写、索引检索、本机模型和报告导出留待其他范围，"
                   "需要目标 Studio 正常入口验收。",
))


for operation, source_id, title in trace_actions:
    row = next(row for row in rows if row["sourceToolId"] == source_id)
    assert row["pluginStatus"] == "candidate"
    row["candidateCapabilities"] = ["devtools.trace." + operation]
    row["reason"] = "完整文本堆栈操作共享核心，unsigned提供方候选已冻结；实际Host及Pi待验收。"
    row["operationScopes"] = [dict(operationId=operation, title=title,
        scopeKind="complete_trace_operation_within_plugin_budget", scope=next(t["scope"] for t in catalog["tools"] if t["id"] == source_id),
        sourceVersion=row["sourceVersion"], pluginStatus="candidate",
        core="zi-trace-core/0.1.0", adapter="zi-trace-wasi/0.1.0",
        packageId="com.zicode.devtools.trace", packageVersion="0.1.1",
        requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1","output.schema.v1":"=1.0.0-rc.1","tool.directory.v1":"=1.0.0-preview.1"},
        limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
        candidateDirectory="release/plugin-candidates/trace-0.1.1-candidate-01",
        verificationReceipt="release/plugin-candidates/trace-0.1.1-candidate-01-verification.json",
        moduleSha256="bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e",
        directoryIndexSha256="2e8b35249384757bc1c809bf971e2feccfc9396211a4efde7ca4f29fda9820c0",
        runtimeVerification="59_native_wasi_byte_parity_wasmtime_36_0_2_budget_and_fresh_process_cold_pass",
        viewVerification="public_sdk_fixture_final_wasi_2_actions_two_themes_not_Host_acceptance",
        blockingReason="unsigned提供方候选已冻结；实际Host安装/权限/生命周期/Worker/Pi待验收")]
json_row = next(row for row in rows if row["sourceToolId"] == "json")
json_ops = [
    ("format", "JSON 格式化", "格式化JSON"),
    ("minify", "JSON 压缩", "压缩JSON"),
    ("validate", "JSON 校验", "校验JSON"),
]
json_row["reason"] = "JSON桌面工具的格式化/压缩/校验三操作均映射为独立能力，共享严格兼容Rust核心；0.4.1候选含目录分类和中英文别名，官方Wasmtime燃料与冷启动通过，Studio Host/Pi仍待接收方验收。"
json_row["scopeComplete"] = True
json_row["packageId"] = "com.zicode.devtools.text"
json_row["packageVersion"] = "0.4.1"
json_row["candidateDirectory"] = "release/text-plugin-0.4.1-candidate-02"
json_row["verificationReceipt"] = "release/plugin-candidates/text-plugin-0.4.1-candidate-02-verification-revision-01.json"
json_row["directoryProfile"] = "tool.directory.v1@=1.0.0-preview.1"
json_row["operationScopes"] = [dict(
    operationId=f"devtools.text.json.{operation}", title=title,
    scopeKind="complete_standalone_json_action",
    scope=f"{scope}；单输入UTF-8文本，运行时限制8192字节、64层JSON、拒绝重复键/不安全整数。",
    sourceVersion=json_row["sourceVersion"], pluginStatus="candidate",
    core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
    packageId="com.zicode.devtools.text", packageVersion="0.4.1",
    hostProfile="compute.wasi.v1@=1.0.0-rc.1",
    directoryProfile="tool.directory.v1@=1.0.0-preview.1",
    limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
    candidateDirectory="release/text-plugin-0.4.1-candidate-02",
    verificationReceipt=json_row["verificationReceipt"],
    runtimeVerification="43_native_wasi_byte_parity_official_wasmtime_36_0_2_fuel_budget_and_fresh_process_cold_pass",
    viewVerification="public_sdk_shaped_bridge_actual_wasi_all_7_operations_two_themes_invalid_json_rejection",
    blockingReason="unsigned提供方候选；Studio真实安装/权限/PI/Host生命周期仍待验收")
    for operation, title, scope in json_ops]
json_object_capabilities = [
    ("devtools.json.object.inspect", "JSON 对象摘要", "structured_object_result"),
    ("devtools.json.object.receive", "接收 JSON 对象", "object_receiver_for_confirmed_transfer"),
]
json_row["candidateCapabilities"] += [capability for capability, _, _ in json_object_capabilities]
json_row["candidatePackages"] = [
    dict(packageId="com.zicode.devtools.text", packageVersion="0.4.1",
         candidateDirectory="release/text-plugin-0.4.1-candidate-02",
         verificationReceipt="release/plugin-candidates/text-plugin-0.4.1-candidate-02-verification-revision-01.json",
         capabilities=pilot["json"]),
    dict(packageId="com.zicode.devtools.json-object", packageVersion="0.1.1",
         candidateDirectory="release/plugin-candidates/json-object-0.1.1-candidate-03",
         verificationReceipt="release/plugin-candidates/json-object-0.1.1-candidate-03-verification.json",
         capabilities=[capability for capability, _, _ in json_object_capabilities]),
]
json_row["reason"] = "JSON 格式化/压缩/校验仍由 Text 0.4.1 候选提供；对象摘要另由 JSON Object 0.1.1 候选声明 result.json.v1 与 tool.directory.v1，对象接收能力用于显式接力。提供方字节一致验证已完成，Studio Host/Pi 和结果接力仍待接收方验收。"
for capability, title, scope_kind in json_object_capabilities:
    json_row["operationScopes"].append(dict(
        operationId=capability, title=title, scopeKind=scope_kind,
        scope="根值必须为 JSON object；输入对象最多 8 KiB、128 个根字段、64 层与 4096 节点，请求/结果最多 48 KiB。" if capability.endswith("inspect") else "仅作为已捕获 JSON object 的显式接收端；保持对象类型，受 8 KiB 输入、128 字段、48 KiB envelope 限制。",
        sourceVersion=json_row["sourceVersion"], pluginStatus="candidate",
        core="zi-json-core/0.1.0", adapter="zi-json-object-wasi/0.1.0",
        packageId="com.zicode.devtools.json-object", packageVersion="0.1.1",
        requiredProfiles={"output.schema.v1": "=1.0.0-rc.1", "result.json.v1": "=1.0.0-preview.1", "tool.directory.v1": "=1.0.0-preview.1"},
        limits=dict(inputObjectBytes=8192, requestBytes=49152, resultBytes=49152, rootFields=128, depth=64, nodes=4096),
        candidateDirectory="release/plugin-candidates/json-object-0.1.1-candidate-03",
        verificationReceipt="release/plugin-candidates/json-object-0.1.1-candidate-03-verification.json",
        runtimeVerification="10_baseline_native_wasi_parity_and_current_packaged_wasi_replay",
        viewVerification="no_plugin_view; standalone_JSON_object_summary_action_uses_same_native_adapter",
        blockingReason="unsigned provider candidate; Studio Host profile negotiation, real JSON capture/transfer/save and lifecycle remain unverified"))
for source_id, operations in {
    "base64": [
        ("devtools.text.base64.encode", "Base64 编码", "UTF-8文本编码为标准Base64。"),
        ("devtools.text.base64.decode", "Base64 解码", "标准Base64解码并验证UTF-8结果。"),
    ],
    "sha256": [("devtools.text.sha256", "SHA-256", "按原始UTF-8字节计算SHA-256。")],
}.items():
    tool_row = next(row for row in rows if row["sourceToolId"] == source_id)
    tool_row["reason"] = "文本插件0.4.1候选包含Base64/SHA；七项既有能力声明Studio目录分类、用途和中英文别名；43条native/WASI一致性、官方Wasmtime fuel/冷启动及两主题View已通过，Studio Host/Pi待验收。"
    tool_row["packageId"] = "com.zicode.devtools.text"
    tool_row["packageVersion"] = "0.4.1"
    tool_row["candidateDirectory"] = "release/text-plugin-0.4.1-candidate-02"
    tool_row["verificationReceipt"] = "release/plugin-candidates/text-plugin-0.4.1-candidate-02-verification-revision-01.json"
    tool_row["directoryProfile"] = "tool.directory.v1@=1.0.0-preview.1"
    tool_row["operationScopes"] = [dict(
        operationId=operation, title=title, scopeKind="complete_text_operation_within_plugin_budget",
        scope=scope + " 输入上限8192 UTF-8字节，序列化结果上限48 KiB。",
        sourceVersion=tool_row["sourceVersion"], pluginStatus="candidate",
        core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
        packageId="com.zicode.devtools.text", packageVersion="0.4.1",
        hostProfile="compute.wasi.v1@=1.0.0-rc.1",
        directoryProfile="tool.directory.v1@=1.0.0-preview.1",
        limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
        candidateDirectory="release/text-plugin-0.4.1-candidate-02",
        verificationReceipt=tool_row["verificationReceipt"],
        runtimeVerification="43_native_wasi_byte_parity_official_wasmtime_36_0_2_fuel_budget_and_fresh_process_cold_pass",
        viewVerification="public_sdk_shaped_bridge_actual_wasi_all_7_operations_two_themes_invalid_json_rejection",
        blockingReason="unsigned提供方候选；Base64/SHA View逐项检查及Studio Host/Pi仍待验收")
        for operation, title, scope in operations]
sql_row = next(row for row in rows if row["sourceToolId"] == "sql-format")
sql_row["reason"] = "共享 Rust 核心接入五种方言、缩进及关键字大小写，旧请求继续使用通用默认值。九组 8 KiB 密集短句保留原输出。十组原耗油的深层模板明确返回结果超限。保护字面量恢复和 Studio 正式安装继续验收。"
sql_row["scopeComplete"] = False
sql_row["packageId"] = "com.zicode.devtools.text"
sql_row["packageVersion"] = "0.4.4"
sql_row["candidateDirectory"] = "release/text-plugin-0.4.4-candidate-01"
sql_row["verificationReceipt"] = "release/text-plugin-0.4.4-candidate-01-verification.json"
sql_row["operationScopes"] = [dict(
    operationId="devtools.text.sql.format", title="SQL 格式化",
    scopeKind="bounded_sql_formatter_options",
    scope="通用、PostgreSQL、MySQL、SQLite、SQL Server，以及两空格、四空格、Tab、大写、小写和保留大小写。省略选项时沿用旧默认输出。",
    sourceVersion=sql_row["sourceVersion"], pluginStatus="candidate",
    core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
    packageId="com.zicode.devtools.text", packageVersion=sql_row["packageVersion"],
    hostProfile="compute.wasi.v1@=1.0.0-rc.1",
    directoryProfile="tool.directory.v1@=1.0.0-preview.1",
    limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152, fuel=10_000_000),
    candidateDirectory=sql_row["candidateDirectory"],
    verificationReceipt=sql_row["verificationReceipt"],
    runtimeVerification="76_frozen_byte_outputs_within_budget_and_15_windows_worker_output_limit_examples",
    viewVerification="public_sdk_shaped_bridge_actual_wasi_all_7_operations_with_generic_sql_defaults",
    blockingReason="实际恢复保护字面量的深层 SQL 仍可能达到预算。当前结果仅覆盖提供方候选，Studio 安装、文件导入下载与 Pi 权限由接收方验证。")]
# File actions extend the strict JSON subset without changing legacy inline packages.
json_file_capabilities = [
    ("devtools.json-file.format", "JSON 文件格式化"),
    ("devtools.json-file.minify", "JSON 文件压缩"),
    ("devtools.json-file.validate", "JSON 文件校验"),
]
json_row["candidateCapabilities"] += [operation for operation, _ in json_file_capabilities]
json_row["candidatePackages"].append(dict(packageId="com.zicode.devtools.json-file", packageVersion="0.1.0",
    candidateDirectory="release/plugin-candidates/json-file-0.1.0-candidate-02",
    verificationReceipt="release/plugin-candidates/json-file-0.1.0-candidate-02-verification.json",
    capabilities=[operation for operation, _ in json_file_capabilities]))
json_row["operationScopes"] += [dict(operationId=operation, title=title,
    scopeKind="resource_sized_strict_json_action", sourceVersion=json_row["sourceVersion"], pluginStatus="candidate",
    scope="UTF-8单文件1MiB/结果8MiB/64层；重复键、不安全整数及十进制精度损失明确作策略拒绝，与语法错误区分。原文件只读，保存需Host另行确认。",
    core="zi-text-core/0.1.0", adapter="zi-json-format-wasi/0.1.0", packageId="com.zicode.devtools.json-file", packageVersion="0.1.0",
    hostProfile="resource.wasi.json-file-transform.v1@=1.0.0-preview.1",
    limits=dict(inputUtf8Bytes=1048576, outputBytes=8388608, maxDepth=64),
    candidateDirectory="release/plugin-candidates/json-file-0.1.0-candidate-02",
    verificationReceipt="release/plugin-candidates/json-file-0.1.0-candidate-02-verification.json",
    blockingReason="独立无签名候选，真实Host/Worker证据单独交付；不推定日常安装已启用。")
    for operation, title in json_file_capabilities]

for row in rows:
    candidate_scopes = [scope for scope in row["operationScopes"] if scope.get("candidateDirectory")]
    directories = {scope["candidateDirectory"] for scope in candidate_scopes}
    packages = {(scope.get("packageId"), scope.get("packageVersion")) for scope in candidate_scopes
                if scope.get("packageId") and scope.get("packageVersion")}
    if len(directories) == len(packages) == 1:
        row.setdefault("candidateDirectory", next(iter(directories)))
        row.setdefault("verificationReceipt", next((scope.get("verificationReceipt") for scope in candidate_scopes
                                                     if scope.get("verificationReceipt")), None))
        row.setdefault("packageId", next(iter(packages))[0])
        row.setdefault("packageVersion", next(iter(packages))[1])
spring = next(row for row in rows if row["sourceToolId"] == "spring-actuator")
spring["reason"] = "0.1.3增加四项页面操作的搜索定位，保留原应用分类与用途；已有冻结HTTP View候选目录及外部校验收据，包含health/info/metrics与摘要4项页面操作；manifest未声明Agent可调用capability。任务中心、认证、context path、单项指标和报告导出未交付；真实Host端到端未验收，完整来源范围未交付。"
spring["candidateCapabilities"] = []
spring["candidateViewOperations"] = [
    "devtools.spring-actuator.health",
    "devtools.spring-actuator.info",
    "devtools.spring-actuator.metrics",
    "devtools.spring-actuator.summary",
]
spring["pilotCapabilities"] = []
spring["scopeComplete"] = False
spring["packageId"] = "com.zicode.devtools.spring-actuator"
spring["packageVersion"] = "0.1.3"
spring["sourceDirectory"] = "plugins/spring-actuator"
spring["candidateDirectory"] = "release/plugin-candidates/spring-actuator-0.1.3-candidate-01"
spring["reviewCandidateDirectory"] = "release/plugin-candidates/spring-actuator-0.1.3-candidate-01"
spring["verificationReceipt"] = "release/plugin-candidates/spring-actuator-0.1.3-candidate-01-verification.json"
spring["requiredProfile"] = {"service.http.get.v1": "=1.0.0-preview.1", "application.directory.v1": "=1.0.0-preview.1", "view.operation.directory.v1": "=1.0.0-preview.1"}
spring["optionalPermissions"] = ["service.http.read"]
spring["proposalProfiles"] = {
    "taskLifecycle": {
        "profile": "plugin.task.lifecycle.v1@1.0.0-preview.1",
        "manifestDeclared": False,
        "studioRuntime": "unimplemented_draft",
        "routes": [
            "POST /v2/plugin-tasks/submit",
            "POST /v2/plugin-tasks/snapshot",
            "POST /v2/plugin-tasks/events",
            "POST /v2/plugin-tasks/confirm",
            "POST /v2/plugin-tasks/cancel",
        ],
        "verification": "plugins/spring-actuator/scripts/verify-contract.py",
    },
    "resultResource": {
        "profile": "result.resource.create.v1@1.0.0-preview.1",
        "manifestDeclared": False,
        "studioRuntime": "unpublished_proposal",
        "verification": "plugins/spring-actuator/scripts/verify-contract.py",
    },
}
spring["operationScopes"] = [
    dict(operationId=f"devtools.spring-actuator.{endpoint}", title=f"Actuator {endpoint} JSON GET",
         scopeKind="fixed_host_http_get_candidate", endpoint=endpoint, sourceVersion=spring["sourceVersion"],
         pluginStatus="candidate", packageId=spring["packageId"], packageVersion=spring["packageVersion"],
         hostProfile="service.http.get.v1@=1.0.0-preview.1", inputSchema="plugins/spring-actuator/schemas/http-get-input.schema.json",
         resultSchema="plugins/spring-actuator/schemas/http-get-result.schema.json",
         verification="plugins/spring-actuator/scripts/test-view.cjs", blockingReason="真实Pi到Host到View、系统根证书和公网DNS尚未验收")
    for endpoint in ("health", "info", "metrics")
] + [
    dict(operationId="devtools.spring-actuator.summary", scopeKind="view_summary_candidate", sourceVersion=spring["sourceVersion"],
         pluginStatus="candidate", implementation="plugins/spring-actuator/views/main.html",
         resultCapture="Studio可信父页面/工作台接力；View不接触resultRef、不写路径"),
    dict(operationId="devtools.spring-actuator.background-task", scopeKind="durable_task_center", sourceVersion=spring["sourceVersion"],
         pluginStatus="not_delivered", proposal="plugins/spring-actuator/schemas/task-api-proposal",
         hostProfile="plugin.task.lifecycle.v1@1.0.0-preview.1_not_declared",
         blockingReason="Studio profile 仍为未实现草案；provider routes/schema/vectors 待Studio复核，不能写Super Agent记录"),
    dict(operationId="devtools.spring-actuator.report-export", scopeKind="provenance_json_resource_export", sourceVersion=spring["sourceVersion"],
         pluginStatus="not_delivered", proposal="plugins/spring-actuator/schemas/result-resource-proposal",
         blockingReason="Studio未发布经用户预览/确认的结果资源创建profile；不得自行写路径"),
]
for scope in spring["operationScopes"]:
    if scope["pluginStatus"] == "candidate":
        scope.update(
            packageId=spring["packageId"], packageVersion=spring["packageVersion"],
            candidateDirectory=spring["candidateDirectory"], verificationReceipt=spring["verificationReceipt"],
            executionSurface="view", viewId="devtools.spring-actuator.main",
            requiredProfiles=spring["requiredProfile"],
        )
for row in rows:
    if row["pluginStatus"] == "candidate":
        assert row.get("candidateDirectory") or row.get("reviewCandidateDirectory"), \
            "Candidate status must link to a concrete candidate/source directory: " + row["sourceToolId"]
        assert row.get("verificationReceipt"), \
            "Candidate status must link to its verification receipt: " + row["sourceToolId"]
for row in rows:
    if row["sourceToolId"] == "diff":
        assert row["pluginStatus"] == "not_delivered"
        row["reason"] = "1.0.3已通过263功能/时钟、258原预算、512混合文本对照、869原生回归与优化构建/隔离服务；258项独立冷启动全部通过（最长3277.577ms）；View/Host待验收，固定矩阵不证明全部合法输入性能，插件未交付。"
        for scope in row["operationScopes"]:
            scope["core"] = "zi-diff-core/0.1.0"
            scope["coreStatus"] = "native_core_verified_node_wasi_clock_and_full_input_limits_diagnostic_pass_Host_unnegotiated"
            scope["experimentalAdapter"] = "zi-compare-wasi/0.1.0"
            scope["adapterContract"] = "1.1.0-proposal.3_not_Host_negotiated"
            scope["adapterVerification"] = "1_0_3_263_functional_258_budget_512_mixed_parity_869_native_release_service_258_cold_pass_View_Host_pending"
            scope["proposal"] = "proposals/studio-fields/v0.1.2/README.md"
            scope["design"] = "docs/text-diff-plugin-design.md"
            scope["blockingReason"] = row["reason"]
    if row["sourceToolId"] in {"json-path", "json-diff"}:
        row["reason"] = "完整JSON核心已共享；字段实验适配220项native/WASI、原预算及冷启动本次全部通过（最高842.455ms）；更大和复杂正则预算仍需完整验证，Host尚未验收。"
        assert row["pluginStatus"] == "not_delivered"
        for scope in row["operationScopes"]:
            scope["core"] = "zi-json-core/0.1.0"
            scope["coreStatus"] = "standalone_shared_core_verified_experimental_fields_adapter_not_delivered"
            scope["experimentalAdapter"] = "zi-fields-wasi/0.1.0"
            scope["adapterContract"] = "1.1.0-proposal.2_not_Host_negotiated"
            scope["adapterVerification"] = "220_native_wasi_budget_pass_28_limit_matching_528_old_algorithm_parity_pass_cold_220_pass_max_843ms_historical_timeout_retained_full_regex_budget_unproven"
            scope["design"] = "docs/json-plugin-design.md"
            scope["blockingReason"] = "220项固定native/WASI功能、原预算和冷启动及28条编译/匹配对照通过，历史超时保留；更大和复杂正则预算未完整证明，缺少View、协议协商及Host验收。"
    if row["sourceToolId"] == "regex":
        row["reason"] = "正则工具1.0.9为单个Unicode标量重复增加轻量识别，避开HIR展开解析的fuel开销；265项native/WASI功能和260项Wasmtime 10M fuel矩阵通过，Host未验收，更大复杂语法预算仍待验证。"
        assert row["pluginStatus"] == "not_delivered"
        for scope in row["operationScopes"]:
            scope["core"] = "zi-regex-core/0.1.0"
            scope["coreStatus"] = "complete_standalone_core_verified_experimental_adapter_budget_incomplete"
            scope["experimentalAdapter"] = "zi-fields-wasi/0.1.0"
            scope["adapterContract"] = "1.1.0-proposal.2_not_Host_negotiated"
            scope["adapterVerification"] = "220_native_wasi_budget_pass_28_limit_matching_528_old_algorithm_parity_pass_cold_220_pass_max_843ms_historical_timeout_retained_full_regex_budget_unproven"
            scope["design"] = "docs/regex-plugin-design.md"
        scope["blockingReason"] = "a{300000}与中{100000}空文本用例已在原10M预算内通过（44,643与45,407 fuel）；更大重复与复杂正则仍需预算证明，并缺少View和真实Host验收。"
    if row["sourceToolId"] in {"ascii-codes", "symbol-library", "ascii-art"}:
        assert row["pluginStatus"] == "not_delivered"
        row["reason"] = "仅计算部分可提取；完整字符工具的主动复制、图片资源读取与TXT保存需分别协商Host授权，不用文字子集代替完整工具。"
        for scope in row["operationScopes"]:
            scope["design"] = "docs/character-plugin-design.md"
            scope["core"] = "zi-character-core/0.1.0"
            scope["coreStatus"] = "standalone_shared_core_native_tests_and_wasi_compile_pass_no_adapter"
            scope["blockingReason"] = row["reason"]
    if row["sourceToolId"] == "agent-runner":
        assert row["pluginStatus"] == "not_delivered"
        row["previewScopes"] = [dict(
            scopeId="model-assisted-view",
            scopeKind="single_turn_model_assistance_view",
            sourceToolVersion=row["sourceVersion"],
            packageId="com.zicode.devtools.agent-assist",
            packageVersion="0.1.3",
            candidateDirectory="release/plugin-candidates/agent-assist-preview-0.1.3-candidate-01",
            verificationFile="release/plugin-candidates/agent-assist-preview-0.1.3-candidate-01-verification.json",
            view="self_contained_html_in_srcdoc_no_external_assets",
            requiredProfiles={
                "model.catalog.read.v1": "1.0.0-preview.1",
                "model.generate.v1": "1.0.0-preview.1",
                "application.directory.v1": "1.0.0-preview.1",
                "view.operation.directory.v1": "1.0.0-preview.1",
            },
            requiredPermissions=["model.catalog.read", "model.generate"],
            validation="public_sdk_mock_sandbox_navigation_two_themes_and_original_model_result_regression",
            nativeStudioEndToEnd="pending",
            deliveryStatus="unsigned_candidate_not_Host_accepted",
            design="docs/agent-model-preview.md",
            excludes=["MCP", "RAG", "file_and_clipboard_input", "multi_step_agent", "tool_execution", "task_resume_and_cancellation"],
        )]
        preview = row["previewScopes"][0]
        row["candidateViewOperations"] = ["devtools.agent-assist.models", "devtools.agent-assist.generate"]
        row["operationScopes"].extend([
            dict(operationId=operation, scopeKind="single_turn_model_assistance_view", pluginStatus="candidate",
                 sourceVersion=row["sourceVersion"], packageId=preview["packageId"], packageVersion=preview["packageVersion"],
                 candidateDirectory=preview["candidateDirectory"], verificationReceipt=preview["verificationFile"],
                 executionSurface="view", viewId="devtools.agent-assist.main",
                 scope="用户主动读取模型目录或逐次确认后进行一轮文字生成；不包含MCP、RAG和自动执行。",
                 blockingReason="unsigned页面候选；真实Studio权限、模型请求、安装与完整Agent范围尚未验收")
            for operation in row["candidateViewOperations"]
        ])
image = next(row for row in rows if row["sourceToolId"] == "image-workflow")
image["candidateCapabilities"] = ["devtools.image.resize"]
image["scopeComplete"] = False
image["reason"] = "已有0.2.0单图缩放资源候选，使用当前0.12.0图片核心与整数参数适配器；当前独立版的完整图片工作流并未迁移。目录/MIME发现与参数schema已验证，真实Host生命周期、全尺寸/跨平台验收仍待完成。"
image["operationScopes"].append(dict(
    operationId="devtools.image.resize", title="缩放授权图片",
    scopeKind="single_image_resource_resize", sourceVersion=image["sourceVersion"], candidateSourceVersion="0.12.0",
    pluginStatus="candidate", packageId="com.zicode.devtools.image", packageVersion="0.2.0",
    candidateDirectory="release/plugin-candidates/image-resize-0.2.0-candidate-01",
    verificationReceipt="release/plugin-candidates/image-resize-0.2.0-candidate-01-verification.json",
    scope="运行模块来源0.12.0，仅单图PNG/JPEG/WebP缩放；宽度1–4096，JPEG质量35–95，PNG/WebP无损。不含批处理、裁剪、OCR和文件保存。",
    runtimeVerification="23_direct_wasi_numeric_legacy_format_parameter_and_zero_output_error_cases",
    blockingReason="unsigned候选；Host真实授权、资源登记、接力、取消及全尺寸/跨平台验收未由本轮证明。",
))
url = next(row for row in rows if row["sourceToolId"] == "url")
url["operationScopes"].append(dict(
    operationId="devtools.transforms.url.encode", title="URL 编码共享文本资源",
    scopeKind="explicit_text_resource_receiver", sourceVersion=url["sourceVersion"], pluginStatus="candidate",
    packageId="com.zicode.devtools.url-text-resource", packageVersion="0.1.1",
    candidateDirectory="release/plugin-candidates/url-text-resource-0.1.1-candidate-01",
    verificationReceipt="release/plugin-candidates/url-text-resource-0.1.1-candidate-01-verification.json",
    requiredProfiles={"resource.input.types.v1": "=1.0.0-preview.1", "output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": "=1.0.0-preview.1"},
    scope="单个用户授权的text/plain资源，1至8192 UTF-8字节；返回编码文本、源字节数或7种明确业务错误。",
    runtimeVerification="13_actual_wasi_schema_and_budget_cases_same_frozen_runtime",
    blockingReason="与transforms共享同一能力ID，隔离接收测试只启用一个提供方；真实Host资源授权、撤权、取消和跨工具接力尚未验收。",
))
result = dict(contract="zicode.devtools-plugin/1.0.0-rc.1", sourceCatalogVersion=catalog["version"],
              acceptedCapabilities=0,
              categories=dict(compute_candidate="可迁移候选", overlap_integration="重叠能力整合（未确认，不推定已有Host能力）",
                              await_host="等待Host接口", standalone_incomplete="独立版未完成"), tools=rows)
output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
path = ROOT / "docs/studio-plugin-migration.json"
if args.check:
    assert path.read_text(encoding="utf-8") == output, "Run scripts/sync_studio_plugin_map.py"
else:
    path.write_bytes(output.encode("utf-8"))
counts = {category: sum(row["migration"] == category for row in rows) for category in result["categories"]}
print(f"Plugin migration: {len(rows)} catalog items, {counts}; accepted capabilities=0")
