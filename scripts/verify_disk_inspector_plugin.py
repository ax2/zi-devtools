"""Compare fixed Windows-native and production WASI vectors for directory usage."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import uuid

PROFILE = "resource.local-directory.usage.draft1"
MAX_INPUT = 64 * 1024
MAX_U64 = 2**64 - 1
PLUGIN = "com.zicode.devtools.disk-inspector"
CAPABILITY = "devtools.disk-inspector.scan"


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def file(path, size):
    return {"kind": "file", "path": path, "logicalBytes": str(size)}


def directory(path):
    return {"kind": "directory", "path": path}


def snapshot(entries, **override):
    return dict(
        schemaVersion=1, profile=PROFILE, entries=entries, visitedEntries=len(entries),
        skippedLinks=0, skippedDepth=0, skippedNames=0, unreadable=0,
        limitReason=None, partial=False,
    ) | override


def ranking(path, size):
    return {"path": path, "logicalBytes": str(size), "pathTruncated": False}


def cases():
    yield "empty", snapshot([]), dict(
        ok=True, partial=False, logicalBytes="0", groups=[], largestFiles=[],
    )
    yield "exact-u64-and-groups", snapshot([
        directory("archive"), file("archive/large", MAX_U64),
        directory("empty"), file("root", 11),
    ]), dict(
        ok=True, partial=False, logicalBytes=str(MAX_U64 + 11),
        rootFilesLogicalBytes="11", groupCount=2, fileCount=2, directoryCount=2,
        groups=[ranking("archive", MAX_U64), ranking("empty", 0)],
        largestFiles=[ranking("archive/large", MAX_U64), ranking("root", 11)],
    )
    yield "host-partial", snapshot(
        [directory("a"), file("a/seen", 7)], visitedEntries=3, skippedLinks=1,
        limitReason="entries", partial=True,
    ), dict(ok=True, partial=True, logicalBytes="7", groupCount=1)
    yield "stable-top-20", snapshot([file(f"f{i:02d}", 2) for i in range(26)]), dict(
        ok=True, partial=False, logicalBytes="52", fileCount=26,
        largestFiles=[ranking(f"f{i:02d}", 2) for i in range(20)],
    )
    long_name = "界" * 190
    yield "long-local-path", snapshot([directory("a"), file("a/" + long_name, 9)]), dict(
        ok=True, partial=False, logicalBytes="9",
        largestFiles=[{"path": "a/" + "界" * 170, "logicalBytes": "9", "pathTruncated": True}],
    )

    # Dense direct groups exercise rank ordering and aggregation under the
    # ordinary Worker budget, independent of the oversized prefix path.
    groups = []
    for i in range(600):
        group = f"g{i:03d}"
        groups.extend([directory(group), file(f"{group}/x", 1)])
    full = snapshot(groups)
    assert len(compact(full)) < MAX_INPUT
    yield "dense-complete-groups", full, dict(
        ok=True, partial=False, logicalBytes="600", groupCount=600,
        groups=[ranking(f"g{i:03d}", 1) for i in range(20)],
    )

    # 4,096 actual source records. Independently count complete encoded records
    # that fit into the disclosed analysis budget, including JSON delimiters.
    def partial_case(entries):
        used = len(compact(snapshot([])).split(b'"entries":[', 1)[0]) + len(b'"entries":[')
        parsed = []
        for item in entries:
            used += len(compact(item)) + bool(parsed)
            if used > MAX_INPUT:
                break
            parsed.append(item)
        full = snapshot(entries)
        sizes = [int(item["logicalBytes"]) for item in parsed if item["kind"] == "file"]
        return full, dict(
            ok=True, partial=True, logicalBytes=str(sum(sizes)), fileCount=len(sizes),
            source=None,
            analysis=dict(examinedEntries=len(parsed), inputBytes=len(compact(full)),
                          readBytes=MAX_INPUT, limitReason="analysis_bytes"),
        )

    yield "budget-prefix", *partial_case([file(f"p{i:04d}", MAX_U64) for i in range(4096)])
    stem = "g/" + "a" * 220 + "/" + "b" * 220
    near_max, expected = partial_case(
        [directory("g"), directory(stem.rsplit("/", 1)[0]), directory(stem)]
        + [file(f"{stem}/f{i:04d}", i + 1) for i in range(4093)]
    )
    assert 2_000_000 < len(compact(near_max)) <= 2 * 1024 * 1024
    yield "near-2mib-host-snapshot", near_max, expected

    for case, value in [
        ("absolute", snapshot([file("C:/outside", 1)])),
        ("parent", snapshot([file("../outside", 1)])),
        ("unsafe-control-name", snapshot([file("line\nbreak", 1)])),
        ("duplicate-path", snapshot([file("x", 1), file("x", 2)])),
        ("orphan-nested-path", snapshot([file("missing/file", 1)])),
        ("overflow", snapshot([file("x", MAX_U64 + 1)])),
        ("wrong-profile", snapshot([], profile="resource.local-directory.draft1")),
        ("false-complete", snapshot([], visitedEntries=1, skippedLinks=1)),
    ]:
        yield case, value, {"ok": False, "errorCode": "INVALID_DIRECTORY_SNAPSHOT"}


def worker_frame(wasm, payload, *, plugin=PLUGIN, capability=CAPABILITY):
    header = compact({
        "profile": "resource.wasi.input.draft2",
        "request": {
            "workerKind": "wasi", "workerPath": "", "pluginId": plugin,
            "capabilityId": capability, "sceneId": "programming", "input": {},
        },
        "module": {"byteLength": len(wasm), "sha256": hashlib.sha256(wasm).hexdigest()},
        "resources": [{
            "resourceId": str(uuid.UUID("12121212-1212-4212-8212-121212121212")),
            "byteLength": len(payload), "mimeType": "text/plain",
            "contentVersion": hashlib.sha256(payload).hexdigest(),
        }],
    })
    return b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + wasm + payload


def invoke(command, data):
    result = subprocess.run(
        command, input=data, capture_output=True, timeout=8,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if result.returncode:
        raise RuntimeError(f"exit={result.returncode} stdout={result.stdout!r} "
                           f"stderr={result.stderr[-1800:]!r}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    wasm = args.module.read_bytes()

    evidence = []
    for case, value, expected in cases():
        data = compact(value)
        direct = invoke([args.native], data)
        try:
            wire = invoke([args.worker, "--zicode-plugin-worker"], worker_frame(wasm, data))
        except RuntimeError as error:
            raise RuntimeError(f"{case}: {error}") from error
        assert wire["ok"], (case, wire)
        guest = wire["result"]
        assert direct == guest, (case, "native/WASI difference")
        for key, invariant in expected.items():
            actual = guest.get("error", {}).get("code") if key == "errorCode" else guest[key]
            assert actual == invariant, (case, key, actual, invariant)
        row = {
            "case": case, "bytes": len(data), "inputSha256": hashlib.sha256(data).hexdigest(),
            "ok": guest["ok"], "partial": guest.get("partial"), "logicalBytes": guest.get("logicalBytes"),
        }
        print(json.dumps(row, ensure_ascii=False), flush=True)
        evidence.append(row)
    summary = {
        "passed": len(evidence), "wasmSha256": hashlib.sha256(wasm).hexdigest(),
        "workerSha256": hashlib.sha256(args.worker.read_bytes()).hexdigest(),
    }
    if args.evidence:
        args.evidence.write_text(json.dumps(summary | {"cases": evidence}, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
