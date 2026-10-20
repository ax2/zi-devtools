#!/usr/bin/env python3
"""Compare the JSON-object provider's native and real WASI adapters byte-for-byte."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent if (SCRIPT_DIR.parent / "plugins/json-object").is_dir() else None
PACKAGE_ROOT = None if PROJECT_ROOT else SCRIPT_DIR.parent
ROOT = PROJECT_ROOT or PACKAGE_ROOT
PLUGIN = PROJECT_ROOT / "plugins/json-object" if PROJECT_ROOT else PACKAGE_ROOT
WASI_RUNNER = PROJECT_ROOT / "scripts/run_text_wasi.mjs" if PROJECT_ROOT else SCRIPT_DIR / "run_text_wasi.mjs"
PLUGIN_ID = "com.zicode.devtools.json-object"
INSPECT = "devtools.json.object.inspect"
RECEIVE = "devtools.json.object.receive"


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def request(capability: str, value, scene="json-object-proof") -> bytes:
    row = {"pluginId": PLUGIN_ID, "sceneId": scene, "capabilityId": capability,
           "commandId": None, "input": {"value": value}}
    return json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wasm", type=Path, default=(PACKAGE_ROOT / "runtime/tools.wasm" if PACKAGE_ROOT else ROOT / "target/wasm32-wasip1/plugin/zi-json-object-wasi.wasm"))
    default_native = ROOT / "target/plugin/zi-json-object-wasi.exe" if PROJECT_ROOT else None
    parser.add_argument("--native", type=Path, default=default_native)
    parser.add_argument("--output", type=Path, default=Path.cwd() / "json-object-replay-proof.json")
    parser.add_argument("--cases-dir", type=Path, default=(PACKAGE_ROOT / "verification/cases" if PACKAGE_ROOT else ROOT / "target/json-object-proof/cases"))
    args = parser.parse_args()
    if PACKAGE_ROOT and args.output.resolve().is_relative_to(PACKAGE_ROOT.resolve()):
        raise ValueError('Replay output must be outside the frozen candidate directory')
    wasm = args.wasm.resolve(strict=True)
    native = args.native.resolve(strict=True) if args.native else None
    assert wasm.is_file() and WASI_RUNNER.is_file()
    module = wasm.read_bytes()
    assert 0 < len(module) <= 2 * 1024 * 1024
    manifest = json.loads((PLUGIN / "plugin.json").read_text(encoding="utf-8"))
    catalog = json.loads((PLUGIN / "catalog.json").read_text(encoding="utf-8"))
    required = {"output.schema.v1": "=1.0.0-rc.1", "result.json.v1": "=1.0.0-preview.1"}
    allowed = {**required, "tool.directory.v1": "=1.0.0-preview.1"}
    assert all(manifest["requiredProfiles"].get(key) == value for key, value in required.items())
    assert all(allowed.get(key) == value for key, value in manifest["requiredProfiles"].items())
    assert catalog["packageId"] == manifest["id"] and catalog["packageVersion"] == manifest["version"]
    assert {row["capabilityId"] for row in catalog["tools"]} == {INSPECT, RECEIVE}
    assert manifest["runtime"]["wasi"] == "runtime/tools.wasm"
    caps = {row["id"]: row for row in manifest["contributes"]["capabilities"]}
    assert set(caps) == {INSPECT, RECEIVE}
    assert all(row["resultCaptureMimeType"] == "application/json" and row["outputSchema"]["properties"]["data"]["type"] == "object" for row in caps.values())

    sample = {"name": "Zi", "enabled": True, "nested": {"count": 3}, "items": [None, 1]}
    stored = PACKAGE_ROOT / "verification/native-wasi-parity.json" if PACKAGE_ROOT else None
    if stored and stored.is_file():
        packaged = json.loads(stored.read_text(encoding="utf-8"))
        assert packaged["wasmSha256"] == digest(module)
        cases = [(row["id"], (args.cases_dir / row["requestFile"]).read_bytes(), row["expectedError"],
                  (args.cases_dir / row["expectedFile"]).read_bytes()) for row in packaged["cases"]]
    else:
        cases = [
            ("inspect-object", request(INSPECT, sample), None, None),
            ("receive-object", request(RECEIVE, sample), None, None),
            ("inspect-empty-object", request(INSPECT, {}), None, None),
            ("reject-array", request(INSPECT, [1, 2]), "INVALID_INPUT", None),
            ("reject-string", request(INSPECT, "text"), "INVALID_INPUT", None),
            ("reject-extra-input", json.dumps({"pluginId": PLUGIN_ID, "sceneId": "proof", "capabilityId": INSPECT,
                 "commandId": None, "input": {"value": {}, "extra": 1}}, separators=(",", ":")).encode(), "INVALID_INPUT", None),
            ("reject-duplicate-key", f'{{"pluginId":"{PLUGIN_ID}","sceneId":"proof","capabilityId":"{INSPECT}","commandId":null,"input":{{"value":{{"x":1,"x":2}}}}}}'.encode(), "INVALID_INPUT", None),
            ("reject-value-over-8k", request(INSPECT, {"v": "x" * 8192}), "INPUT_TOO_LARGE", None),
            ("reject-129-fields", request(INSPECT, {f"k{i}": i for i in range(129)}), "INVALID_INPUT", None),
            ("reject-envelope-over-48k", b" " * (48 * 1024 + 1), "INPUT_TOO_LARGE", None),
        ]
    results = []
    for index, (name, raw, expected_error, expected_bytes) in enumerate(cases):
        start = time.monotonic()
        native_run = subprocess.run([str(native)], input=raw, capture_output=True, timeout=5, check=True) if native else None
        wasi_run = subprocess.run(["node", "--disable-warning=ExperimentalWarning", str(WASI_RUNNER), str(wasm)],
                                  input=raw, capture_output=True, timeout=5, check=True)
        if native_run:
            assert native_run.stdout == wasi_run.stdout, name + ": native/WASI bytes differ"
            assert not native_run.stderr, name + ": native wrote to stderr"
        if expected_bytes is not None:
            assert expected_bytes == wasi_run.stdout, name + ": WASI result differs from packaged expected bytes"
        telemetry = json.loads(wasi_run.stderr)
        assert telemetry["exitCode"] == 0 and telemetry["memoryBytes"] <= 64 * 1024 * 1024, name
        result = json.loads(wasi_run.stdout)
        assert len(wasi_run.stdout) <= 48 * 1024, name + ": result too large"
        assert set(result) == {"contractVersion", "ok", "data", "error"}, name + ": protocol envelope fields changed"
        assert result["contractVersion"] == "1.0.0-rc.1", name + ": unsupported contract version"
        if expected_error:
            assert result["ok"] is False and result["error"]["code"] == expected_error, (name, result)
            assert result["data"] is None and isinstance(result["error"], dict), name + ": failure envelope is incomplete"
        else:
            assert result["ok"] is True and result["error"] is None and isinstance(result["data"], dict), name
            if name == "inspect-object":
                assert result["data"]["value"] == sample and result["data"]["keyCount"] == len(sample)
                assert result["data"]["valueTypes"]["nested"] == "object" and result["data"]["maxDepth"] == 3
            if name == "receive-object":
                assert result["data"]["receivedValue"] == sample and isinstance(result["data"]["receivedValue"], dict)
        req_name, expected_name = f"{index:03d}-{name}.request", f"{index:03d}-{name}.expected.json"
        results.append({"id": name, "requestFile": req_name, "expectedFile": expected_name,
                        "requestBytes": len(raw), "resultBytes": len(wasi_run.stdout),
                        "requestSha256": digest(raw), "expectedSha256": digest(wasi_run.stdout),
                        "expectedError": expected_error, "elapsedMs": round((time.monotonic() - start) * 1000, 3),
                        **telemetry})
        if PROJECT_ROOT:
            (args.cases_dir / req_name).parent.mkdir(parents=True, exist_ok=True)
            (args.cases_dir / req_name).write_bytes(raw)
            (args.cases_dir / expected_name).write_bytes(wasi_run.stdout)

    proof = {"profile": "result.json.v1@1.0.0-preview.1", "wasmBytes": len(module),
             "wasmSha256": digest(module), "nativeSha256": digest(native.read_bytes()) if native else None,
             "nativeCompared": native is not None,
             "cases": results, "limitations": ["Node WASI proves adapter behavior, not production sandbox/fuel", "Studio Host capture, transfer, save, permissions and lifecycle remain unverified"]}
    portable = {"wasmBytes": len(module), "wasmSha256": digest(module),
                "cases": [{"id": row["id"], "requestFile": row["requestFile"],
                           "expectedFile": row["expectedFile"], "requestSha256": row["requestSha256"],
                           "expectedSha256": row["expectedSha256"], "checks": {},
                           "expectedError": row["expectedError"]} for row in results]}
    if PROJECT_ROOT:
        (args.cases_dir / "fuel-proof.json").write_text(json.dumps(portable, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    mode = 'native/WASI byte-parity' if native else 'WASI packaged-result replay'
    print(f"PASS {len(results)} {mode} cases; module {len(module)} bytes; max output {max(row['resultBytes'] for row in results)} bytes")


if __name__ == "__main__":
    main()
