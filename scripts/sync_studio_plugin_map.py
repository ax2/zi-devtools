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
                         core="zi-diagnostics-core/0.1.3", adapter="zi-diagnostics-wasi/0.1.3",
                         packageId="com.zicode.devtools.diagnostics", packageVersion="0.1.3",
                         hostProfile="compute.wasi.v1@=1.0.0-rc.1", diagnosticsProfile="diagnostics.compute.wasi.v1.proposal",
                         diagnosticsSchemaVersion="2.0.0-rc.1", requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1","output.schema.v1":"=1.0.0-rc.1"},
                         candidateDirectory="release/plugin-candidates/diagnostics-0.1.3-candidate-01",
                         verificationReceipt="release/plugin-candidates/diagnostics-0.1.3-candidate-01-verification.json",
                         runtimeVerification="native_wasi_byte_parity_and_independent_wasmtime_budget_fixtures",
                         viewVerification="public_sdk_fixture_actual_wasi_11_actions_two_viewports",
                         blockingReason="unsigned提供方候选目录；诊断schema/profile/错误码及完整Host生命周期待验收")]
                         if tool["id"] in report_scopes else [dict(operationId=op, title=title,
                             scopeKind="complete_transform_operation_within_plugin_budget", scope=tool["scope"],
                             sourceVersion=tool["tool_version"], pluginStatus="candidate",
                             core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
                             packageId="com.zicode.devtools.transforms", packageVersion="0.1.0",
                             limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
                             candidateDirectory="release/transforms-0.1.0-candidate-02",
                             verificationReceipt="release/transforms-0.1.0-candidate-02-verification.json",
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
                             verificationReceipt="release/plugin-candidates/inspect-0.1.2-candidate-01-verification.json",
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
        row["verificationReceipt"] = "release/plugin-candidates/inspect-0.1.2-candidate-01-verification.json"
for operation, source_id, title in trace_actions:
    row = next(row for row in rows if row["sourceToolId"] == source_id)
    assert row["pluginStatus"] == "candidate"
    row["candidateCapabilities"] = ["devtools.trace." + operation]
    row["reason"] = "完整文本堆栈操作共享核心，unsigned提供方候选已冻结；实际Host及Pi待验收。"
    row["operationScopes"] = [dict(operationId=operation, title=title,
        scopeKind="complete_trace_operation_within_plugin_budget", scope=next(t["scope"] for t in catalog["tools"] if t["id"] == source_id),
        sourceVersion=row["sourceVersion"], pluginStatus="candidate",
        core="zi-trace-core/0.1.0", adapter="zi-trace-wasi/0.1.0",
        packageId="com.zicode.devtools.trace", packageVersion="0.1.0",
        limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
        candidateDirectory="release/trace-0.1.0-candidate-01",
        verificationReceipt="release/trace-0.1.0-candidate-01-verification.json",
        moduleSha256="bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e",
        directoryIndexSha256="45dfdda2bb48f4878cc4166241db343bfc7a91c942e4fce3d7c6dd668b848ecd",
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
json_row["verificationReceipt"] = "release/text-plugin-0.4.1-candidate-02-verification.json"
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
         verificationReceipt="release/text-plugin-0.4.1-candidate-02-verification.json",
         capabilities=pilot["json"]),
    dict(packageId="com.zicode.devtools.json-object", packageVersion="0.1.0",
         candidateDirectory="release/plugin-candidates/json-object-0.1.0-candidate-07",
         verificationReceipt="release/plugin-candidates/json-object-0.1.0-candidate-07-verification-revision-01.json",
         capabilities=[capability for capability, _, _ in json_object_capabilities]),
]
json_row["reason"] = "JSON 格式化/压缩/校验仍由 Text 0.4.1 候选提供；对象摘要另由 JSON Object 0.1.0 候选声明 result.json.v1，对象接收能力用于显式接力。提供方字节一致验证已完成，Studio Host/Pi 和结果接力仍待接收方验收。"
for capability, title, scope_kind in json_object_capabilities:
    json_row["operationScopes"].append(dict(
        operationId=capability, title=title, scopeKind=scope_kind,
        scope="根值必须为 JSON object；输入对象最多 8 KiB、128 个根字段、64 层与 4096 节点，请求/结果最多 48 KiB。" if capability.endswith("inspect") else "仅作为已捕获 JSON object 的显式接收端；保持对象类型，受 8 KiB 输入、128 字段、48 KiB envelope 限制。",
        sourceVersion=json_row["sourceVersion"], pluginStatus="candidate",
        core="zi-json-core/0.1.0", adapter="zi-json-object-wasi/0.1.0",
        packageId="com.zicode.devtools.json-object", packageVersion="0.1.0",
        requiredProfiles={"output.schema.v1": "=1.0.0-rc.1", "result.json.v1": "=1.0.0-preview.1"},
        limits=dict(inputObjectBytes=8192, requestBytes=49152, resultBytes=49152, rootFields=128, depth=64, nodes=4096),
        candidateDirectory="release/plugin-candidates/json-object-0.1.0-candidate-07",
        verificationReceipt="release/plugin-candidates/json-object-0.1.0-candidate-07-verification-revision-01.json",
        runtimeVerification="10_native_wasi_byte_parity_cases_duplicate_key_and_size_rejection",
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
    tool_row["verificationReceipt"] = "release/text-plugin-0.4.1-candidate-02-verification.json"
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
        runtimeVerification="43_native_wasi_byte_parity_official_wasmtime_36_0_2_fuel_budget_and_fresh_process_cold_pass",
        viewVerification="public_sdk_shaped_bridge_actual_wasi_all_7_operations_two_themes_invalid_json_rejection",
        blockingReason="unsigned提供方候选；Base64/SHA View逐项检查及Studio Host/Pi仍待验收")
        for operation, title, scope in operations]
sql_row = next(row for row in rows if row["sourceToolId"] == "sql-format")
sql_row["reason"] = "共享纯Rust SQL格式化核心新增WASI通用方言操作；仅覆盖格式化单操作，不含桌面方言/缩进/大小写选择，Studio Host/Pi仍待接收方验收。"
sql_row["scopeComplete"] = False
sql_row["packageId"] = "com.zicode.devtools.text"
sql_row["packageVersion"] = "0.4.1"
sql_row["candidateDirectory"] = "release/text-plugin-0.4.1-candidate-02"
sql_row["verificationReceipt"] = "release/text-plugin-0.4.1-candidate-02-verification.json"
sql_row["operationScopes"] = [dict(
    operationId="devtools.text.sql.format", title="SQL 格式化 · 通用方言",
    scopeKind="partial_standalone_formatter", scope="通用SQL词法模式和默认缩进/关键字大小写；不提供桌面工具的PostgreSQL/MySQL/SQLite/SQL Server方言选择。",
    sourceVersion=sql_row["sourceVersion"], pluginStatus="candidate",
    core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
    packageId="com.zicode.devtools.text", packageVersion="0.4.1",
    hostProfile="compute.wasi.v1@=1.0.0-rc.1",
    directoryProfile="tool.directory.v1@=1.0.0-preview.1",
    limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
    candidateDirectory="release/text-plugin-0.4.1-candidate-02",
    runtimeVerification="43_native_wasi_byte_parity_official_wasmtime_36_0_2_fuel_budget_and_fresh_process_cold_pass",
    viewVerification="public_sdk_shaped_bridge_actual_wasi_all_7_operations_two_themes_invalid_json_rejection",
    blockingReason="unsigned且不完整提供方候选；SQL桌面方言选项、Studio安装/权限/PI/Host生命周期仍待验收")]
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
spring["reason"] = "0.1.1仅有固定health/info/metrics HTTP View候选；Studio任务生命周期profile仍是未实现草案，provider schema/vectors待Studio复核；报告资源profile亦未发布。无正式包/收据、真实Pi到Host到View未验收，完整来源范围未交付。"
spring["candidateCapabilities"] = [
    "devtools.spring-actuator.health",
    "devtools.spring-actuator.info",
    "devtools.spring-actuator.metrics",
    "devtools.spring-actuator.summary",
]
spring["pilotCapabilities"] = []
spring["scopeComplete"] = False
spring["packageId"] = "com.zicode.devtools.spring-actuator"
spring["packageVersion"] = "0.1.1"
spring["candidateDirectory"] = "plugins/spring-actuator"
spring["reviewCandidateDirectory"] = "release/plugin-candidates/spring-actuator-0.1.1-candidate-02"
spring["verificationReceipt"] = "release/plugin-candidates/spring-actuator-0.1.1-candidate-02-verification.json"
spring["requiredProfile"] = {"service.http.get.v1": "=1.0.0-preview.1"}
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
            packageVersion="0.1.0",
            candidateDirectory="release/agent-assist-preview-0.1.0-candidate-03",
            verificationFile="release/agent-assist-preview-0.1.0-candidate-03-verification.json",
            view="self_contained_html_in_srcdoc_no_external_assets",
            requiredProfiles={
                "model.catalog.read.v1": "1.0.0-preview.1",
                "model.generate.v1": "1.0.0-preview.1",
            },
            requiredPermissions=["model.catalog.read", "model.generate"],
            validation="three_utf8_limit_tests_jsdom_csp_meta_mock_host_and_installable_root_external_proof_pass",
            nativeStudioEndToEnd="pending",
            deliveryStatus="unsigned_candidate_not_Host_accepted",
            design="docs/agent-model-preview.md",
            excludes=["MCP", "RAG", "file_and_clipboard_input", "multi_step_agent", "tool_execution", "task_resume_and_cancellation"],
        )]
result = dict(contract="zicode.devtools-plugin/1.0.0-rc.1", sourceCatalogVersion=catalog["version"],
              acceptedCapabilities=0,
              categories=dict(compute_candidate="可迁移候选", overlap_integration="重叠能力整合（未确认，不推定已有Host能力）",
                              await_host="等待Host接口", standalone_incomplete="独立版未完成"), tools=rows)
output = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
path = ROOT / "docs/studio-plugin-migration.json"
if args.check:
    assert path.read_text(encoding="utf-8") == output, "Run scripts/sync_studio_plugin_map.py"
else:
    path.write_text(output, encoding="utf-8")
counts = {category: sum(row["migration"] == category for row in rows) for category in result["categories"]}
print(f"Plugin migration: {len(rows)} catalog items, {counts}; accepted capabilities=0")
