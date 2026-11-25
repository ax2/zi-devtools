"""Compare fixed native and production-Windows-Worker log report vectors."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.log-viewer"
CAPABILITY = "devtools.log-viewer.filter"
PROFILE = "resource.wasi.input.draft2"
MAX_FILE = 16384
MAX_QUERY = 256
MAX_PREVIEW = 512
MAX_MATCHES = 12
MAX_RESULT = 48 * 1024


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def event(number, level, lines, text, clipped=False):
    return dict(eventNumber=number, level=level, lineCount=lines, text=text, previewTruncated=clipped)


def report(source, total, matches, count=None):
    count = len(matches) if count is None else count
    return dict(ok=True, sourceByteCount=len(source), eventCount=total, matchCount=count,
                matches=matches, truncated=count > len(matches))


def fixtures():
    start = "2026-11-25T13:00:00 INFO started\r\n"
    failure = "2026-11-25 13:00:01 ERROR failed\r\n\tat synthetic.Main.run(Main.java:42)\r\nCaused by: 连接超时\r\n"
    retry = " [2026-11-25 13:00:02] WARNING retry\n"
    unknown = "INFO_VAR"
    mixed = ("\ufeff" + start + failure + retry + unknown).encode()
    yield "multiline-bom-crlf", mixed, {}, "application/octet-stream", report(mixed, 4, [
        event(1, "info", 1, start), event(2, "error", 3, failure),
        event(3, "warning", 1, retry), event(4, "unknown", 1, unknown),
    ])
    yield "filter-continuation", mixed, {"level": "error", "query": "连接超时"}, "text/plain", report(mixed, 4, [
        event(2, "error", 3, failure),
    ])
    yield "case-sensitive-no-hit", mixed, {"query": "Started"}, "text/plain", report(mixed, 4, [])
    yield "empty-file", b"", {}, "text/plain", report(b"", 0, [])
    boundary = b"x\n" * (MAX_FILE // 2)
    yield "largest-dense-file", boundary, {}, "text/plain", report(boundary, len(boundary) // 2, [
        event(i + 1, "unknown", 1, "x\n") for i in range(MAX_MATCHES)
    ], len(boundary) // 2)
    for name, line, level in (("dense-level-tokens", "WARN\n", "warning"),
                              ("dense-timestamps", "2026-11-25T13:00:01 ERROR\n", "error")):
        count = MAX_FILE // len(line)
        source = line.encode() * count
        yield name, source, {"level": level}, "text/plain", report(source, count, [
            event(i + 1, level, 1, line) for i in range(MAX_MATCHES)
        ], count)
    continued = "INFO " + "界" * 300 + " last\n"
    exact = continued.encode()
    truncated = ("INFO " + "界" * 169)
    yield "search-beyond-utf8-preview", exact, {"query": "last"}, "text/plain", report(exact, 1, [
        event(1, "info", 1, truncated, True)
    ])
    raw = b"INFO " + b"\x01" * (MAX_PREVIEW - 6) + b"\n"
    dense_output = raw * MAX_MATCHES
    yield "escaped-report-limit", dense_output, {}, "text/plain", report(dense_output, MAX_MATCHES, [
        event(i + 1, "info", 1, raw.decode()) for i in range(MAX_MATCHES)
    ])
    controls = "Fatal error\nWARN now\nTRACE t\nDEBUG d"
    controls_bytes = controls.encode()
    yield "strict-level-tokens", controls_bytes, {"level": "fatal"}, "text/plain", report(controls_bytes, 4, [
        event(1, "fatal", 1, "Fatal error\n"),
    ])
    yield "too-big", b"x" * (MAX_FILE + 1), {}, "text/plain", "INPUT_TOO_LARGE"
    yield "invalid-utf8", b"\xff\x81", {}, "application/octet-stream", "INVALID_UTF8"
    yield "wrong-mime", b"INFO a", {}, "image/png", "UNSUPPORTED_MIME"
    yield "unknown-argument", b"INFO a", {"extra": ""}, "text/plain", "INVALID_REQUEST"
    yield "invalid-level", b"INFO a", {"level": "TRACE"}, "text/plain", "INVALID_REQUEST"
    yield "oversized-utf8-query", b"INFO a", {"query": "界" * 86}, "text/plain", "QUERY_TOO_LARGE"


def requests(wasm, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(wasm), sha256=digest(wasm)), resources=[resource]))
    wire = b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + wasm + payload
    return direct, wire


def invoke(command, request):
    result = subprocess.run(command, input=request, capture_output=True, timeout=9,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if result.returncode:
        raise RuntimeError(f"exit={result.returncode}, {result.stdout[:1200]!r}, {result.stderr[-1500:]!r}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + option, required=True, type=Path)
    args = parser.parse_args()
    wasm = args.module.read_bytes()
    recorded = []
    with TemporaryDirectory() as temporary:
        fixture = Path(temporary) / "synthetic.log"
        for name, source, inputs, mime, expected in fixtures():
            fixture.write_bytes(source)
            direct, wire = requests(wasm, source, inputs, mime)
            native = invoke([args.native, fixture], direct)
            try:
                response = invoke([args.worker, "--zicode-plugin-worker"], wire)
            except RuntimeError as error:
                raise RuntimeError(f"{name}: {error}") from error
            assert response["ok"], (name, response)
            actual = response["result"]
            assert actual == native, (name, "native/WASI mismatch")
            if isinstance(expected, str):
                assert not actual["ok"] and actual["error"]["code"] == expected, (name, actual)
            else:
                assert actual == expected, (name, actual)
            output = compact(actual)
            assert len(output) <= MAX_RESULT
            recorded.append(dict(case=name, inputBytes=len(source), inputSha256=digest(source), input=inputs,
                                 mime=mime, ok=actual["ok"], outputBytes=len(output), outputSha256=digest(output)))
            print(name, "passed", flush=True)
    proof = dict(passed=len(recorded), workerSha256=digest(args.worker.read_bytes()),
                 wasmSha256=digest(wasm), workerProfile=PROFILE, cases=recorded)
    args.evidence.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
