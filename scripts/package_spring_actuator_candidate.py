"""Create an unsigned, hash-bound Spring Actuator plugin review directory."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "plugins/spring-actuator"
OUTPUT_ROOT = ROOT / "release/plugin-candidates"
NAME = "spring-actuator-0.1.1-candidate-02"
DEST = OUTPUT_ROOT / NAME
RECEIPT = OUTPUT_ROOT / f"{NAME}-verification.json"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)


def run(label: str, command: list[str]) -> str:
    env = os.environ.copy()
    deps = ROOT / "target/plugin-verification-deps"
    env["PYTHONPATH"] = os.pathsep.join(filter(None, [str(deps), env.get("PYTHONPATH", "")]))
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True)
    output = (result.stdout + result.stderr).strip()
    if result.returncode:
        raise RuntimeError(f"{label} failed ({result.returncode}):\n{output}")
    return output


def git_text(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True, encoding="utf-8").strip()


def git_bytes(*args: str) -> bytes:
    return subprocess.check_output(["git", *args], cwd=ROOT)


def main() -> None:
    if DEST.exists() or RECEIPT.exists():
        raise SystemExit(f"Refusing to overwrite candidate or receipt: {DEST} / {RECEIPT}")
    manifest = load_json(SOURCE / "plugin.json")
    catalog = load_json(SOURCE / "catalog.json")
    assert manifest["id"] == "com.zicode.devtools.spring-actuator"
    assert manifest["version"] == "0.1.1"
    assert manifest["requiredProfiles"] == {"service.http.get.v1": "=1.0.0-preview.1"}
    assert catalog["sourceTool"]["id"] == "spring-actuator"
    assert catalog["sourceTool"]["version"] == "1.1.1"
    assert catalog["sourceTool"]["scopeComplete"] is False
    assert catalog["delivery"]["fullSourceToolDelivered"] is False
    assert not any("plugin.task.lifecycle" in key for key in manifest["requiredProfiles"])

    verification = {
        "view": run("View harness", ["node", "plugins/spring-actuator/scripts/test-view.cjs"]),
        "contracts": run("Provider contract verifier", [sys.executable, "plugins/spring-actuator/scripts/verify-contract.py"]),
        "toolCatalogSync": run("Tool catalog sync", [sys.executable, "scripts/sync_tools.py", "--check"]),
        "migrationSync": run("Studio map sync", [sys.executable, "scripts/sync_studio_plugin_map.py", "--check"]),
    }
    for path in SOURCE.rglob("*.json"):
        load_json(path)

    OUTPUT_ROOT.mkdir(parents=True, exist_ok=True)
    shutil.copytree(SOURCE, DEST, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    shutil.copyfile(ROOT / "LICENSE", DEST / "LICENSE")
    assert not list(DEST.rglob("__pycache__")) and not list(DEST.rglob("*.pyc")), "bytecode cache must not enter the candidate"
    copied = run("Candidate contract verifier", [sys.executable, str(DEST / "scripts/verify-contract.py")])
    copied_view = run("Candidate View harness", ["node", str(DEST / "scripts/test-view.cjs")])
    verification["candidateContracts"] = copied
    verification["candidateView"] = copied_view

    file_rows = []
    for path in sorted(path for path in DEST.rglob("*") if path.is_file()):
        blob = path.read_bytes()
        file_rows.append({"path": path.relative_to(DEST).as_posix(), "bytes": len(blob), "sha256": digest(blob)})
    index_bytes = json.dumps(file_rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8")

    revision = git_text("rev-parse", "HEAD")
    dirty = bool(git_text("status", "--porcelain"))
    base_catalog = json.loads(git_bytes("show", "HEAD:docs/tools.json"))
    current_catalog = load_json(ROOT / "docs/tools.json")
    fields = ("name", "tool_version", "status", "scope")
    old_tools = {row["id"]: row for row in base_catalog["tools"]}
    new_tools = {row["id"]: row for row in current_catalog["tools"]}
    catalog_changes = []
    for tool_id in sorted(old_tools.keys() | new_tools.keys()):
        old = old_tools.get(tool_id)
        new = new_tools.get(tool_id)
        changed = [field for field in fields if (old or {}).get(field) != (new or {}).get(field)]
        if changed:
            catalog_changes.append({
                "id": tool_id,
                "fields": changed,
                "before": {field: old.get(field) for field in changed} if old else None,
                "after": {field: new.get(field) for field in changed} if new else None,
            })

    receipt = {
        "candidate": DEST.relative_to(ROOT).as_posix(),
        "status": "unsigned provider review directory; not an installable .zicode-plugin and not Studio accepted",
        "pluginId": manifest["id"],
        "pluginVersion": manifest["version"],
        "sourceTool": {"id": catalog["sourceTool"]["id"], "version": catalog["sourceTool"]["version"]},
        "sourceRevision": revision,
        "sourceTreeDirty": dirty,
        "taskProfileDeclared": False,
        "requiredProfiles": manifest["requiredProfiles"],
        "signing": {"signed": False, "authorizedSigningKeyProvided": False, "packageSha256": None},
        "verification": verification,
        "sourceCatalog": {
            "baselineRevision": revision,
            "baselineSha256": digest(git_bytes("show", "HEAD:docs/tools.json")),
            "currentSha256": digest((ROOT / "docs/tools.json").read_bytes()),
            "changedToolsAgainstHead": catalog_changes,
        },
        "migrationMap": {"sha256": digest((ROOT / "docs/studio-plugin-migration.json").read_bytes()), "acceptedCapabilities": 0},
        "files": file_rows,
        "directoryIndexSha256": digest(index_bytes),
        "packageSha256": None,
        "limitations": [
            "Studio task lifecycle and result/resource profiles are unimplemented/unpublished drafts.",
            "HTTP Host profile and Pi-to-Host-to-View behavior have not been accepted or tested end-to-end.",
            "No signing key was provided; no signed container was created.",
            "Full standalone source scope, including background task center and report export, is not delivered.",
        ],
    }
    RECEIPT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Created unsigned candidate directory: {DEST}")
    print(f"Created external verification receipt: {RECEIPT}")
    print(f"Files: {len(file_rows)}; bytes: {sum(row['bytes'] for row in file_rows)}; catalog changes: {len(catalog_changes)}")


if __name__ == "__main__":
    main()
