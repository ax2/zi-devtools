#!/usr/bin/env python3
"""Verify package boundary, manifest declaration, and every file hash."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PACKAGE = ROOT / "release/plugin-candidates/url-text-resource-0.1.1-candidate-01"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", type=Path, default=DEFAULT_PACKAGE)
    args = parser.parse_args()
    base = (ROOT / "release/plugin-candidates").resolve(strict=True)
    package = args.package.resolve(strict=True)
    require(package.is_relative_to(base), "Candidate package escapes release/plugin-candidates")
    cursor = package
    while cursor != base:
        require(not cursor.is_symlink() and not cursor.is_junction(), f"Linked package path rejected: {cursor}")
        cursor = cursor.parent
    receipt_path = package.parent / f"{package.name}-verification.json"
    require(receipt_path.is_file() and not receipt_path.is_symlink(), "External verification receipt missing or linked")
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    manifest = json.loads((package / "plugin.json").read_text(encoding="utf-8"))
    catalog = json.loads((package / "catalog.json").read_text(encoding="utf-8"))
    require(manifest["schemaVersion"] == 2 and manifest["id"] == "com.zicode.devtools.url-text-resource", "Manifest identity mismatch")
    require(manifest["requiredProfiles"] == {"resource.input.types.v1": "=1.0.0-preview.1", "output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": "=1.0.0-preview.1"}, "Required profile must be exactly pinned")
    require(manifest["runtime"] == {"wasi": "runtime/tools.wasm", "resourceInputProfile": "resource.wasi.input.draft2"}, "Resource runtime profile mismatch")
    capability = manifest["contributes"]["capabilities"]
    require(len(capability) == 1 and capability[0]["id"] == "devtools.transforms.url.encode", "Candidate must declare only the existing URL encode capability")
    require(capability[0]["resourceInputMimeTypes"] == ["text/plain"], "Candidate MIME allowlist mismatch")
    require(capability[0]["inputSchema"].get("additionalProperties") is False, "Input schema must reject extra fields")
    require(all(branch.get("additionalProperties") is False for branch in capability[0]["outputSchema"]["oneOf"]), "Output branches must reject extra fields")
    permissions = manifest["permissions"]
    required_permissions = permissions.get("required", [])
    require(set(permissions) <= {"required", "optional"} and len(required_permissions) == 1, "Unexpected permission declaration")
    require(required_permissions[0].get("name") == "resource.read" and bool(required_permissions[0].get("reason")), "Only resource.read may be required")
    require(not permissions.get("optional"), "Candidate must not add optional permissions")
    row = catalog["tools"][0]
    require(row["toolId"] == row["capabilityId"] == "devtools.transforms.url.encode" and row["version"] == "1.0.2", "Tool identity/version mismatch")
    require(receipt["deliveryStatus"] == "unsigned_provider_candidate_not_host_accepted", "Candidate status must remain explicit")
    host_cases = json.loads((package / "fixtures/host-negative-cases.json").read_text(encoding="utf-8"))
    require({case["id"] for case in host_cases["cases"]} == {"cancel-before-worker", "permission-revoked", "stale-transfer-grant"}, "Host negative-case coverage mismatch")
    files = receipt["files"]
    actual_paths = sorted(path.relative_to(package).as_posix() for path in package.rglob("*") if path.is_file())
    for path in package.rglob("*"):
        require(not path.is_symlink(), f"Linked package entry rejected: {path.relative_to(package)}")
    declared_paths = sorted(row["path"] for row in files)
    require(actual_paths == declared_paths, "Package file set differs from the SHA-256 receipt")
    for entry in files:
        data = (package / entry["path"]).read_bytes()
        require(len(data) == entry["size"] and sha256(data) == entry["sha256"], f"File receipt mismatch: {entry['path']}")
    index = hashlib.sha256(json.dumps(files, separators=(",", ":")).encode()).hexdigest()
    require(index == receipt["directoryIndexSha256"], "Directory index hash mismatch")
    require(sha256((package / "runtime/tools.wasm").read_bytes()) == receipt["wasmSha256"], "WASM hash mismatch")
    require(any("Host cancellation" in item for item in receipt["unverified"]), "Host-side negative tests must remain unverified")
    print(f"PASS package receipt: {len(files)} files; {sum(entry['size'] for entry in files)} bytes; unsigned provider candidate, not Host-accepted")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
