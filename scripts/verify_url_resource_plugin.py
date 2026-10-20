#!/usr/bin/env python3
"""Run the URL resource adapter in Wasmtime with a bounded synthetic Host grant."""
from __future__ import annotations

import argparse
import hashlib
import json
import tempfile
import threading
import time
from urllib.parse import quote_from_bytes
from pathlib import Path
from importlib.metadata import version

import wasmtime
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
MAX_BYTES = 8192


def call(module: bytes, resource: bytes, mime: str = "text/plain", *, overrides=None, size_override=None, read_fault=False, metrics=None) -> dict:
    envelope = {
        "pluginId": "com.zicode.devtools.url-text-resource",
        "sceneId": "provider-verification",
        "capabilityId": "devtools.transforms.url.encode",
        "commandId": None,
        "input": {},
        "resourceInputProfile": "resource.wasi.input.draft2",
        "resources": [{
            "index": 0,
            "resourceId": "c2bd760a-f4d8-4e17-b04f-114cc73a28a4",
            "contentVersion": hashlib.sha256(resource).hexdigest(),
            "mimeType": mime,
            "byteLength": len(resource),
        }],
    }
    envelope.update(overrides or {})
    counts = {"sizeCalls": 0, "readCalls": 0}
    with tempfile.TemporaryDirectory(prefix="zi-url-resource-") as folder:
        root = Path(folder)
        (root / "input.json").write_bytes(json.dumps(envelope, separators=(",", ":")).encode())
        config = wasmtime.WasiConfig()
        config.stdin_file = str(root / "input.json")
        config.stdout_file = str(root / "stdout.json")
        config.stderr_file = str(root / "stderr.txt")
        engine_config = wasmtime.Config()
        engine_config.consume_fuel = True
        engine_config.epoch_interruption = True
        engine_config.max_wasm_stack = 2 * 1024 * 1024
        engine = wasmtime.Engine(engine_config)
        store = wasmtime.Store(engine)
        store.set_limits(memory_size=64 * 1024 * 1024)
        store.set_fuel(10_000_000)
        store.set_epoch_deadline(1)
        store.set_wasi(config)
        linker = wasmtime.Linker(engine)
        linker.define_wasi()

        def size(_caller, index: int) -> int:
            counts["sizeCalls"] += 1
            if size_override is not None:
                return size_override
            return len(resource) if index == 0 else -1

        def read(caller, index: int, offset: int, destination: int, length: int) -> int:
            counts["readCalls"] += 1
            if read_fault:
                return -1
            if index != 0 or offset < 0 or length < 0 or offset > len(resource):
                return -1
            chunk = resource[offset : offset + length]
            memory = caller.get("memory")
            if memory is None:
                return -1
            memory.write(caller, chunk, destination)
            return len(chunk)

        i32, i64 = wasmtime.ValType.i32(), wasmtime.ValType.i64()
        linker.define(store, "zicode_resource_input_draft1", "size",
                      wasmtime.Func(store, wasmtime.FuncType([i32], [i64]), size, access_caller=True))
        linker.define(store, "zicode_resource_input_draft1", "read",
                      wasmtime.Func(store, wasmtime.FuncType([i32, i64, i32, i32], [i32]), read, access_caller=True))
        compiled = wasmtime.Module(engine, module)
        timer = threading.Timer(5, engine.increment_epoch)
        start = time.monotonic()
        timer.start()
        try:
            instance = linker.instantiate(store, compiled)
            instance.exports(store)["_start"](store)
            elapsed = time.monotonic() - start
            output = (root / "stdout.json").read_bytes()
            assert elapsed < 5 and len(output) <= 32768
            if metrics is not None:
                metrics.update(**counts, fuelUsed=10_000_000-store.get_fuel(), elapsedMs=round(elapsed*1000,3),
                               resultBytes=len(output), resourceSha256=hashlib.sha256(resource).hexdigest())
            return json.loads(output)
        finally:
            timer.cancel()
            timer.join()
            store.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    portable = (ROOT / "plugin.json").is_file()
    parser.add_argument("--wasm", type=Path, default=ROOT / ("runtime/tools.wasm" if portable else "target/wasm32-wasip1/plugin/zi-text-resource-wasi.wasm"))
    parser.add_argument("--plugin-root", type=Path, default=ROOT if portable else ROOT / "plugins/url-text-resource")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if portable and args.output and args.output.resolve().is_relative_to(ROOT.resolve()):
        raise SystemExit("Verification output must remain outside the frozen candidate")
    module = args.wasm.read_bytes()
    imports = wasmtime.Module(wasmtime.Engine(), module).imports
    import_rows = sorted({(item.module, item.name) for item in imports})
    allowed_modules = {"wasi_snapshot_preview1", "zicode_resource_input_draft1"}
    assert {module_name for module_name, _ in import_rows} <= allowed_modules, import_rows
    assert {name for module_name, name in import_rows if module_name == "zicode_resource_input_draft1"} == {"size", "read"}, import_rows
    assert not any(name.startswith("sock_") or name in {"path_open", "path_create_directory", "path_unlink_file", "path_rename"} for _, name in import_rows), import_rows
    vectors = json.loads((args.plugin_root / "fixtures/success-vectors.json").read_text(encoding="utf-8"))
    negatives = json.loads((args.plugin_root / "fixtures/runtime-negative-vectors.json").read_text(encoding="utf-8"))
    rows = []
    schema = json.loads((args.plugin_root / "schemas/output.schema.json").read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)

    def invoke(case_id, data, mime="text/plain", **kwargs):
        metrics = {}
        result = call(module, data, mime, metrics=metrics, **kwargs)
        validator.validate(result)
        rows.append({"id": case_id, "ok": True, "result": result, "outputSchemaValid": True, **metrics})
        return result
    for case in vectors["cases"]:
        text = case.get("text")
        if text is None:
            data = case["pattern"].encode() * case["repeatBytes"]
            expected = {"ok": True, "encodedText": case["encodedText"].replace("a repeated 8192 times", "a" * 8192), "sourceByteCount": case["sourceByteCount"]}
        else:
            data = text.encode("utf-8")
            expected = {"ok": True, "encodedText": case["encodedText"], "sourceByteCount": case["sourceByteCount"]}
        actual = invoke(case["id"], data, case["mimeType"])
        assert actual == expected, (case["id"], actual, expected)
    for case in negatives["cases"]:
        if "bytesHex" in case:
            data = bytes.fromhex(case["bytesHex"])
        else:
            data = bytes.fromhex(case["repeatByteHex"]) * case["repeatBytes"]
        actual = invoke(case["id"], data, case["mimeType"])
        assert actual.get("ok") is False and actual.get("error", {}).get("code") == case["expectedError"], (case["id"], actual)
        if case["expectedError"] != "INVALID_UTF8":
            assert rows[-1]["sizeCalls"] == rows[-1]["readCalls"] == 0
    for case_id, data in [("maximum-percent-expansion", b" " * 8192), ("utf8-exact-byte-limit", "中".encode()*2730+b"ab")]:
        assert invoke(case_id, data) == {"ok": True, "encodedText": quote_from_bytes(data, safe=""), "sourceByteCount": 8192}
    for case_id, kwargs, code, expected_counts in [
        ("wrong-plugin", {"overrides": {"pluginId": "unrelated"}}, "INVALID_REQUEST", (0,0)),
        ("missing-resource", {"overrides": {"resources": []}}, "INVALID_RESOURCE", (0,0)),
        ("size-mismatch", {"size_override": 2}, "RESOURCE_READ_FAILED", (1,0)),
        ("read-failure", {"read_fault": True}, "RESOURCE_READ_FAILED", (1,1)),
    ]:
        actual = invoke(case_id, b"a", **kwargs)
        assert actual["ok"] is False and actual["error"]["code"] == code
        assert (rows[-1]["sizeCalls"], rows[-1]["readCalls"]) == expected_counts
    report = {
        "wasmBytes": len(module),
        "wasmSha256": hashlib.sha256(module).hexdigest(),
        "engine": f"Wasmtime {version('wasmtime')}",
        "budget": {"fuel": 10_000_000, "deadlineSeconds": 5, "memoryBytes": 64*1024*1024, "compileTimeIncluded": False},
        "outputSchemaSha256": hashlib.sha256((args.plugin_root / "schemas/output.schema.json").read_bytes()).hexdigest(),
        "resourceInputProfile": "resource.wasi.input.draft2",
        "importModule": "zicode_resource_input_draft1",
        "wasmImports": [{"module": module_name, "name": name} for module_name, name in import_rows],
        "sourceApiBoundary": "Adapter source uses no std::fs, std::env, reqwest, or direct path/network APIs; environ imports are Rust WASI startup imports and are not read by adapter logic.",
        "cases": rows,
        "hostCases": "Not executed here; see fixtures/host-negative-cases.json for Studio-only cancellation, revoke, and stale-grant acceptance tests.",
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"PASS {len(rows)} actual WASI resource cases; {len(module)} byte module; SHA-256 {report['wasmSha256']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
