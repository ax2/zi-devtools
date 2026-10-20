"""Build an independent Inspect candidate with Studio's tool.open.v1 profile."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "release/plugin-candidates/inspect-0.1.1-candidate-02"
DEFAULT_OUTPUT = ROOT / "release/plugin-candidates/inspect-0.1.2-candidate-01"
CONTRACT = ROOT / "contracts/studio-tool-open/1.0.0-preview.1/README.md"
OPEN_PROFILE = "=1.0.0-preview.1"
OPEN_TOOL = "tool.open.v1"

parser = argparse.ArgumentParser()
parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
args = parser.parse_args()
output = args.output.resolve(strict=False)
release_root = (ROOT / "release/plugin-candidates").resolve(strict=True)
if not output.is_relative_to(release_root) or output == release_root:
    raise SystemExit("Candidate output must be a new child of release/plugin-candidates")
if output.exists():
    raise SystemExit(f"Candidate output already exists; refusing to overwrite: {output}")
if not BASE.is_dir() or not CONTRACT.is_file():
    raise SystemExit("Frozen Inspect candidate or vendored tool.open.v1 contract is missing")

old_manifest = json.loads((BASE / "plugin.json").read_text(encoding="utf-8"))
if old_manifest.get("version") != "0.1.1" or old_manifest.get("id") != "com.zicode.devtools.inspect":
    raise SystemExit("Frozen base candidate identity differs from the expected Inspect 0.1.1")
required = dict(old_manifest.get("requiredProfiles", {}))
if required.get("tool.directory.v1") != "=1.0.0-preview.1":
    raise SystemExit("Frozen Inspect directory profile is missing or changed")
if OPEN_TOOL in required:
    raise SystemExit("Base candidate already declares tool.open.v1")

shutil.copytree(BASE, output)
manifest_path = output / "plugin.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
manifest["version"] = "0.1.2"
manifest["requiredProfiles"] = {**required, OPEN_TOOL: OPEN_PROFILE}

view_path = output / "views/main.html"
view = view_path.read_text(encoding="utf-8")
assert view.count("async function run(){") == 1
assert "navigation.onOpenTool" not in view
view = view.replace(
    "async function run(){",
    """function openTool({capabilityId}={}){
  const action=actions.find(item=>'devtools.inspect.'+item.id===capabilityId);
  if(!action)return 'not_found';
  try{
    if(!$('operation')||!$('input')||!$('search'))return 'unsupported';
    $('search').value='';
    active=action.id;
    options();
    $('operation').value=action.id;
    render();
    $('operation').focus();
    $('input').focus();
    status(`已定位到「${action.title}」。请检查当前输入后主动处理文本。`);
    return 'opened';
  }catch{return 'unsupported'}
}
let openToolSubscription=null;
window.addEventListener('pagehide',()=>openToolSubscription?.dispose?.(),{once:true});

async function run(){""",
    1,
)
needle = "await sdk.ready();await sdk.context.get();ready=true;"
replacement = """await sdk.ready();
if(!sdk.navigation||typeof sdk.navigation.onOpenTool!=='function')throw new Error('tool.open.v1');
openToolSubscription=sdk.navigation.onOpenTool(openTool);
await sdk.context.get();ready=true;"""
assert view.count(needle) == 1
view = view.replace(needle, replacement, 1)
manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
view_path.write_text(view, encoding="utf-8", newline="\n")

catalog_path = output / "catalog.json"
catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
catalog["packageVersion"] = manifest["version"]
catalog["requiredProfiles"] = manifest["requiredProfiles"]
catalog_path.write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

contract_copy = output / "contracts/tool-open/README.md"
contract_copy.parent.mkdir(parents=True, exist_ok=True)
shutil.copyfile(CONTRACT, contract_copy)
vectors = {
    "profile": {"id": OPEN_TOOL, "version": OPEN_PROFILE},
    "view": {"id": "devtools.inspect.main", "location": "main"},
    "mapping": {
        capability["id"]: capability["id"].removeprefix("devtools.inspect.")
        for capability in manifest["contributes"]["capabilities"]
    },
    "cases": [
        {
            "id": "declared-color-converter",
            "capabilityId": "devtools.inspect.color.convert",
            "expected": "opened",
            "selectedAction": "color.convert",
            "capabilityInvocations": 0,
        },
        {
            "id": "unknown-capability",
            "capabilityId": "devtools.inspect.not-registered",
            "expected": "not_found",
            "selectedActionChanges": False,
            "capabilityInvocations": 0,
        },
        {
            "id": "slow-view-callback",
            "capabilityId": "devtools.inspect.color.convert",
            "delayMs": 4500,
            "viewBudgetMs": 4000,
            "expected": "unsupported",
            "lateAcknowledgementAccepted": False,
            "capabilityInvocations": 0,
        },
    ],
}
vectors_path = output / "verification/tool-open-vectors.json"
vectors_path.parent.mkdir(parents=True, exist_ok=True)
vectors_path.write_text(json.dumps(vectors, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
slow_sample = output / "verification/samples/slow-view-callback.js"
slow_sample.parent.mkdir(parents=True, exist_ok=True)
slow_sample.write_text(
    "// Receiver-boundary test only. This delayed callback is not used by the production View.\n"
    "async function slowViewCallback() {\n"
    "  await new Promise(resolve => setTimeout(resolve, 4500));\n"
    "  return 'opened';\n"
    "}\n"
    "let accepted = null, timedOut = false, lateAckIgnored = false;\n"
    "(async () => {\n"
    "  const result = await Promise.race([slowViewCallback().then(value => {\n"
    "    if (timedOut) lateAckIgnored = true; else accepted = value;\n"
    "    return value;\n"
    "  }), new Promise(resolve => setTimeout(() => resolve('unsupported'), 4000))]);\n"
    "  timedOut = result === 'unsupported';\n"
    "  await new Promise(resolve => setTimeout(resolve, 700));\n"
    "  if (result !== 'unsupported' || accepted !== null || !lateAckIgnored) process.exit(2);\n"
    "  console.log(JSON.stringify({status: result, lateAckIgnored, capabilityInvocations: 0, elapsedMs: 4500}));\n"
    "})().catch(() => process.exit(3));\n",
    encoding="utf-8",
    newline="\n",
)

readme = output / "README.md"
readme.write_text(
    "# ZiDevTools Inspect 0.1.2 candidate\n\n"
    "This independent unsigned candidate adds the exact Studio profile "
    "`tool.open.v1@=1.0.0-preview.1` to Inspect 0.1.1. Its single `location: main` View "
    "maps each of its eight declared capability IDs to the matching operation selector. "
    "The callback only selects/focuses the UI and returns `opened`, `not_found` or `unsupported`; "
    "it does not invoke capabilities or perform file/service/network work.\n\n"
    "The frozen 0.1.1 candidate is unchanged. Runtime WASM is byte-identical to that candidate; "
    "only the manifest, package metadata, View, contract snapshot and verification evidence differ. "
    "This is an unsigned provider candidate, not a Studio Host acceptance result.\n",
    encoding="utf-8",
)
(output / "MIGRATION.md").write_text(
    "# Migration 0.1.1 → 0.1.2\n\n"
    "This is a new package version. Candidate 0.1.1 remains frozen and usable on Hosts that do "
    "not implement `tool.open.v1`; 0.1.2 requires exactly `=1.0.0-preview.1` and must only be "
    "installed on a Host that advertises that profile. The compute module and eight capability "
    "contracts are unchanged. The added callback selects an operation UI only.\n",
    encoding="utf-8",
)

wasm = output / "runtime/tools.wasm"
wasm_sha = hashlib.sha256(wasm.read_bytes()).hexdigest()
source_paths = [
    ROOT / "scripts/package_inspect_tool_open_plugin.py",
    ROOT / "plugins/inspect/plugin.json",
    ROOT / "plugins/inspect/catalog.json",
    ROOT / "plugins/inspect/views/main.html",
    ROOT / "contracts/studio-tool-open/1.0.0-preview.1/README.md",
    BASE / "runtime/tools.wasm",
    BASE / "verification/view-proof.json",
]
provenance = {
    "packageId": manifest["id"],
    "packageVersion": manifest["version"],
    "status": "unsigned provider candidate; not Studio Host accepted",
    "requiredProfiles": manifest["requiredProfiles"],
    "baseCandidate": "release/plugin-candidates/inspect-0.1.1-candidate-02",
    "baseCandidateUnmodified": True,
    "runtimeModuleReused": True,
    "runtimeWasmSha256": wasm_sha,
    "sourceFiles": [
        {"path": path.relative_to(ROOT).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        for path in source_paths
    ],
    "limitations": [
        "The tool.open.v1 profile and receiver lifecycle are not accepted until Studio verifies this candidate.",
        "A location acknowledgement is not capability authorization or execution success.",
        "The slow callback is a separate receiver-boundary fixture, not production View code.",
    ],
}
(output / "provenance.json").write_text(json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"Built new candidate {output.relative_to(ROOT)}; WASM SHA-256 {wasm_sha}; base candidate untouched")
