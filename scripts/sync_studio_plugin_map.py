"""Keep all standalone catalog items mapped, without claiming plugin acceptance."""
from pathlib import Path
import argparse
import json

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--check", action="store_true")
args = parser.parse_args()
catalog = json.loads((ROOT / "docs/tools.json").read_text(encoding="utf-8"))
compute = set("json base64 url sha256 jwt regex number ascii-codes symbol-library ascii-art qr color text-stats html-escape text-escape case-convert yaml hex lines url-inspect cidr diff json-path json-diff data-schema data-transform csv-merge unicode java-trace django-trace java-thread-dump java-gc-log".split())
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
                     sourceStatus=tool["status"], migration=category, pluginStatus="candidate" if tool["id"] in pilot else "not_delivered",
                     pilotCapabilities=pilot.get(tool["id"], []), reason=reason))
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
