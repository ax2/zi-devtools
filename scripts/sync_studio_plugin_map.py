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
                     sourceStatus=tool["status"], migration=category, pluginStatus="candidate" if tool["id"] in pilot or tool["id"] in report_scopes else "not_delivered",
                     candidateCapabilities=pilot.get(tool["id"], []) + (["devtools.diagnostics."+report_actions[tool["id"]]] if tool["id"] in report_scopes else []),
                     pilotCapabilities=pilot.get(tool["id"], []),
                     operationScopes=([dict(operationId=report_actions[tool["id"]], scopeKind="completed_imported_report_subset", scope=report_scopes[tool["id"]],
                         sourceVersion=tool["tool_version"], pluginStatus="candidate",
                         core="zi-diagnostics-core/0.1.1", adapter="zi-diagnostics-wasi/0.1.1",
                         runtimeVerification="native_wasi_byte_parity_and_independent_wasmtime_budget_fixtures",
                         viewVerification="public_sdk_fixture_actual_wasi_11_actions_two_viewports",
                         blockingReason="unsigned提供方候选目录；诊断schema/profile/错误码及完整Host生命周期待验收")]
                         if tool["id"] in report_scopes else [dict(scopeKind="declared_standalone_scope",
                             scope=tool["scope"], sourceStatus=tool["status"], sourceVersion=tool["tool_version"],
                             pluginStatus="candidate" if tool["id"] in pilot else "not_delivered",
                             blockingReason=reason)]), reason=reason))
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
