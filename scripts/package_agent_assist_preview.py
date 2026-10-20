"""Create a small unsigned, signable directory candidate for the Agent Assist View."""
from __future__ import annotations

from datetime import datetime
import hashlib
import json
from pathlib import Path
import argparse
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / "release"
DEST = RELEASE / "agent-assist-preview-0.1.1-candidate-02"
VERIFY = RELEASE / "agent-assist-preview-0.1.1-candidate-02-verification.json"
PLUGIN = ROOT / "plugins" / "agent-assist-preview"
SNAPSHOT = ROOT / "contracts" / "studio-model-generation-preview" / "1.0.0-preview.1"
PAYLOAD = {
    "LICENSE": "LICENSE",
    "plugin.json": "plugins/agent-assist-preview/plugin.json",
    "catalog.json": "plugins/agent-assist-preview/catalog.json",
    "README.md": "plugins/agent-assist-preview/README.md",
    "views/main.html": "plugins/agent-assist-preview/views/main.html",
    "schemas/model-generation-input.schema.json": "plugins/agent-assist-preview/schemas/model-generation-input.schema.json",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def assert_no_links(path: Path) -> None:
    absolute = Path(path.absolute())
    for item in (absolute, *absolute.parents):
        if item.exists() and item.is_symlink():
            raise RuntimeError(f"Refusing linked path: {item}")


def under_root(path: Path, root: Path) -> bool:
    return path == root or root in path.parents


def collect_tests() -> list[dict[str, str]]:
    proof_root = ROOT / "target" / "agent-assist-sandbox-011-c02"
    checks = [
        ["node", "--test", "plugins/agent-assist-preview/tests/limits.test.cjs"],
        [sys.executable, "scripts/sync_studio_plugin_map.py", "--check"],
        [sys.executable, "scripts/verify_agent_assist_preview_view.py", "--width", "1180", "--height", "900",
         "--theme", "light", "--output", str(proof_root / "desktop-proof.json"), "--screenshot", str(proof_root / "view-desktop.png")],
        [sys.executable, "scripts/verify_agent_assist_preview_view.py", "--width", "390", "--height", "844",
         "--theme", "dark", "--output", str(proof_root / "mobile-proof.json"), "--screenshot", str(proof_root / "view-mobile.png")],
    ]
    results = []
    for command in checks:
        run = subprocess.run(
            command,
            cwd=ROOT,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        if run.returncode:
            raise RuntimeError(f"Candidate validation failed: {' '.join(command)}\n{run.stdout}\n{run.stderr}")
        lines = [line.strip() for line in run.stdout.splitlines() if line.strip()]
        results.append({
            "command": " ".join(command),
            "status": "passed",
            "summary": " | ".join(lines[-4:]),
        })
    return results


def create_verification() -> tuple[list[dict[str, object]], dict[str, object]]:
    if not DEST.is_dir() or DEST.is_symlink():
        raise RuntimeError(f"Candidate directory missing or linked: {DEST}")
    if VERIFY.is_symlink():
        raise RuntimeError(f"Refusing linked verification file: {VERIFY}")
    if not VERIFY.parent.resolve(strict=True).is_relative_to(RELEASE.resolve(strict=True)):
        raise RuntimeError("External proof path escapes release directory")
    if VERIFY.resolve().is_relative_to(DEST.resolve()):
        raise RuntimeError("Verification must be outside the plugin package")
    for path in (DEST, *DEST.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Linked package entry: {path}")

    expected = set(PAYLOAD)
    actual_paths = {path.relative_to(DEST).as_posix() for path in DEST.rglob("*") if path.is_file()}
    if actual_paths != expected:
        raise RuntimeError(f"Candidate file inventory differs from manifest: {sorted(actual_paths ^ expected)}")
    file_entries: list[dict[str, object]] = []
    for package_path, source_path in PAYLOAD.items():
        packaged = DEST / package_path
        source = safe_source(source_path)
        if packaged.read_bytes() != source.read_bytes():
            raise RuntimeError(f"Frozen candidate differs from current source: {package_path}")
        file_entries.append({"path": package_path, "size": packaged.stat().st_size, "sha256": sha256(packaged)})
    file_entries.sort(key=lambda item: str(item["path"]))
    index_bytes = json.dumps(file_entries, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    revision = subprocess.check_output(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True, stderr=subprocess.DEVNULL
    ).strip()
    dirty = bool(subprocess.check_output(
        ["git", "-C", str(ROOT), "status", "--porcelain"], text=True, stderr=subprocess.DEVNULL
    ).strip())
    contract_index = safe_source("contracts/studio-model-generation-preview/1.0.0-preview.1/SHA256SUMS.json")
    contract_data = json.loads(contract_index.read_text(encoding="utf-8"))
    tests = collect_tests()
    view_proofs = []
    for proof_path, expected in [
        (ROOT / "target/agent-assist-sandbox-011-c02/desktop-proof.json", (1180, 900, False)),
        (ROOT / "target/agent-assist-sandbox-011-c02/mobile-proof.json", (390, 844, True)),
    ]:
        view_proof = json.loads(proof_path.read_text(encoding="utf-8"))
        width, height, dark = expected
        if not view_proof.get("pass") or view_proof.get("viewSha256") != sha256(PLUGIN / "views/main.html"):
            raise RuntimeError(f"Agent View proof is stale or failed: {proof_path}")
        if view_proof.get("viewport") != {"width": width, "height": height, "devicePixelRatio": 1, "prefersDark": dark}:
            raise RuntimeError(f"Unexpected Agent View viewport: {proof_path}")
        screenshot = proof_path.parent / view_proof["screenshot"]
        if sha256(screenshot) != view_proof.get("screenshotSha256"):
            raise RuntimeError(f"Agent View screenshot hash mismatch: {screenshot}")
        if view_proof.get("outerPageRequests") not in (["/"], ["/", "/favicon.ico"]):
            raise RuntimeError(f"Unexpected browser network request: {view_proof.get('outerPageRequests')}")
        view_proofs.append(view_proof)
    verification: dict[str, object] = {
        "schemaVersion": 1,
        "kind": "unsigned-provider-candidate",
        "packageId": "com.zicode.devtools.agent-assist",
        "packageVersion": "0.1.1",
        "candidateDirectory": DEST.relative_to(ROOT).as_posix(),
        "createdAt": datetime.now().astimezone().isoformat(timespec="seconds"),
        "sourceRevision": revision,
        "sourceWorkingTreeDirty": dirty,
        "platform": {"os": platform.system(), "release": platform.release(), "architecture": platform.machine()},
        "contract": {"version": contract_data["version"], "checksumManifestSha256": sha256(contract_index)},
        "profiles": {
            "model.catalog.read.v1": "=1.0.0-preview.1",
            "model.generate.v1": "=1.0.0-preview.1",
        },
        "permissions": ["model.catalog.read", "model.generate"],
        "files": file_entries,
        "directoryIndexSha256": hashlib.sha256(index_bytes).hexdigest(),
        "tests": tests,
        "viewProofs": view_proofs,
        "signature": {"signed": False, "publisherKeyAvailable": False},
        "acceptance": {
            "providerCandidate": True,
            "studioReceiptVerified": False,
            "nativeStudioEndToEnd": "pending",
            "pending": ["signature installation", "permission revocation", "model request", "update/rollback", "uninstall"],
        },
        "limitations": [
            "jsdom mock-Host coverage does not enforce Chromium iframe sandbox or CSP",
            "No Studio native installation, permission dialog, or live model request has been verified",
        ],
    }
    return file_entries, verification


def safe_source(relative: str) -> Path:
    source = ROOT / relative
    assert_no_links(source)
    resolved = source.resolve(strict=True)
    if not under_root(resolved, ROOT.resolve(strict=True)) or not resolved.is_file():
        raise RuntimeError(f"Invalid package source: {relative}")
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--refresh-verification", action="store_true",
                        help="refresh only the external proof for the frozen candidate; never edit package files")
    args = parser.parse_args()
    if ROOT.is_symlink() or not ROOT.is_dir():
        raise RuntimeError("Project root is not a real directory")
    manifest_path = safe_source("plugins/agent-assist-preview/plugin.json")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    expected_profiles = {
        "model.catalog.read.v1": "=1.0.0-preview.1",
        "model.generate.v1": "=1.0.0-preview.1",
    }
    if manifest.get("schemaVersion") != 2 or manifest.get("version") != "0.1.1":
        raise RuntimeError("Unexpected manifest schema or package version")
    if manifest.get("requiredProfiles") != expected_profiles:
        raise RuntimeError("Required profile pins do not match the vendored contract")
    permissions = {item.get("name") for item in manifest.get("permissions", {}).get("required", [])}
    if permissions != {"model.catalog.read", "model.generate"}:
        raise RuntimeError("Unexpected required permissions")
    if manifest.get("permissions", {}).get("optional", []) != [] or manifest.get("runtime") is not None:
        raise RuntimeError("The preview must not add optional permissions or a Worker runtime")
    contributes = manifest.get("contributes", {})
    if any(contributes.get(name, []) for name in ("capabilities", "commands", "piTools")):
        raise RuntimeError("The preview must not add executable tools or commands")
    if manifest.get("activationEvents") != ["onView:devtools.agent-assist.main"]:
        raise RuntimeError("Unexpected activation event")
    declared_views = contributes.get("views", [])
    if len(declared_views) != 1 or declared_views[0].get("entry") != "views/main.html":
        raise RuntimeError("The manifest must declare one self-contained main View")
    view = safe_source("plugins/agent-assist-preview/views/main.html")
    view_html = view.read_text(encoding="utf-8")
    if ('<script src=' in view_html.lower() or '<link ' in view_html.lower() or
            "window.ZiAgentAssistLimits" not in view_html or "window.zicode?.services?.models" not in view_html):
        raise RuntimeError("View must be self-contained and include its byte-limit helper")

    contract_manifest_path = safe_source("contracts/studio-model-generation-preview/1.0.0-preview.1/SHA256SUMS.json")
    contract_manifest = json.loads(contract_manifest_path.read_text(encoding="utf-8"))
    if contract_manifest.get("version") != "1.0.0-preview.1":
        raise RuntimeError("Unexpected contract snapshot version")
    for item in contract_manifest.get("files", []):
        path = safe_source(f"contracts/studio-model-generation-preview/1.0.0-preview.1/{item['path']}")
        if sha256(path) != item.get("sha256"):
            raise RuntimeError(f"Contract snapshot checksum mismatch: {item['path']}")

    assert_no_links(RELEASE)
    if args.refresh_verification:
        if VERIFY.exists():
            raise RuntimeError("Frozen verification receipts are immutable; create a new external receipt revision")
        entries, proof = create_verification()
        temporary = VERIFY.with_suffix(VERIFY.suffix + ".tmp")
        temporary.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        temporary.replace(VERIFY)
        print(f"Refreshed external verification only: {VERIFY}")
        print(f"Files: {len(entries)}; bytes: {sum(int(item['size']) for item in entries):,}; unsigned, native acceptance pending")
        return 0
    if DEST.exists() or VERIFY.exists():
        raise RuntimeError(f"Refusing to overwrite existing candidate or verification file: {DEST}")
    resolved_release = RELEASE.resolve(strict=True)
    if not under_root(DEST.absolute(), resolved_release):
        raise RuntimeError(f"Candidate path escapes release directory: {DEST}")

    sources = [(package_path, safe_source(source_path)) for package_path, source_path in PAYLOAD.items()]
    DEST.mkdir(parents=True, exist_ok=False)
    try:
        for package_path, source in sources:
            target = DEST / package_path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)

        file_entries = []
        for path in sorted(item for item in DEST.rglob("*") if item.is_file()):
            assert_no_links(path)
            file_entries.append({"path": path.relative_to(DEST).as_posix(), "size": path.stat().st_size, "sha256": sha256(path)})
        if not (DEST / "plugin.json").is_file() or not (DEST / "views/main.html").is_file():
            raise RuntimeError("Package root is missing plugin.json or its declared View")
        _, verification = create_verification()
        VERIFY.write_text(json.dumps(verification, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    except Exception:
        shutil.rmtree(DEST)
        raise

    print(f"Created unsigned candidate: {DEST}")
    print(f"Payload files: {len(file_entries)}; package size: {sum((DEST / item['path']).stat().st_size for item in file_entries):,} bytes")
    print(f"External verification: {VERIFY}")
    print(f"Directory index SHA-256: {verification['directoryIndexSha256']}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # Keep validation failure visible and non-destructive.
        print(f"Package not created: {error}", file=sys.stderr)
        raise SystemExit(1)
