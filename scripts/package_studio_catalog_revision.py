"""Create an immutable catalog snapshot with verified provider-package mapping receipts."""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE_ROOT = ROOT / "release" / "plugin-candidates"
TOOLS_PATH = ROOT / "docs" / "tools.json"
MIGRATION_PATH = ROOT / "docs" / "studio-plugin-migration.json"
ENCODING = {
    "hash": "SHA-256",
    "encoding": "UTF-8",
    "content": "files array in listed order",
    "objectKeyOrder": ["path", "size", "sha256"],
    "ensureAscii": False,
    "separators": [",", ":"],
    "trailingNewline": False,
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def json_bytes(value, *, pretty=False) -> bytes:
    if pretty:
        return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def reject_link_chain(path: Path, root: Path) -> Path:
    root_abs = root.absolute()
    path_abs = path.absolute()
    if not path_abs.is_relative_to(root_abs):
        raise RuntimeError(f"Path escapes {root_abs}: {path}")
    cursor = path_abs
    while cursor != root_abs:
        if cursor.is_symlink():
            raise RuntimeError(f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    if root_abs.is_symlink():
        raise RuntimeError(f"Linked root rejected: {root_abs}")
    resolved_root = root_abs.resolve(strict=True)
    resolved = path_abs.resolve(strict=True)
    if not resolved.is_relative_to(resolved_root):
        raise RuntimeError(f"Resolved path escapes {resolved_root}: {path}")
    return resolved


def directory_files(directory: Path) -> list[dict]:
    rows = []
    for base, dirs, files in os.walk(directory, followlinks=False):
        base_path = Path(base)
        for name in dirs:
            path = base_path / name
            if path.is_symlink():
                raise RuntimeError(f"Linked package directory rejected: {path}")
        for name in files:
            path = base_path / name
            if path.is_symlink() or not path.is_file():
                raise RuntimeError(f"Unsupported package entry: {path}")
            data = path.read_bytes()
            rows.append({
                "path": path.relative_to(directory).as_posix(),
                "size": len(data),
                "sha256": sha256(data),
            })
    return sorted(rows, key=lambda row: row["path"])


def normalized_receipt_rows(receipt: dict) -> list[dict]:
    rows = receipt.get("files")
    if not isinstance(rows, list) or not rows:
        raise RuntimeError("Package receipt has no file inventory")
    result = []
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("path"), str):
            raise RuntimeError("Package receipt has an invalid file row")
        size = row.get("size", row.get("bytes"))
        if not isinstance(size, int) or size < 0 or not isinstance(row.get("sha256"), str):
            raise RuntimeError("Package receipt has an invalid size or digest")
        path = Path(row["path"])
        if path.is_absolute() or ".." in path.parts or "\\" in row["path"]:
            raise RuntimeError(f"Invalid package-relative path: {row['path']}")
        result.append({"path": row["path"], "size": size, "sha256": row["sha256"]})
    if len({row["path"] for row in result}) != len(result):
        raise RuntimeError("Duplicate package receipt paths")
    return result


def verify_package(candidate_path: str, receipt_path: str) -> dict:
    relative_candidate = Path(candidate_path)
    if relative_candidate.is_absolute() or ".." in relative_candidate.parts:
        raise RuntimeError(f"Candidate path must be project-relative: {candidate_path}")
    candidate = reject_link_chain(ROOT / relative_candidate, ROOT)
    receipt_relative = Path(receipt_path)
    if receipt_relative.is_absolute() or ".." in receipt_relative.parts:
        raise RuntimeError(f"Receipt path must be project-relative: {receipt_path}")
    receipt_file = reject_link_chain(ROOT / receipt_relative, ROOT)
    if receipt_file.is_relative_to(candidate):
        raise RuntimeError("Package receipt must be outside its candidate directory")
    receipt_bytes = receipt_file.read_bytes()
    receipt = json.loads(receipt_bytes)
    manifest_path = candidate / "plugin.json"
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise RuntimeError(f"Candidate has no safe plugin.json identity: {candidate_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    package_id = manifest.get("id")
    package_version = manifest.get("version")
    if not isinstance(package_id, str) or not isinstance(package_version, str):
        raise RuntimeError(f"Candidate manifest has no package identity: {candidate_path}")
    if receipt.get("packageId") not in (None, package_id) or receipt.get("packageVersion") not in (None, package_version):
        raise RuntimeError(f"Receipt package identity conflicts with manifest: {receipt_path}")
    declared = normalized_receipt_rows(receipt)
    actual = directory_files(candidate)
    if actual != sorted(declared, key=lambda row: row["path"]):
        raise RuntimeError(f"Package inventory does not match: {candidate_path}")

    encoding_status = "absent"
    index_digest = receipt.get("directoryIndexSha256")
    if index_digest:
        encoding = receipt.get("directoryIndexEncoding")
        if encoding is None:
            encoding_status = "encoding-unverified"
        else:
            if encoding == ENCODING:
                index_digest_actual = sha256(json_bytes(declared))
                if index_digest_actual != index_digest:
                    raise RuntimeError(f"Directory index mismatch: {receipt_path}")
                encoding_status = "matched"
            else:
                # Preserve provider-specific declarations without claiming the
                # receiver's stricter structured encoding check has matched.
                encoding_status = "encoding-unverified"
    return {
        "path": candidate_path,
        "receiptPath": receipt_path,
        "receiptSha256": sha256(receipt_bytes),
        "packageId": package_id,
        "packageVersion": package_version,
        "fileCount": len(actual),
        "totalBytes": sum(row["size"] for row in actual),
        "candidateTreeSha256": sha256(json_bytes(actual)),
        "directoryIndexSha256": index_digest,
        "directoryIndexStatus": encoding_status,
        "signature": "unsigned-or-unverified",
        "runtimeAccepted": False,
    }


def git_source_state() -> dict:
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip())
    return {"gitHead": revision, "workingTreeDirty": dirty}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--comparison", type=Path, required=True,
                        help="Studio's read-only compare-provider-catalog JSON output")
    parser.add_argument("--output-root", type=Path, default=RELEASE_ROOT)
    args = parser.parse_args()

    tools_bytes = TOOLS_PATH.read_bytes()
    migration_bytes = MIGRATION_PATH.read_bytes()
    tools = json.loads(tools_bytes)
    migration = json.loads(migration_bytes)
    tools_by_id = {row["id"]: row for row in tools["tools"]}
    migration_by_id = {row["sourceToolId"]: row for row in migration["tools"]}
    if len(tools_by_id) != len(tools["tools"]) or len(migration_by_id) != len(migration["tools"]):
        raise RuntimeError("Duplicate source tool IDs")
    if set(tools_by_id) != set(migration_by_id) or len(tools_by_id) != 154:
        raise RuntimeError("Expected exactly matching 154-ID source directories")
    if migration.get("acceptedCapabilities") != 0:
        raise RuntimeError("This provider snapshot must not imply Host acceptance")
    source_catalog_version = tools["version"]
    if migration.get("sourceCatalogVersion") != source_catalog_version:
        raise RuntimeError("Migration map is not synchronized to the source catalog version")

    comparison_path = reject_link_chain(args.comparison, ROOT)
    comparison_bytes = comparison_path.read_bytes()
    comparison = json.loads(comparison_bytes)
    if comparison.get("catalogSha256") != sha256(tools_bytes):
        raise RuntimeError("Studio comparison does not cover the exact current tools.json bytes")
    if comparison.get("catalogToolCount") != 154 or comparison.get("baselineToolCount") != 154:
        raise RuntimeError("Studio comparison does not cover both 154-tool catalogs")

    package_scopes: dict[tuple[str, str, str], dict] = {}
    mapping_tools = []
    for source_id, tool_row in tools_by_id.items():
        migration_row = migration_by_id[source_id]
        scopes = []
        for scope in migration_row.get("operationScopes", []):
            directory = scope.get("candidateDirectory")
            package_id = scope.get("packageId")
            package_version = scope.get("packageVersion")
            if not directory or not package_id or not package_version:
                continue
            key = (package_id, package_version, directory)
            package = package_scopes.setdefault(key, {"sourceToolIds": set(), "capabilities": set(), "scopeRows": []})
            package["sourceToolIds"].add(source_id)
            operation_id = scope.get("operationId", "")
            matches = [cap for cap in migration_row.get("candidateCapabilities", [])
                       if cap == operation_id or cap.endswith("." + operation_id)]
            package["capabilities"].update(matches)
            package["scopeRows"].append({
                "sourceToolId": source_id,
                "operationId": operation_id,
                "scopeKind": scope.get("scopeKind"),
                "scope": scope.get("scope"),
                "pluginStatus": scope.get("pluginStatus"),
                "blockingReason": scope.get("blockingReason"),
            })
            scopes.append({
                "operationId": operation_id,
                "packageId": package_id,
                "packageVersion": package_version,
                "candidateDirectory": directory,
                "verificationReceipt": scope.get("verificationReceipt"),
                "scopeKind": scope.get("scopeKind"),
                "scope": scope.get("scope"),
                "pluginStatus": scope.get("pluginStatus"),
                "blockingReason": scope.get("blockingReason"),
            })
        mapping_tools.append({
            "sourceToolId": source_id,
            "title": tool_row.get("name"),
            "sourceVersion": tool_row.get("tool_version"),
            "sourceStatus": tool_row.get("status"),
            "migration": migration_row.get("migration"),
            "pluginStatus": migration_row.get("pluginStatus"),
            "candidateCapabilities": migration_row.get("candidateCapabilities", []),
            "operationScopes": scopes,
            "uncoveredScope": migration_row.get("reason"),
        })

    package_records = []
    for (package_id, package_version, candidate), data in sorted(package_scopes.items()):
        receipt_paths = sorted({
            scope.get("verificationReceipt")
            for source_id in data["sourceToolIds"]
            for scope in migration_by_id[source_id].get("operationScopes", [])
            if scope.get("candidateDirectory") == candidate and scope.get("verificationReceipt")
        })
        if len(receipt_paths) > 1:
            raise RuntimeError(f"Conflicting package receipts for {candidate}: {receipt_paths}")
        candidate_name = Path(candidate).name
        receipt_path = receipt_paths[0] if receipt_paths else (Path(candidate).parent / f"{candidate_name}-verification.json").as_posix()
        receipt_info = verify_package(candidate, receipt_path)
        if receipt_info["packageId"] != package_id or receipt_info["packageVersion"] != package_version:
            raise RuntimeError(f"Receipt package identity mismatch: {receipt_path}")
        package_records.append({
            **receipt_info,
            "sourceToolIds": sorted(data["sourceToolIds"]),
            "capabilities": sorted(data["capabilities"]),
            "scopeRows": sorted(data["scopeRows"], key=lambda row: (row["sourceToolId"], row["operationId"])),
        })

    comparison_summary = {
        "catalogSha256": comparison["catalogSha256"],
        "matrixSha256": comparison["matrixSha256"],
        "baselineToolCount": comparison["baselineToolCount"],
        "catalogToolCount": comparison["catalogToolCount"],
        "statusCounts": comparison["statusCounts"],
        "added": comparison.get("added", []),
        "removed": comparison.get("removed", []),
        "versionStatusChanged": comparison.get("snapshotComparison", {}).get("versionStatusChanged", []),
        "scopeChanged": comparison.get("snapshotComparison", {}).get("scopeChanged", []),
        "discoveryChanged": comparison.get("snapshotComparison", {}).get("discoveryChanged", []),
    }
    if len(comparison_summary["versionStatusChanged"]) != 76 or len(comparison_summary["scopeChanged"]) != 70 or len(comparison_summary["discoveryChanged"]) != 76:
        raise RuntimeError("Frozen dev.105 comparison counts differ from the requested reviewed state")

    source_state = git_source_state()
    status_counts = comparison_summary["statusCounts"]
    package_map = {
        "schemaVersion": 1,
        "sourceCatalogVersion": source_catalog_version,
        "sourceToolCount": len(mapping_tools),
        "acceptedCapabilities": migration["acceptedCapabilities"],
        "receiptArtifacts": "each mapped provider receipt is copied into package-receipts/ and checked against its source SHA-256",
        "tools": sorted(mapping_tools, key=lambda row: row["sourceToolId"]),
        "packages": package_records,
    }
    package_map_bytes = json_bytes(package_map, pretty=True)
    change_bytes = json_bytes(comparison_summary, pretty=True)
    revision_hash = sha256(
        b"zicode-devtools-catalog-revision-v1\0"
        + sha256(tools_bytes).encode()
        + b"\0"
        + sha256(migration_bytes).encode()
        + b"\0"
        + sha256(package_map_bytes).encode()
        + b"\0"
        + sha256(change_bytes).encode()
    )
    revision_id = f"devtools-catalog-2026-10-18-{revision_hash[:16]}"
    payload = {
        "README.md": (
            "# ZiDevTools immutable catalog revision\n\n"
            f"Revision ID: `{revision_id}`  \n"
            f"Revision SHA-256: `{revision_hash}`  \n"
            f"Source application catalog version: `{source_catalog_version}` (legacy app version; not the directory revision ID)\n\n"
            f"This immutable snapshot contains {len(mapping_tools)} matching source and migration IDs. Status counts: "
            f"implemented {status_counts['implemented']}, in-progress {status_counts['in-progress']}, planned {status_counts['planned']}. "
            f"Against Studio's frozen dev.105 baseline: 0 added, 0 removed, 76 version/status changes, 70 scope changes, "
            f"76 discovery changes. It records {len(package_records)} unsigned provider package mappings and "
            f"{migration['acceptedCapabilities']} Host-accepted capabilities.\n\n"
            "The revision ID is content-derived from both source JSON files, the full per-tool package/scope map, and the frozen-baseline comparison. "
            "It does not replace or reinterpret the frozen dev.105 snapshot. Package receipt and directory hashes describe provider bytes only; they do not prove Studio installation, signature, runtime acceptance, or full-tool acceptance.\n"
        ).encode("utf-8"),
        "sources/tools.json": tools_bytes,
        "sources/studio-plugin-migration.json": migration_bytes,
        "comparison/frozen-dev105.json": change_bytes,
        "package-map.json": package_map_bytes,
    }
    for package in package_records:
        original_receipt = reject_link_chain(ROOT / package["receiptPath"], ROOT)
        receipt_bytes = original_receipt.read_bytes()
        if sha256(receipt_bytes) != package["receiptSha256"]:
            raise RuntimeError(f"Package receipt changed during snapshot assembly: {package['receiptPath']}")
        payload[f"package-receipts/{package['packageId']}-{package['packageVersion']}-verification.json"] = receipt_bytes
    files = [
        {"path": name, "size": len(data), "sha256": sha256(data)}
        for name, data in sorted(payload.items())
    ]
    index_digest = sha256(json_bytes(files))
    receipt = {
        "schemaVersion": 1,
        "kind": "ZiDevTools immutable source catalog directory revision",
        "revisionId": revision_id,
        "revisionSha256": revision_hash,
        "sourceCatalogVersion": source_catalog_version,
        "source": source_state,
        "sourceDirectories": [
            {"path": "sources/tools.json", "sha256": sha256(tools_bytes), "toolCount": len(tools_by_id)},
            {"path": "sources/studio-plugin-migration.json", "sha256": sha256(migration_bytes),
             "toolCount": len(migration_by_id), "acceptedCapabilities": migration["acceptedCapabilities"]},
        ],
        "changeSummary": {
            "baseline": "Studio frozen dev.105",
            "added": len(comparison_summary["added"]),
            "removed": len(comparison_summary["removed"]),
            "versionStatusChanged": len(comparison_summary["versionStatusChanged"]),
            "scopeChanged": len(comparison_summary["scopeChanged"]),
            "discoveryChanged": len(comparison_summary["discoveryChanged"]),
            "statusCounts": status_counts,
        },
        "packages": package_records,
        "files": files,
        "directoryIndexSha256": index_digest,
        "directoryIndexEncoding": ENCODING,
        "packageSha256": None,
        "signatureVerified": False,
        "runtimeAccepted": False,
        "fullToolAcceptance": False,
        "limitations": [
            "Unsigned source-directory snapshot; no .zicode-plugin container or signing key.",
            "This receipt proves source and candidate directory bytes only; Studio Host installation, Worker, Pi, lifecycle, and runtime acceptance remain unverified.",
            "Tool catalog status or source package mapping does not count as plugin acceptance.",
        ],
    }

    output_root = args.output_root
    if not output_root.is_absolute():
        output_root = ROOT / output_root
    output_root = reject_link_chain(output_root, ROOT)
    output = output_root / revision_id
    external_receipt = output_root / f"{revision_id}-verification.json"
    if output.exists() or external_receipt.exists():
        raise RuntimeError(f"Immutable revision already exists; refusing overwrite: {revision_id}")
    temporary = Path(tempfile.mkdtemp(prefix=f".{revision_id}-", dir=output_root))
    try:
        for relative, data in payload.items():
            target = temporary / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        if directory_files(temporary) != files:
            raise RuntimeError("Temporary revision verification failed")
        if sha256(json_bytes(files)) != index_digest:
            raise RuntimeError("Temporary directory index verification failed")
        os.rename(temporary, output)
        with external_receipt.open("xb") as stream:
            stream.write(json_bytes(receipt, pretty=True))
    except Exception:
        if temporary.exists():
            shutil.rmtree(temporary)
        raise

    print(f"Revision: {revision_id}")
    print(f"Directory: {output.relative_to(ROOT).as_posix()}")
    print(f"External receipt: {external_receipt.relative_to(ROOT).as_posix()}")
    print(f"Directory index SHA-256: {index_digest}; {len(files)} files, {sum(row['size'] for row in files)} bytes")
    print(f"Source catalog SHA-256: {sha256(tools_bytes)}")
    print(f"Migration map SHA-256: {sha256(migration_bytes)}")
    print(f"Mapped unsigned packages: {len(package_records)}; accepted capabilities: 0")


if __name__ == "__main__":
    main()
