"""Write a structured, byte-verifiable external receipt for an existing plugin candidate."""

import argparse
import hashlib
import json
import os
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE_ROOT = ROOT / "release"
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


def safe_release_path(value: str, *, must_exist: bool) -> Path:
    relative = Path(value)
    if relative.is_absolute() or ".." in relative.parts:
        raise RuntimeError(f"Path must remain under release: {value}")
    candidate = RELEASE_ROOT / relative
    root = RELEASE_ROOT.absolute()
    absolute = candidate.absolute()
    if not absolute.is_relative_to(root):
        raise RuntimeError(f"Path escapes {root}: {value}")
    cursor = absolute
    while cursor != root:
        if cursor.is_symlink():
            raise RuntimeError(f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    if root.is_symlink():
        raise RuntimeError(f"Linked root rejected: {root}")
    resolved_root = root.resolve(strict=True)
    resolved = absolute.resolve(strict=must_exist)
    if not resolved.is_relative_to(resolved_root):
        raise RuntimeError(f"Resolved path escapes {resolved_root}: {value}")
    if must_exist and not resolved.exists():
        raise RuntimeError(f"Required path does not exist: {value}")
    return resolved


def package_files(candidate: Path) -> list[dict]:
    rows = []
    for directory, names, filenames in os.walk(candidate, followlinks=False):
        base = Path(directory)
        for name in names:
            path = base / name
            if path.is_symlink():
                raise RuntimeError(f"Linked package directory rejected: {path}")
        for name in filenames:
            path = base / name
            if path.is_symlink() or not path.is_file():
                raise RuntimeError(f"Unsupported package entry: {path}")
            data = path.read_bytes()
            rows.append({
                "path": path.relative_to(candidate).as_posix(),
                "size": len(data),
                "sha256": sha256(data),
            })
    return sorted(rows, key=lambda row: row["path"])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("candidate", help="candidate directory, relative to release")
    parser.add_argument("source_receipt", help="existing external receipt, relative to release")
    parser.add_argument("output_receipt", help="new external receipt name, relative to release")
    args = parser.parse_args()

    candidate = safe_release_path(args.candidate, must_exist=True)
    source_path = safe_release_path(args.source_receipt, must_exist=True)
    output_path = safe_release_path(args.output_receipt, must_exist=False)
    if (
        not candidate.is_dir()
        or source_path.is_dir()
        or source_path.is_relative_to(candidate)
        or output_path.is_relative_to(candidate)
        or not output_path.parent.is_dir()
        or output_path.exists()
    ):
        raise RuntimeError("Candidate/receipt types are invalid or output receipt already exists")
    source = json.loads(source_path.read_text(encoding="utf-8"))
    listed = source.get("files")
    if not isinstance(listed, list) or not listed:
        raise RuntimeError("Source receipt has no file inventory")

    normalized = []
    seen = set()
    for row in listed:
        if not isinstance(row, dict) or not isinstance(row.get("path"), str):
            raise RuntimeError("Source receipt has an invalid file row")
        size = row.get("size", row.get("bytes"))
        if not isinstance(size, int) or size < 0 or not isinstance(row.get("sha256"), str):
            raise RuntimeError("Source receipt has an invalid file size or SHA-256")
        relative = Path(row["path"])
        if relative.is_absolute() or ".." in relative.parts or "\\" in row["path"]:
            raise RuntimeError(f"Invalid package-relative path: {row['path']}")
        if row["path"] in seen:
            raise RuntimeError(f"Duplicate package path: {row['path']}")
        seen.add(row["path"])
        normalized.append({"path": row["path"], "size": size, "sha256": row["sha256"]})

    actual = package_files(candidate)
    if actual != sorted(normalized, key=lambda row: row["path"]):
        raise RuntimeError("Candidate files differ from the source receipt; refusing receipt-only revision")

    index_rows = [
        {"path": row["path"], "size": row["size"], "sha256": row["sha256"]}
        for row in normalized
    ]
    index_digest = sha256(json.dumps(index_rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))
    source["files"] = index_rows
    source["directoryIndexSha256"] = index_digest
    source["directoryIndexEncoding"] = ENCODING
    source["receiptRevision"] = int(source.get("receiptRevision", 0)) + 1
    source["revisesReceipt"] = source_path.name

    payload = (json.dumps(source, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    with output_path.open("xb") as stream:
        stream.write(payload)
    print(
        f"PASS: {len(index_rows)} files, {sum(row['size'] for row in index_rows)} bytes; "
        f"directory index {index_digest}; receipt SHA-256 {sha256(payload)}"
    )


if __name__ == "__main__":
    main()
