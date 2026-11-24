"""Check native and production Windows Worker parity for directory comparison."""

import argparse
from pathlib import Path

from verify_disk_inspector_plugin import compact
from verify_duplicate_files_plugin import verify_vectors

PROFILE = "resource.local-directory.compare.draft1"
PLUGIN = "com.zicode.devtools.directory-compare"
CAPABILITY = "devtools.directory-compare.scan"
PERMISSION = "resource.directory.compare.read"
MAX_INPUT = 60 * 1024
MAX_REPORT = 48 * 1024
ROW_BUDGET = MAX_REPORT - 2048
INVALID = dict(ok=False, errorCode="INVALID_DIRECTORY_SNAPSHOT")
STATES = ("onlyLeft", "onlyRight", "typeMismatch", "sizeMismatch", "contentMismatch",
          "unverified", "changed", "readError")


def directory():
    return {"kind": "directory"}


def file(size, digest=None):
    result = {"kind": "file", "logicalBytes": str(size)}
    if digest is not None:
        result["sha256"] = digest
    return result


def pair(path, left=None, right=None, verification=None):
    result = {"path": path}
    if left is not None:
        result["left"] = left
    if right is not None:
        result["right"] = right
    if verification is not None:
        result["verification"] = verification
    return result


def row(path, difference, left=None, right=None):
    return {"path": path, "difference": difference, **{
        key: value for key, value in pair(path, left, right).items() if key != "path"
    }}


def snapshot(rows, *, coverage=True, partial=False, reasons=()):
    return dict(schemaVersion=1, profile=PROFILE, coverageComplete=coverage, partial=partial,
                limitReasons=list(reasons), rows=rows)


def report(differences, *, same_files=0, same_dirs=0, unassessed=0, coverage=True,
           partial=False, reasons=(), projected=None):
    return dict(
        ok=True, schemaVersion=1, material="directory-compare-report", coverageComplete=coverage,
        partial=partial, limitReasons=list(reasons),
        totalPaths=same_files + same_dirs + unassessed + len(differences),
        sameFiles=same_files, sameDirectories=same_dirs, unassessedOneSide=unassessed,
        differenceCount=len(differences),
        differenceCounts={state: sum(row["difference"] == state for row in differences) for state in STATES},
        rows=differences if projected is None else projected,
        projectionTruncated=projected is not None and len(projected) < len(differences),
    )


def projection(differences):
    selected = []
    used = 0
    for item in differences:
        used += len(compact(item)) + 1
        if used > ROW_BUDGET:
            break
        selected.append(item)
    return selected


def cases():
    a, b = "a" * 64, "b" * 64
    yield "empty", snapshot([]), report([])
    mixed = [
        pair("size.txt", file(1), file(2**64 - 1)),
        pair("relative/目录.txt", file(7, a), file(7, b)),
        pair("empty.txt", file(0, a), file(0, a)),
        pair("dir", directory(), directory()),
        pair("only-a", file(2)),
        pair("only-b", right=directory()),
        pair("as-type", directory(), file(1)),
    ]
    yield "all-confirmed-differences-and-identical-counters", snapshot(mixed), report([
        row("as-type", "typeMismatch", directory(), file(1)),
        row("only-a", "onlyLeft", file(2)),
        row("only-b", "onlyRight", right=directory()),
        row("relative/目录.txt", "contentMismatch", file(7, a), file(7, b)),
        row("size.txt", "sizeMismatch", file(1), file(2**64 - 1)),
    ], same_files=1, same_dirs=1)

    bounds = ("depth", "read_error")
    yield "partial-inventory-suppresses-all-one-sided-claims", snapshot([
        pair("b", file(1)), pair("c", right=file(4), verification="readError"),
        pair("a", file(2, a), file(2, b)),
    ], coverage=False, partial=True, reasons=bounds), report([
        row("a", "contentMismatch", file(2, a), file(2, b)),
    ], unassessed=2, coverage=False, partial=True, reasons=bounds)
    yield "hash-limit-preserves-confirmed-absence", snapshot([
        pair("a", file(7, a), file(7)), pair("b", file(4)), pair("c", right=directory()),
    ], partial=True, reasons=("hash_bytes",)), report([
        row("a", "unverified", file(7, a), file(7)),
        row("b", "onlyLeft", file(4)), row("c", "onlyRight", right=directory()),
    ], partial=True, reasons=("hash_bytes",))

    yield "explicit-failures-supersede-identical-metadata", snapshot([
        pair("a", file(7, a), file(7, a), "changed"),
        pair("b", file(7, a), file(7), "readError"),
        pair("c", right=file(4), verification="changed"),
    ], partial=True, reasons=("changed", "read_error")), report([
        row("a", "changed", file(7, a), file(7, a)),
        row("b", "readError", file(7, a), file(7)),
        row("c", "changed", right=file(4)),
    ], partial=True, reasons=("changed", "read_error"))
    yield "equal-size-without-hashes-is-unverified", snapshot([
        pair("Large", file(2**64 - 1), file(2**64 - 1)),
        pair("small", file(0), file(0)),
    ]), report([
        row("Large", "unverified", file(2**64 - 1), file(2**64 - 1)),
        row("small", "unverified", file(0), file(0)),
    ], partial=True)
    yield "case-sensitive-path-matching", snapshot([
        pair("case", file(0)), pair("Case", right=file(0)),
    ]), report([
        row("Case", "onlyRight", right=file(0)), row("case", "onlyLeft", file(0)),
    ])
    yield "safe-future-limits", snapshot([], coverage=False, partial=True, reasons=("future_guard",)), report(
        [], coverage=False, partial=True, reasons=("future_guard",))
    # Preserve the size boundary and stable sorted prefix even with all 256
    # pairs occupied. Any rows beyond projection still contribute to totals.
    paths = [f"g{i:03}/" + "x" * 130 for i in range(256)]
    values = snapshot([pair(path, directory(), file(0)) for path in reversed(paths)])
    encoded = compact(values)
    assert 50 * 1024 < len(encoded) < MAX_INPUT
    dense = [row(path, "typeMismatch", directory(), file(0)) for path in paths]
    projected = projection(dense)
    assert 0 < len(projected) < len(dense)
    yield "max-pairs-at-max-wire-budget", encoded.ljust(MAX_INPUT, b" "), report(dense, projected=projected)
    yield "wire-budget-overage", encoded.ljust(MAX_INPUT + 1, b" "), dict(ok=False, errorCode="INPUT_TOO_LARGE")

    long_prefix = "/".join(["界" * 80] * 8)
    names = [f"g{i:02}/{long_prefix}/整" for i in range(28)]
    long = snapshot([pair(name, file(4), file(7)) for name in reversed(names)])
    assert 52 * 1024 < len(compact(long)) < MAX_INPUT
    differences = [row(name, "sizeMismatch", file(4), file(7)) for name in names]
    yield "whole-unicode-path-projection", long, report(differences, projected=projection(differences))

    for name, invalid in [
        ("too-many-pairs", snapshot([pair(f"f{i}", file(0)) for i in range(257)])),
        ("no-observation", snapshot([pair("empty")])),
        ("duplicate-relative-path", snapshot([pair("a", file(7)), pair("a", right=file(7))])),
        ("absolute-path", snapshot([pair("C:/private", directory())])),
        ("parent-path", snapshot([pair("a/../b", file(7))])),
        ("empty-path-component", snapshot([pair("a//b", file(7))])),
        ("control-character", snapshot([pair("a\nb", file(7))])),
        ("missing-logical-bytes", snapshot([pair("file", {"kind": "file"})])),
        ("noncanonical-logical-bytes", snapshot([pair("file", file("01"))])),
        ("overflow-logical-bytes", snapshot([pair("file", file(2**64))])),
        ("digest-on-directory", snapshot([pair("dir", directory() | {"sha256": a})])),
        ("malformed-digest", snapshot([pair("file", file(1, "A" * 64))])),
        ("unknown-verification", snapshot([pair("file", file(1), verification="otherError")])),
        ("duplicate-limit", snapshot([], partial=True, reasons=("time", "time"))),
        ("unsafe-limit", snapshot([], partial=True, reasons=("absolute/path",))),
        ("unknown-profile", snapshot([]) | {"profile": "resource.local-directory.duplicates.draft1"}),
    ]:
        yield name, invalid, INVALID


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    verify_vectors(parser.parse_args(), plugin=PLUGIN, capability=CAPABILITY, vectors=cases(),
                   extras=("totalPaths", "differenceCount", "projectionTruncated"))


if __name__ == "__main__":
    main()
