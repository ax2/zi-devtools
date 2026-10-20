"""Read-only verification of frozen Actuator bytes, separate from live source drift."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from package_studio_catalog_revision import directory_files, normalized_receipt_rows, reject_link_chain

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "release/plugin-candidates"
NAME = "spring-actuator-0.1.1-candidate-02"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result


def load(path: Path):
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify_candidate(candidate: Path, receipt_path: Path, project_root: Path = ROOT) -> dict:
    candidate = reject_link_chain(candidate, project_root)
    receipt_path = reject_link_chain(receipt_path, project_root)
    require(not receipt_path.is_relative_to(candidate), "Receipt must be external")
    receipt = load(receipt_path)
    require(receipt["candidate"] == candidate.relative_to(project_root.resolve()).as_posix(), "Candidate identity mismatch")
    require(receipt["signing"] == {"signed": False, "authorizedSigningKeyProvided": False, "packageSha256": None}, "Signing claim mismatch")
    require(receipt["packageSha256"] is None and receipt["taskProfileDeclared"] is False, "Unsupported delivery claim")
    require(receipt["migrationMap"]["acceptedCapabilities"] == 0, "Unsupported acceptance claim")

    declared = normalized_receipt_rows(receipt)
    actual = directory_files(candidate)
    require(actual == sorted(declared, key=lambda row: row["path"]), "Candidate inventory differs from receipt")
    require(not any("__pycache__" in Path(row["path"]).parts or row["path"].endswith(".pyc") for row in actual), "Bytecode cache in candidate")
    index = digest(json.dumps(receipt["files"], ensure_ascii=False, separators=(",", ":")).encode("utf-8"))
    require(index == receipt["directoryIndexSha256"], "Directory index hash mismatch")
    manifest, catalog = load(candidate / "plugin.json"), load(candidate / "catalog.json")
    require(manifest["id"] == receipt["pluginId"] == "com.zicode.devtools.spring-actuator", "Plugin identity mismatch")
    require(manifest["version"] == receipt["pluginVersion"] == catalog["pluginVersion"], "Plugin version mismatch")
    require(catalog["pluginId"] == manifest["id"], "Catalog identity mismatch")
    require(manifest["requiredProfiles"] == receipt["requiredProfiles"] == {"service.http.get.v1": "=1.0.0-preview.1"}, "Profile claim mismatch")
    require(not manifest["contributes"].get("capabilities"), "View-only candidate unexpectedly declares capabilities")
    require(catalog["sourceTool"]["id"] == receipt["sourceTool"]["id"] == "spring-actuator", "Source identity mismatch")
    require(catalog["sourceTool"]["version"] == receipt["sourceTool"]["version"], "Source version mismatch")
    require(catalog["sourceTool"]["scopeComplete"] is False and catalog["delivery"]["fullSourceToolDelivered"] is False, "Full source scope overclaimed")

    def current_match(relative, recorded):
        path = project_root / relative
        if not path.exists():
            return None
        return digest(reject_link_chain(path, project_root).read_bytes()) == recorded

    return {
        "verified": True, "candidate": receipt["candidate"], "fileCount": len(actual),
        "totalBytes": sum(row["size"] for row in actual), "directoryIndexSha256": index,
        "sourceCatalogMatchesCurrent": current_match("docs/tools.json", receipt["sourceCatalog"]["currentSha256"]),
        "migrationMapMatchesCurrent": current_match("docs/studio-plugin-migration.json", receipt["migrationMap"]["sha256"]),
        "sourceSnapshotNote": "Historical source hashes are provenance, not a requirement for current working-tree equality.",
        "signed": False, "runtimeAccepted": False, "acceptedCapabilities": 0,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, default=BASE / NAME)
    parser.add_argument("--receipt", type=Path, default=BASE / f"{NAME}-verification.json")
    parser.add_argument("--require-current-source", action="store_true", help="Additionally require current source bytes to equal historical provenance")
    args = parser.parse_args()
    result = verify_candidate(args.candidate.absolute(), args.receipt.absolute())
    if args.require_current_source:
        require(result["sourceCatalogMatchesCurrent"] is True and result["migrationMapMatchesCurrent"] is True, "Frozen bytes verified, but live source differs from historical provenance")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
