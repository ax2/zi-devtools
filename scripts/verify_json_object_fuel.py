#!/usr/bin/env python3
"""Run independent Wasmtime fuel/memory/deadline checks for JSON object cases."""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import tempfile
import threading
import time

import wasmtime
from wasmtime import _ffi

parser = argparse.ArgumentParser()
parser.add_argument("--wasm", type=Path, required=True)
parser.add_argument("--fixtures", type=Path, required=True)
parser.add_argument("--engine-provenance", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
engine_provenance = json.loads(args.engine_provenance.read_text(encoding="utf-8"))
assert Path(_ffi.dll._name).resolve() == Path(engine_provenance["dllPath"]).resolve()
assert hashlib.sha256(Path(_ffi.dll._name).read_bytes()).hexdigest() == engine_provenance["dllSha256"]
fixture_index = json.loads((args.fixtures / "fuel-proof.json").read_text(encoding="utf-8"))
config = wasmtime.Config()
config.consume_fuel = True
config.epoch_interruption = True
config.max_wasm_stack = 2 * 1024 * 1024
engine = wasmtime.Engine(config)
module_bytes = args.wasm.read_bytes()
assert 0 < len(module_bytes) <= 2 * 1024 * 1024
module = wasmtime.Module(engine, module_bytes)
rows, failures = [], []
for case in fixture_index["cases"]:
    request = (args.fixtures / case["requestFile"]).read_bytes()
    expected = (args.fixtures / case["expectedFile"]).read_bytes()
    assert hashlib.sha256(request).hexdigest() == case["requestSha256"]
    assert hashlib.sha256(expected).hexdigest() == case["expectedSha256"]
    with tempfile.TemporaryDirectory() as temporary:
        folder = Path(temporary)
        (folder / "stdin").write_bytes(request)
        wasi = wasmtime.WasiConfig()
        wasi.stdin_file = str(folder / "stdin")
        wasi.stdout_file = str(folder / "stdout")
        wasi.stderr_file = str(folder / "stderr")
        with wasmtime.Store(engine) as store:
            store.set_limits(memory_size=64 * 1024 * 1024)
            store.set_fuel(10_000_000)
            store.set_epoch_deadline(1)
            store.set_wasi(wasi)
            linker = wasmtime.Linker(engine)
            linker.define_wasi()
            timer = threading.Timer(5, engine.increment_epoch)
            start = time.monotonic()
            instance, trap = None, None
            timer.start()
            try:
                instance = linker.instantiate(store, module)
                instance.exports(store)["_start"](store)
            except Exception as exc:
                trap = str(exc).splitlines()[-1]
            finally:
                timer.cancel()
                timer.join()
            elapsed = time.monotonic() - start
            remaining = store.get_fuel()
            output = (folder / "stdout").read_bytes()
            stderr = (folder / "stderr").read_bytes()
            memory = instance.exports(store)["memory"].data_len(store) if instance is not None else None
    response = json.loads(expected)
    if case["expectedError"]:
        assert response["contractVersion"] == "1.0.0-rc.1"
        assert response["ok"] is False and response["error"]["code"] == case["expectedError"]
        assert response["data"] is None and isinstance(response["error"], dict)
    else:
        assert response["contractVersion"] == "1.0.0-rc.1"
        assert response["ok"] is True and response["error"] is None and isinstance(response.get("data"), dict)
    passed = trap is None and not stderr and output == expected and elapsed < 5 and memory <= 64 * 1024 * 1024
    if not passed:
        failures.append(case["id"])
    rows.append({"id": case["id"], "passBudget": passed, "trap": trap,
                 "fuelUsed": 10_000_000 - remaining, "fuelRemaining": remaining,
                 "elapsedMs": round(elapsed * 1000, 3), "memoryBytes": memory,
                 "requestBytes": len(request), "resultBytes": len(output),
                 "requestSha256": case["requestSha256"], "expectedSha256": case["expectedSha256"],
                 "expectedError": case["expectedError"]})
    print(f"{case['id']}: fuel={10_000_000 - remaining} pass={passed}", flush=True)
proof = {"fixtureKind": "plugin-runtime", "wasmBytes": len(module_bytes),
         "wasmSha256": hashlib.sha256(module_bytes).hexdigest(),
         "wasmtimeVersion": importlib.metadata.version("wasmtime"),
         "budget": {"fuel": 10_000_000, "deadlineSeconds": 5,
                    "memoryBytes": 64 * 1024 * 1024, "guestStackBytes": 2 * 1024 * 1024},
         "engineProvenance": engine_provenance, "cases": rows, "failures": failures,
         "limitations": ["Independent Wasmtime C-API; not Studio Host acceptance",
                         "No inherited environment or preopened directories; stdout redirected to runner files"]}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
assert not failures, failures
