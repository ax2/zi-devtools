"""Issue a new external inventory receipt for JSON Object candidate 07."""

import hashlib
import json
import os
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / "release" / "plugin-candidates"
CANDIDATE = RELEASE / "json-object-0.1.0-candidate-07"
SOURCE_RECEIPT = RELEASE / "json-object-0.1.0-candidate-07-verification.json"
OUTPUT_RECEIPT = RELEASE / "json-object-0.1.0-candidate-07-verification-revision-01.json"
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


def within(root: Path, path: Path) -> Path:
    absolute_root = root.absolute()
    absolute = path.absolute()
    if not absolute.is_relative_to(absolute_root):
        raise RuntimeError(f"Path escapes {absolute_root}: {path}")
    cursor = absolute
    while cursor != absolute_root:
        if cursor.is_symlink():
            raise RuntimeError(f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    if absolute_root.is_symlink():
        raise RuntimeError(f"Linked root rejected: {absolute_root}")
    resolved_root = absolute_root.resolve(strict=True)
    resolved = absolute.resolve(strict=True)
    if not resolved.is_relative_to(resolved_root):
        raise RuntimeError(f"Resolved path escapes {resolved_root}: {path}")
    return resolved


def main() -> None:
    candidate = within(RELEASE, CANDIDATE)
    source_receipt = within(RELEASE, SOURCE_RECEIPT)
    output_receipt = OUTPUT_RECEIPT
    if output_receipt.exists():
        raise RuntimeError(f"Refusing to overwrite existing receipt: {output_receipt}")

    receipt = json.loads(source_receipt.read_text(encoding="utf-8"))
    rows = receipt.get("files")
    if not isinstance(rows, list) or not rows:
        raise RuntimeError("Source receipt has no file inventory")
    if any(set(row) != {"path", "size", "sha256"} for row in rows):
        raise RuntimeError("Unexpected file inventory shape")

    expected = {}
    for row in rows:
        relative = Path(row["path"])
        if relative.is_absolute() or ".." in relative.parts or "\\" in row["path"]:
            raise RuntimeError(f"Invalid relative path in receipt: {row['path']}")
        if row["path"] in expected:
            raise RuntimeError(f"Duplicate path in receipt: {row['path']}")
        expected[row["path"]] = row

    actual_paths = set()
    for directory, names, filenames in os.walk(candidate, followlinks=False):
        base = Path(directory)
        for name in names:
            path = base / name
            if path.is_symlink():
                raise RuntimeError(f"Linked directory rejected: {path}")
        for name in filenames:
            path = base / name
            if path.is_symlink() or not path.is_file():
                raise RuntimeError(f"Unsupported candidate entry: {path}")
            relative = path.relative_to(candidate).as_posix()
            data = path.read_bytes()
            actual_paths.add(relative)
            expected_row = expected.get(relative)
            if expected_row is None:
                raise RuntimeError(f"File is absent from source receipt: {relative}")
            if expected_row["size"] != len(data) or expected_row["sha256"] != sha256(data):
                raise RuntimeError(f"Candidate byte mismatch: {relative}")

    if actual_paths != set(expected):
        raise RuntimeError("Candidate and receipt inventory differ")

    index_rows = [
        {"path": row["path"], "size": row["size"], "sha256": row["sha256"]}
        for row in rows
    ]
    index_bytes = json.dumps(index_rows, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    index_digest = sha256(index_bytes)

    # Candidate 07 inherited a digest of path,sha256,size objects. The new
    # receipt declares and hashes the receiver's required path,size,sha256 form.
    receipt["files"] = index_rows
    receipt["directoryIndexSha256"] = index_digest
    receipt["directoryIndexEncoding"] = ENCODING
    receipt["receiptRevision"] = 1
    receipt["revisesReceipt"] = source_receipt.name
    within(RELEASE, output_receipt.parent)
    with output_receipt.open("xb") as stream:
        stream.write((json.dumps(receipt, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    print(
        f"PASS: {len(index_rows)} files, {sum(row['size'] for row in index_rows)} bytes; "
        f"directory index {index_digest}; receipt {sha256(output_receipt.read_bytes())}"
    )


if __name__ == "__main__":
    main()
