"""Compare fixed native and production Windows Worker duplicate reports."""

import argparse
import hashlib
import json
from pathlib import Path

from verify_disk_inspector_plugin import compact, invoke, worker_frame

PROFILE = "resource.local-directory.duplicates.draft1"
PLUGIN = "com.zicode.devtools.duplicate-files"
CAPABILITY = "devtools.duplicate-files.scan"
PERMISSION = "resource.directory.duplicates.read"
MAX_INPUT = 60 * 1024
MAX_REPORT = 48 * 1024
INVALID = dict(ok=False, errorCode="INVALID_DIRECTORY_SNAPSHOT")


def payload(value):
    if isinstance(value, bytes):
        return value
    return compact(value)


def file(path, size, digest):
    return dict(path=path, logicalBytes=str(size), sha256=digest)


def snapshot(files, **overrides):
    return dict(
        schemaVersion=1, profile=PROFILE, files=files, visitedEntries=len(files),
        candidateFiles=len(files), hashedFiles=len(files),
        hashedBytes=str(sum(int(item["logicalBytes"]) for item in files)),
        skippedLinks=0, skippedDepth=0, skippedNames=0, unreadable=0, changedFiles=0,
        skippedBudget=0, limitReason=None, partial=False,
    ) | overrides


def group(size, sha256, paths, count=None):
    count = len(paths) if count is None else count
    return dict(logicalBytes=str(size), sha256=sha256, redundantBytes=str(size * (count - 1)),
                fileCount=count, paths=paths)


def report(groups, count, members, total, *, partial=False, projection=False, source=None):
    result = dict(ok=True, schemaVersion=1, material="duplicate-files-report", partial=partial,
                  groupCount=count, duplicateFileCount=members, redundantBytes=str(total),
                  groups=groups, projectionTruncated=projection)
    if source is not None:
        result["source"] = {k: v for k, v in source.items() if k not in ("schemaVersion", "profile", "files")}
    return result


def cases():
    a, b, c = "a" * 64, "b" * 64, "c" * 64
    empty = snapshot([])
    yield "empty", empty, report([], 0, 0, 0, source=empty)

    files = snapshot([file("目录/z.txt", 7, a), file("other.txt", 7, b), file("a.txt", 7, a),
                      file("zero-b", 0, c), file("zero-a", 0, c)])
    yield "identical-size-with-distinct-content-and-empty-pair", files, report([
        group(7, a, ["a.txt", "目录/z.txt"]), group(0, c, ["zero-a", "zero-b"]),
    ], 2, 4, 7, source=files)

    sizes = snapshot([file("small-b", 1, a), file("big-b", 2, a),
                      file("small-a", 1, a), file("big-a", 2, a)])
    yield "same-digest-is-still-size-specific", sizes, report([
        group(2, a, ["big-a", "big-b"]), group(1, a, ["small-a", "small-b"]),
    ], 2, 4, 3, source=sizes)

    partial = snapshot([file("a", 6, a), file("b", 6, a), file("remaining", 7, b)],
                       visitedEntries=8, candidateFiles=6, skippedLinks=1, changedFiles=1,
                       skippedBudget=2, partial=True, limitReason="hash_bytes")
    yield "partial-with-singleton-and-budget", partial, report([
        group(6, a, ["a", "b"]),
    ], 1, 2, 6, partial=True, source=partial)

    limit = 256 * 1024 * 1024
    bounded = snapshot([file("a", limit // 2, a), file("b", limit // 2, a)])
    yield "exact-host-byte-budget", bounded, report([
        group(limit // 2, a, ["a", "b"]),
    ], 1, 2, limit // 2, source=bounded)

    members = [f"f{i:03}" for i in range(256)]
    count = snapshot([file(path, 0, a) for path in reversed(members)])
    yield "max-digests-one-group", count, report([
        group(0, a, members[:12], len(members)),
    ], 1, 256, 0, projection=True, source=count)

    pairs = [
        (f"{i:064x}", [f"g{i:02d}/a", f"g{i:02d}/b"])
        for i in range(26)
    ]
    rank = snapshot([file(path, 1, digest) for digest, names in reversed(pairs) for path in reversed(names)])
    yield "stable-top-20", rank, report([
        group(1, digest, names) for digest, names in pairs[:20]
    ], 26, 52, 26, projection=True, source=rank)

    component = "界" * 80
    prefix = "/".join([component] * 8)
    long_pairs = [(f"{i:064x}", [f"g{i:02d}/{prefix}/a", f"g{i:02d}/{prefix}/b"]) for i in range(14)]
    large = snapshot([file(path, 1, digest) for digest, names in reversed(long_pairs) for path in names])
    encoded = compact(large)
    assert 50 * 1024 < len(encoded) < MAX_INPUT
    remaining = 32 * 1024
    expected = []
    for sha256, names in long_pairs:
        fit = min(len(names), remaining // len(names[0].encode()))
        expected.append(group(1, sha256, names[:fit], len(names)))
        remaining -= sum(len(name.encode()) for name in names[:fit])
    yield "exact-wire-budget-with-long-paths", encoded.ljust(MAX_INPUT, b" "), report(
        expected, 14, 28, 14, projection=True, source=large)
    yield "wire-budget-overage", encoded.ljust(MAX_INPUT + 1, b" "), dict(ok=False, errorCode="INPUT_TOO_LARGE")

    dense_pairs = [(f"{i:064x}", [f"g{i:03}/" + "x" * 120 + f"/{j}" for j in range(2)])
                   for i in range(128)]
    dense = snapshot([file(path, 1, sha256) for sha256, paths in reversed(dense_pairs) for path in paths])
    dense_wire = compact(dense)
    assert 58 * 1024 < len(dense_wire) <= MAX_INPUT
    yield "maximum-records-at-wire-budget", dense_wire.ljust(MAX_INPUT, b" "), report([
        group(1, sha256, paths) for sha256, paths in dense_pairs[:20]
    ], 128, 256, 128, projection=True, source=dense)

    for label, bad in [
        ("absolute-path", snapshot([file("C:/secret", 1, a)])),
        ("parent-path", snapshot([file("a/../b", 1, a)])),
        ("empty-component", snapshot([file("a//b", 1, a)])),
        ("control-character", snapshot([file("a\tb", 1, a)])),
        ("duplicate-relative-path", snapshot([file("a", 7, a), file("a", 7, b)])),
        ("invalid-digest", snapshot([file("a", 1, "A" * 64)])),
        ("noncanonical-size", snapshot([file("a", "01", a)])),
        ("overflow-size", snapshot([file("a", 2**64, a)], hashedBytes="0")),
        ("hash-count-mismatch", snapshot([file("a", 7, a)], hashedFiles=0)),
        ("hash-bytes-mismatch", snapshot([file("a", 7, a)], hashedBytes="1")),
        ("over-host-digest-budget", snapshot([file("a", limit + 1, a)])),
        ("mislabeled-partial", snapshot([], visitedEntries=1, skippedLinks=1)),
        ("wrong-profile", snapshot([], profile="resource.local-directory.usage.draft1")),
        ("unknown-limit", snapshot([], partial=True, limitReason="other")),
    ]:
        yield label, bad, INVALID


def verify_vectors(args, *, plugin, capability, vectors, extras):
    wasm = args.module.read_bytes()

    records = []
    for case, value, expected in vectors:
        data = payload(value)
        direct = invoke([args.native], data)
        try:
            wire = invoke([args.worker, "--zicode-plugin-worker"], worker_frame(
                wasm, data, plugin=plugin, capability=capability))
        except RuntimeError as error:
            raise RuntimeError(f"{case}: {error}") from error
        assert wire["ok"], (case, wire)
        guest = wire["result"]
        assert direct == guest, (case, "native/WASI mismatch")
        if expected["ok"]:
            assert guest == expected, (case, guest, expected)
        else:
            assert guest == dict(ok=False, error=guest["error"]), (case, guest)
            assert guest["error"]["code"] == expected["errorCode"], (case, guest, expected)
        row = dict(case=case, bytes=len(data), inputSha256=hashlib.sha256(data).hexdigest(),
                   ok=guest["ok"], partial=guest.get("partial"))
        row.update({key: guest.get(key) for key in extras})
        print(json.dumps(row, ensure_ascii=False), flush=True)
        records.append(row)
    proof = dict(passed=len(records), wasmSha256=hashlib.sha256(wasm).hexdigest(),
                 workerSha256=hashlib.sha256(args.worker.read_bytes()).hexdigest())
    if args.evidence:
        args.evidence.write_text(json.dumps(proof | {"cases": records}, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(proof))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    verify_vectors(parser.parse_args(), plugin=PLUGIN, capability=CAPABILITY, vectors=cases(),
                   extras=("groupCount", "projectionTruncated"))


if __name__ == "__main__":
    main()
