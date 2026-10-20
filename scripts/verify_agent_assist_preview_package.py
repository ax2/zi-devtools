"""Verify the installable directory layout and external proof for Agent Assist Preview."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / "release"
PACKAGE = RELEASE / "agent-assist-preview-0.1.1-candidate-02"
VERIFICATION = RELEASE / "agent-assist-preview-0.1.1-candidate-02-verification.json"
EXPECTED_ID = "com.zicode.devtools.agent-assist"
EXPECTED_VERSION = "0.1.1"
EXPECTED_PROFILES = {
    "model.catalog.read.v1": "=1.0.0-preview.1",
    "model.generate.v1": "=1.0.0-preview.1",
}
EXPECTED_PERMISSIONS = {"model.catalog.read", "model.generate"}
SOURCE_FILES = {
    "LICENSE": "LICENSE",
    "plugin.json": "plugins/agent-assist-preview/plugin.json",
    "catalog.json": "plugins/agent-assist-preview/catalog.json",
    "README.md": "plugins/agent-assist-preview/README.md",
    "views/main.html": "plugins/agent-assist-preview/views/main.html",
    "schemas/model-generation-input.schema.json": "plugins/agent-assist-preview/schemas/model-generation-input.schema.json",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


require(ROOT.is_dir() and not ROOT.is_symlink(), "Project root missing or linked")
require(RELEASE.is_dir() and not RELEASE.is_symlink(), "release/ missing or linked")
require(PACKAGE.is_dir() and not PACKAGE.is_symlink(), "Candidate directory missing or linked")
require(VERIFICATION.is_file() and not VERIFICATION.is_symlink(), "External verification file missing or linked")
release_root = RELEASE.resolve(strict=True)
for path in (PACKAGE, VERIFICATION):
    require(path.resolve(strict=True).is_relative_to(release_root), f"Path escapes release/: {path}")
require(not VERIFICATION.resolve(strict=True).is_relative_to(PACKAGE.resolve(strict=True)), "Proof must be external to package")

for entry in PACKAGE.rglob("*"):
    require(not entry.is_symlink(), f"Linked package entry: {entry.relative_to(PACKAGE)}")
actual_files = sorted(path.relative_to(PACKAGE).as_posix() for path in PACKAGE.rglob("*") if path.is_file())
require(actual_files == sorted(SOURCE_FILES), "Candidate inventory differs from source package manifest")

manifest_path = PACKAGE / "plugin.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
require(manifest.get("schemaVersion") == 2, "Manifest schemaVersion mismatch")
require(manifest.get("id") == EXPECTED_ID and manifest.get("version") == EXPECTED_VERSION, "Package identity mismatch")
require(manifest.get("requiredProfiles") == EXPECTED_PROFILES, "Required profiles mismatch")
permissions = {item.get("name") for item in manifest.get("permissions", {}).get("required", [])}
require(permissions == EXPECTED_PERMISSIONS, "Required permissions mismatch")
require(manifest.get("permissions", {}).get("optional", []) == [], "Unexpected optional permissions")
require(manifest.get("runtime") is None, "View-only preview must not declare a Worker runtime")
contributes = manifest.get("contributes", {})
require(all(contributes.get(name, []) == [] for name in ("capabilities", "commands", "piTools")), "Unexpected executable contribution")
require(manifest.get("activationEvents") == ["onView:devtools.agent-assist.main"], "Activation event mismatch")
views = contributes.get("views", [])
view_entries = [entry.get("entry") for entry in views if entry.get("location") == "main"]
require(view_entries == ["views/main.html"], "Expected exactly one main view at views/main.html")
html = (PACKAGE / view_entries[0]).read_text(encoding="utf-8")
require("<script src=" not in html.lower(), "View references an external script")
require("<link " not in html.lower(), "View references an external asset")
require("<form" not in html.lower() and re.search(r'<button id="generate"[^>]*type="button"', html) is not None,
        "Generation must use a sandbox-compatible button event, not form submission")
require("window.ZiAgentAssistLimits" in html, "Embedded byte-limit helper missing")
require("window.zicode?.services?.models" in html, "Public model SDK calls missing")
require("connect-src 'none'" not in html, "Plugin must not weaken the Host CSP")

proof = json.loads(VERIFICATION.read_text(encoding="utf-8"))
require(proof.get("schemaVersion") == 1 and proof.get("kind") == "unsigned-provider-candidate", "Verification kind/schema mismatch")
require(proof.get("packageId") == EXPECTED_ID and proof.get("packageVersion") == EXPECTED_VERSION, "Proof identity mismatch")
require(proof.get("candidateDirectory") == PACKAGE.relative_to(ROOT).as_posix(), "Proof package path mismatch")
require(isinstance(proof.get("sourceRevision"), str) and len(proof["sourceRevision"]) == 40, "Source revision missing")
require(isinstance(proof.get("sourceWorkingTreeDirty"), bool), "Source working-tree state must be disclosed")
require(all(proof.get("platform", {}).get(key) for key in ("os", "release", "architecture")), "Build platform details missing")
require(proof.get("signature", {}).get("signed") is False, "Candidate must remain clearly unsigned")
require(proof.get("signature", {}).get("publisherKeyAvailable") is False, "Publisher key availability must be disclosed")
acceptance = proof.get("acceptance", {})
require(acceptance.get("providerCandidate") is True, "Provider candidate state missing")
require(acceptance.get("studioReceiptVerified") is False, "Candidate must not claim Studio receipt verification")
require(acceptance.get("nativeStudioEndToEnd") == "pending", "Candidate must not claim native Studio acceptance")
require(proof.get("contract", {}).get("version") == "1.0.0-preview.1", "Contract version missing")
require(proof.get("profiles") == EXPECTED_PROFILES, "Proof profile pins mismatch")
require(set(proof.get("permissions", [])) == EXPECTED_PERMISSIONS, "Proof permissions mismatch")
tests = proof.get("tests", [])
require(tests and all(item.get("status") == "passed" for item in tests), "Candidate test evidence missing or failed")
view_proofs = proof.get("viewProofs", [])
require(len(view_proofs) == 2, "Expected desktop and mobile sandbox View proofs")
for view_proof, viewport in zip(view_proofs, ((1180, 900, False), (390, 844, True))):
    require(view_proof.get("pass") is True, "Sandbox View proof failed")
    require(view_proof.get("viewSha256") == sha256(PACKAGE / "views/main.html"), "Sandbox View proof is stale")
    require(view_proof.get("viewport") == {"width": viewport[0], "height": viewport[1],
            "devicePixelRatio": 1, "prefersDark": viewport[2]}, "Sandbox View viewport mismatch")
    require(view_proof.get("modelsReadByUserAction") == 1 and view_proof.get("generationRequests") == 3,
            "Sandbox View interaction proof incomplete")
    interactions = view_proof.get("resultInteractions", {})
    require(set(interactions) == {"modelSelectionPreserved", "inputsLockedWhilePending", "staleInputWarning",
            "rejectionPreservedPreviousAnswer", "resultJumpFocus", "selectionMatchesAnswer",
            "clearPreservesDraft", "noHorizontalOverflow"} and all(value is True for value in interactions.values()),
            "Result interaction evidence incomplete")
    require(view_proof.get("rejectionKeptResultHidden") and view_proof.get("approvalReturnedLocalFixture") and
            view_proof.get("userMaterialMarkedUntrusted"), "Sandbox View request boundary proof incomplete")
    require(view_proof.get("sandbox") == "allow-scripts; opaque origin; frameElement inaccessible",
            "Sandbox restrictions not demonstrated")
    require("connect-src 'none'" in view_proof.get("csp", ""), "CSP network restriction proof missing")
    require(not [path for path in view_proof.get("outerPageRequests", []) if path not in ("/", "/favicon.ico")],
            "Sandbox View attempted an unexpected network request")
    require(view_proof.get("runtimeExceptions") == [], "Sandbox View had runtime exceptions")
    screenshot = ROOT / "target" / "agent-assist-sandbox-011-c02" / view_proof.get("screenshot", "")
    require(screenshot.is_file() and sha256(screenshot) == view_proof.get("screenshotSha256"),
            "Sandbox View screenshot hash mismatch")
    result_screenshot = screenshot.parent / view_proof.get("resultScreenshot", "")
    require(result_screenshot.is_file() and sha256(result_screenshot) == view_proof.get("resultScreenshotSha256"),
            "Sandbox result screenshot hash mismatch")

entries = proof.get("files", [])
entry_paths = [entry.get("path") for entry in entries]
require(sorted(entry_paths) == actual_files and len(entry_paths) == len(set(entry_paths)), "External proof inventory does not match package contents")
for entry in entries:
    require(set(entry) == {"path", "size", "sha256"}, f"Unexpected proof file fields: {entry.get('path')}")
    packaged = PACKAGE / entry["path"]
    source = ROOT / SOURCE_FILES[entry["path"]]
    require(not source.is_symlink() and source.resolve(strict=True).is_relative_to(ROOT.resolve(strict=True)), f"Invalid source path: {entry['path']}")
    require(packaged.stat().st_size == entry.get("size"), f"Payload size mismatch: {entry['path']}")
    require(sha256(packaged) == entry.get("sha256"), f"Payload checksum mismatch: {entry['path']}")
    require(packaged.read_bytes() == source.read_bytes(), f"Frozen candidate differs from current source: {entry['path']}")
index_bytes = json.dumps(entries, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
require(hashlib.sha256(index_bytes).hexdigest() == proof.get("directoryIndexSha256"), "Directory index checksum mismatch")

snapshot = ROOT / "contracts" / "studio-model-generation-preview" / "1.0.0-preview.1"
contract_index_path = snapshot / "SHA256SUMS.json"
require(not contract_index_path.is_symlink(), "Contract checksum manifest is linked")
contract_index = json.loads(contract_index_path.read_text(encoding="utf-8"))
require(contract_index.get("version") == "1.0.0-preview.1", "Contract snapshot version mismatch")
require(proof.get("contract", {}).get("checksumManifestSha256") == sha256(contract_index_path), "Contract index checksum mismatch")
for entry in contract_index.get("files", []):
    contract_file = snapshot / entry["path"]
    require(not contract_file.is_symlink() and contract_file.resolve(strict=True).is_relative_to(snapshot.resolve(strict=True)), f"Invalid contract path: {entry['path']}")
    require(sha256(contract_file) == entry["sha256"], f"Contract checksum mismatch: {entry['path']}")

total_bytes = sum(entry["size"] for entry in entries)
require(len(entries) <= 2048 and total_bytes <= 50 * 1024 * 1024, "Studio package size/file count limit exceeded")
print(f"Verified Studio package root: {PACKAGE}")
print(f"Verified {len(entries)} files ({total_bytes:,} bytes); external proof, sizes, hashes, profiles, permissions, and contract hashes match")
print("Candidate remains unsigned; native Studio acceptance remains pending")
