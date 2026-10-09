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
compute = set("json base64 url sha256 jwt regex number ascii-codes symbol-library ascii-art qr color text-stats html-escape text-escape case-convert yaml hex lines url-inspect cidr diff json-path json-diff data-schema data-transform csv-merge unicode java-trace django-trace java-thread-dump java-gc-log".split())
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
    "json": ["devtools.text.json.format", "devtools.text.json.minify"],
    "base64": ["devtools.text.base64.encode", "devtools.text.base64.decode"],
    "sha256": ["devtools.text.sha256"],
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
candidate_ids = {source for _, source, _ in trace_actions} | set(pilot) | set(report_scopes) | set(transform_actions) | set(inspect_actions)
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
                         core="zi-diagnostics-core/0.1.2", adapter="zi-diagnostics-wasi/0.1.2",
                         runtimeVerification="native_wasi_byte_parity_and_independent_wasmtime_budget_fixtures",
                         viewVerification="public_sdk_fixture_actual_wasi_11_actions_two_viewports",
                         blockingReason="unsigned提供方候选目录；诊断schema/profile/错误码及完整Host生命周期待验收")]
                         if tool["id"] in report_scopes else [dict(operationId=op, title=title,
                             scopeKind="complete_transform_operation_within_plugin_budget", scope=tool["scope"],
                             sourceVersion=tool["tool_version"], pluginStatus="candidate",
                             core="zi-text-core/0.1.0", adapter="zi-text-wasi/0.1.0",
                             packageId="com.zicode.devtools.transforms", packageVersion="0.1.0",
                             limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
                             runtimeVerification="174_independent_expected_vectors_native_wasi_parity_and_wasmtime_36_0_2_budget",
                             viewVerification="public_sdk_fixture_actual_wasi_23_actions_desktop_mobile_light_dark",
                             blockingReason="unsigned提供方候选；完整Host安装/权限/生命周期/Pi待验收")
                             for op, title in transform_actions[tool["id"]]] if tool["id"] in transform_actions else [dict(operationId=op, title=title,
                             scopeKind="complete_inspection_operation_within_plugin_budget", scope=tool["scope"],
                             sourceVersion=tool["tool_version"], pluginStatus="candidate",
                             core="zi-inspect-core/0.1.1", adapter="zi-inspect-wasi/0.1.1",
                             packageId="com.zicode.devtools.inspect", packageVersion="0.1.1",
                             limits=dict(inputUtf8Bytes=8192, serializedResultBytes=49152),
                             runtimeVerification="121_independent_vectors_native_wasi_parity_and_wasmtime_36_0_2_budget",
                             viewVerification="public_sdk_fixture_actual_wasi_8_actions_desktop_mobile_light_dark",
                             blockingReason="0.1.1提供方验证中，尚未冻结新版；冷启动复核及完整Host安装/权限/生命周期/Pi待验收")
                             for op, title in inspect_actions[tool["id"]]] if tool["id"] in inspect_actions else [dict(scopeKind="declared_standalone_scope",
                             scope=tool["scope"], sourceStatus=tool["status"], sourceVersion=tool["tool_version"],
                             pluginStatus="candidate" if tool["id"] in pilot else "not_delivered",
                             blockingReason=reason)]), reason=reason))
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
        moduleSha256="bb77ecac41351a9a8c3886dc0400c2705dd2003d3cfb52aa29e011d3ff43a35e",
        directoryIndexSha256="45dfdda2bb48f4878cc4166241db343bfc7a91c942e4fce3d7c6dd668b848ecd",
        runtimeVerification="59_native_wasi_byte_parity_wasmtime_36_0_2_budget_and_fresh_process_cold_pass",
        viewVerification="public_sdk_fixture_final_wasi_2_actions_two_themes_not_Host_acceptance",
        blockingReason="unsigned提供方候选已冻结；实际Host安装/权限/生命周期/Worker/Pi待验收")]
for row in rows:
    if row["sourceToolId"] in {"json-path", "json-diff"}:
        assert row["pluginStatus"] == "not_delivered"
        for scope in row["operationScopes"]:
            scope["core"] = "zi-json-core/0.1.0"
            scope["coreStatus"] = "standalone_shared_core_verified_experimental_fields_adapter_not_delivered"
            scope["experimentalAdapter"] = "zi-fields-wasi/0.1.0"
            scope["adapterContract"] = "1.1.0-proposal.2_not_Host_negotiated"
            scope["adapterVerification"] = "147_native_wasi_byte_and_schema_vectors_budget_cold_pass_not_complete_adversarial_or_Host_proof"
            scope["design"] = "docs/json-plugin-design.md"
            scope["blockingReason"] = "三项JSON实验适配已验证147条；需完整恶意向量、View、协议协商及真实Host验收，不是五操作提案全实现或插件交付。"
    if row["sourceToolId"] in {"ascii-codes", "symbol-library", "ascii-art"}:
        assert row["pluginStatus"] == "not_delivered"
        row["reason"] = "仅计算部分可提取；完整字符工具的主动复制、图片资源读取与TXT保存需分别协商Host授权，不用文字子集代替完整工具。"
        for scope in row["operationScopes"]:
            scope["design"] = "docs/character-plugin-design.md"
            scope["core"] = "zi-character-core/0.1.0"
            scope["coreStatus"] = "standalone_shared_core_native_tests_and_wasi_compile_pass_no_adapter"
            scope["blockingReason"] = row["reason"]
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
