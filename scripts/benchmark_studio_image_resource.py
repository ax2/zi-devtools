#!/usr/bin/env python3
"""Benchmark the image WASI resource adapter against the cached Wasmtime and Studio Worker."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import random
import struct
import subprocess
import sys
import threading
import time
import uuid

PROJECT = Path(__file__).resolve().parents[1]
TARGET = PROJECT / "target"
BENCH = TARGET / "image-resource-benchmark"
PYTHON_CACHE = TARGET / "diagnostics-verification-python36"
WASM = TARGET / "wasm32-wasip1" / "plugin" / "zi-image-wasi.wasm"
STUDIO_EXE = Path(r"E:\zicode-studio-desktop\target\debug\zicode-studio-desktop.exe")
WASMTIME_DLL = PYTHON_CACHE / "wasmtime" / "win32-x86_64" / "_wasmtime.dll"
WASMTIME_DLL_SHA256 = "e001d681b140a3b6a382f8cfa50bf1b5e81d809b6be11b77a84a3a96bee06e5b"
FUEL_BUDGET = 10_000_000
IMAGE_RESIZE_FUEL_BUDGET = 30_000_000_000
RESOURCE_LIMIT = 32 * 1024 * 1024
MEMORY_LIMIT = 256 * 1024 * 1024
STACK_LIMIT = 2 * 1024 * 1024
CHUNK = 64 * 1024
INPUT_MAGIC = b"ZIRW\0\0\0\x02"
OUTPUT_MAGIC = b"ZIWO\0\0\0\x01"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def wasm_memory_ceiling(wasm: bytes) -> int | None:
    def uleb(offset: int) -> tuple[int, int]:
        value = shift = 0
        while True:
            byte = wasm[offset]
            offset += 1
            value |= (byte & 0x7F) << shift
            if byte < 0x80:
                return value, offset
            shift += 7

    offset = 8
    while offset < len(wasm):
        section_id = wasm[offset]
        section_length, content = uleb(offset + 1)
        end = content + section_length
        if section_id == 5:
            count, cursor = uleb(content)
            if count:
                flags, cursor = uleb(cursor)
                _, cursor = uleb(cursor)  # initial pages
                if flags & 1:
                    pages, _ = uleb(cursor)
                    return pages * 65536
                return None
        offset = end
    return None


def ensure_under_target(path: Path) -> Path:
    path = path.resolve(strict=False)
    root = TARGET.resolve()
    if not path.is_relative_to(root):
        raise RuntimeError(f"benchmark output escapes project target: {path}")
    cursor = path
    while cursor != root:
        if cursor.exists() and cursor.is_symlink():
            raise RuntimeError(f"linked benchmark path rejected: {cursor}")
        cursor = cursor.parent
    return path


def load_image_library():
    from PIL import Image

    return Image


def make_fixtures():
    Image = load_image_library()
    BENCH.mkdir(parents=True, exist_ok=True)
    rng = random.Random(0x5A1C0DE)
    fixtures = []
    for label, size in (("4mp", (2000, 2000)), ("16mp", (4096, 3906))):
        raw = rng.randbytes(size[0] * size[1] * 3)
        path = BENCH / f"{label}-rgb-noise.jpg"
        Image.frombytes("RGB", size, raw).save(
            path, format="JPEG", quality=84, subsampling=2, optimize=True
        )
        del raw
        fixtures.append((label, path, "image/jpeg", Image.open(path).size))
    transparent = BENCH / "2mp-transparent.png"
    image = Image.new("RGBA", (2048, 1024), (21, 104, 187, 73))
    image.save(transparent, format="PNG", optimize=True)
    fixtures.append(("2mp-transparent", transparent, "image/png", image.size))
    profile_edge = BENCH / "profile-over-edge.jpg"
    Image.new("RGB", (5000, 2000), (12, 34, 56)).save(
        profile_edge, format="JPEG", quality=90, optimize=True
    )
    fixtures.append(("profile-over-edge", profile_edge, "image/jpeg", (5000, 2000)))
    damaged = BENCH / "damaged.png"
    damaged.write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDRbroken")
    fixtures.append(("damaged", damaged, "image/png", None))
    return fixtures


def inner_request(operation: dict, mime: str, resource: bytes) -> dict:
    return {
        "workerPath": "resource://com.zicode.devtools.image/runtime/tools.wasm",
        "workerKind": "wasi",
        "pluginId": "com.zicode.devtools.image",
        "sceneId": "programming",
        "capabilityId": "devtools.image.resize",
        "executionProfile": "resource.wasi.image.resize.v1",
        "commandId": None,
        "input": operation,
        "resourceInputProfile": "resource.wasi.input.draft2",
        "resourceOutputProfile": "resource.wasi.output.draft1",
        "resources": [
            {
                "index": 0,
                "resourceId": str(uuid.uuid4()),
                "contentVersion": sha256(resource),
                "mimeType": mime,
                "byteLength": len(resource),
            }
        ],
    }


def encode_frame(request: dict, module: bytes, mime: str, resource: bytes) -> bytes:
    header = {
        "profile": "resource.wasi.input.draft2",
        "request": request,
        "resources": [
            {
                "resourceId": request["resources"][0]["resourceId"],
                "contentVersion": sha256(resource),
                "mimeType": mime,
                "byteLength": len(resource),
            }
        ],
        "module": {"byteLength": len(module), "sha256": sha256(module)},
    }
    encoded = json.dumps(header, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(module) > 2 * 1024 * 1024 or len(resource) > RESOURCE_LIMIT:
        raise RuntimeError("module or resource exceeds Studio frame limits")
    if len(encoded) > 256 * 1024:
        raise RuntimeError("resource frame header exceeds Studio limit")
    return INPUT_MAGIC + struct.pack("<I", len(encoded)) + encoded + module + resource


def parse_output_frame(frame: bytes) -> tuple[dict, list[tuple[str, bytes]]]:
    if len(frame) < 12 or frame[:8] != OUTPUT_MAGIC:
        try:
            response = json.loads(frame)
        except Exception as exc:
            raise RuntimeError(f"Worker returned neither resource frame nor JSON: {exc}")
        if response.get("ok") is False:
            raise RuntimeError(f"Worker rejected request: {response.get('error')}")
        raise RuntimeError("Worker returned an unexpected non-binary response")
    header_size = struct.unpack_from("<I", frame, 8)[0]
    end = 12 + header_size
    if not header_size or end > len(frame):
        raise RuntimeError("invalid resource output header length")
    header = json.loads(frame[12:end])
    if header.get("schemaVersion") != 1 or header.get("profile") != "resource.wasi.output.draft1":
        raise RuntimeError("unexpected resource output profile")
    resources = []
    cursor = end
    for index, descriptor in enumerate(header.get("resources", [])):
        length = descriptor["byteLength"]
        payload = frame[cursor : cursor + length]
        if descriptor.get("index") != index or len(payload) != length:
            raise RuntimeError("invalid resource output range")
        if sha256(payload) != descriptor.get("contentVersion"):
            raise RuntimeError("resource output digest mismatch")
        resources.append((descriptor["mimeType"], payload))
        cursor += length
    if cursor != len(frame):
        raise RuntimeError("trailing bytes in resource output frame")
    return header["result"], resources


def process_peak_sampler(process: subprocess.Popen) -> tuple[threading.Thread, dict]:
    data = {"peakWorkingSetBytes": None, "sampledWorkingSetBytes": None, "samplingError": None}
    if os.name != "nt":
        return threading.Thread(target=lambda: None), data
    class Counters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong), ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
            ("PrivateUsage", ctypes.c_size_t),
        ]
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel32.OpenProcess.restype = ctypes.c_void_p
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    psapi.GetProcessMemoryInfo.argtypes = [ctypes.c_void_p, ctypes.POINTER(Counters), ctypes.c_ulong]
    psapi.GetProcessMemoryInfo.restype = ctypes.c_int
    handle = kernel32.OpenProcess(0x0400 | 0x0010, False, process.pid)
    if not handle:
        data["samplingError"] = ctypes.get_last_error()
        return threading.Thread(target=lambda: None), data

    def sample():
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        while process.poll() is None:
            if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
                peak = int(counters.PeakWorkingSetSize)
                current = int(counters.WorkingSetSize)
                data["peakWorkingSetBytes"] = max(data["peakWorkingSetBytes"] or 0, peak, current)
                data["sampledWorkingSetBytes"] = current
            time.sleep(0.005)
        if psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            data["peakWorkingSetBytes"] = max(data["peakWorkingSetBytes"] or 0, int(counters.PeakWorkingSetSize))
            data["sampledWorkingSetBytes"] = int(counters.WorkingSetSize)
        else:
            data["samplingError"] = ctypes.get_last_error()
        kernel32.CloseHandle(handle)

    thread = threading.Thread(target=sample, daemon=True)
    thread.start()
    return thread, data


def run_worker(exe: Path, frame: bytes, case: str = "worker", timeout: float = 12.0) -> dict:
    process = subprocess.Popen(
        [str(exe), "--zicode-plugin-worker"],
        cwd=exe.parent,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    sampler, memory = process_peak_sampler(process)
    started = time.perf_counter_ns()
    try:
        stdout, stderr = process.communicate(frame, timeout=timeout)
        timed_out = False
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
        timed_out = True
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    sampler.join(timeout=1)
    result = {
        "exitCode": process.returncode,
        "timedOut": timed_out,
        "workerWallMs": round(elapsed_ms, 3),
        "peakWorkingSetBytes": memory["peakWorkingSetBytes"],
        "memorySamplingError": memory["samplingError"],
        "stderr": stderr.decode("utf-8", errors="replace")[:4096],
    }
    try:
        output_result, resources = parse_output_frame(stdout)
        result["result"] = output_result
        result["outputResources"] = []
        for index, (mime, payload) in enumerate(resources):
            image_path = BENCH / f"{case}-output-{index}.{mime.rsplit('/', 1)[-1]}"
            image_path.write_bytes(payload)
            result["outputResources"].append(
                {"mimeType": mime, "byteLength": len(payload), "sha256": sha256(payload), "path": str(image_path)}
            )
    except Exception as exc:
        try:
            result["response"] = json.loads(stdout)
        except Exception:
            result["stdoutPrefixHex"] = stdout[:128].hex()
        result["error"] = str(exc)
    return result


def load_wasmtime():
    actual_hash = sha256(WASMTIME_DLL.read_bytes())
    if actual_hash != WASMTIME_DLL_SHA256:
        raise RuntimeError(f"cached Wasmtime DLL hash mismatch: {actual_hash}")
    os.add_dll_directory(str(WASMTIME_DLL.parent))
    sys.path.insert(0, str(PYTHON_CACHE))
    import wasmtime
    import wasmtime._ffi as ffi

    version = importlib.metadata.version("wasmtime")
    if version != "36.0.0":
        raise RuntimeError(f"unexpected cached Python Wasmtime binding: {version}")
    return wasmtime, ffi, version, actual_hash


def run_direct_wasmtime(wasmtime, ffi, wasm: bytes, request: dict, mime: str, resource: bytes, case: str, fuel_budget: int = FUEL_BUDGET) -> dict:
    work = BENCH / "direct" / case
    work.mkdir(parents=True, exist_ok=True)
    runtime_started = time.perf_counter_ns()
    stdin_path = work / "stdin.json"
    stdout_path = work / "stdout.json"
    stderr_path = work / "stderr.txt"
    guest_request = {
        key: request[key]
        for key in (
            "pluginId", "sceneId", "capabilityId", "commandId", "input",
            "resourceInputProfile", "resourceOutputProfile", "resources",
        )
    }
    stdin_path.write_bytes(json.dumps(guest_request, separators=(",", ":")).encode("utf-8"))
    input_calls = {"bytes": 0, "calls": 0}
    outputs: list[dict] = []
    output_call_count = 0

    config = wasmtime.Config()
    config.consume_fuel = True
    config.cranelift_opt_level = "none"
    ffi.wasmtime_config_max_wasm_stack_set(config.ptr(), STACK_LIMIT)
    engine = wasmtime.Engine(config)
    compile_started = time.perf_counter_ns()
    module = wasmtime.Module(engine, wasm)
    module_compile_ms = (time.perf_counter_ns() - compile_started) / 1_000_000
    store = wasmtime.Store(engine)
    store.set_limits(memory_size=MEMORY_LIMIT, instances=1, memories=2, tables=4)
    store.set_fuel(fuel_budget)
    wasi = wasmtime.WasiConfig()
    wasi.argv = ["zicode-plugin"]
    wasi.stdin_file = str(stdin_path)
    wasi.stdout_file = str(stdout_path)
    wasi.stderr_file = str(stderr_path)
    store.set_wasi(wasi)
    linker = wasmtime.Linker(engine)
    linker.define_wasi()

    def input_size(index: int) -> int:
        return len(resource) if index == 0 else -1

    def input_read(caller, index: int, offset: int, destination: int, length: int) -> int:
        if index != 0 or offset < 0 or length < 0 or length > CHUNK:
            return -1
        if input_calls["calls"] >= 4096:
            return -3
        input_calls["calls"] += 1
        start = min(offset, len(resource))
        payload = resource[start : min(start + length, len(resource))]
        if input_calls["bytes"] + len(payload) > 64 * 1024 * 1024:
            return -3
        try:
            caller["memory"].write(caller, payload, start=destination)
        except Exception:
            return -2
        input_calls["bytes"] += len(payload)
        return len(payload)

    def output_create(index: int, length: int) -> int:
        nonlocal output_call_count
        output_call_count += 1
        if index not in (1, 2, 3) or length < 0 or length > RESOURCE_LIMIT or len(outputs) >= 4:
            return -3
        outputs.append({"mimeIndex": index, "expected": length, "bytes": bytearray(), "sealed": False})
        return len(outputs) - 1

    def output_write(caller, index: int, source: int, length: int) -> int:
        nonlocal output_call_count
        output_call_count += 1
        if index < 0 or index >= len(outputs) or length <= 0 or length > CHUNK:
            return -1
        try:
            payload = caller["memory"].read(caller, source, source + length)
        except Exception:
            return -2
        record = outputs[index]
        if record["sealed"] or len(record["bytes"]) + length > record["expected"]:
            return -1
        record["bytes"].extend(payload)
        return length

    def output_finish(index: int) -> int:
        nonlocal output_call_count
        output_call_count += 1
        if index < 0 or index >= len(outputs):
            return -1
        record = outputs[index]
        if len(record["bytes"]) != record["expected"]:
            return -1
        record["sealed"] = True
        return 0

    linker.define_func("zicode_resource_input_draft1", "size", wasmtime.FuncType([wasmtime.ValType.i32()], [wasmtime.ValType.i64()]), input_size)
    linker.define_func("zicode_resource_input_draft1", "read", wasmtime.FuncType([wasmtime.ValType.i32(), wasmtime.ValType.i64(), wasmtime.ValType.i32(), wasmtime.ValType.i32()], [wasmtime.ValType.i32()]), input_read, access_caller=True)
    linker.define_func("zicode_resource_output_draft1", "create", wasmtime.FuncType([wasmtime.ValType.i32(), wasmtime.ValType.i64()], [wasmtime.ValType.i32()]), output_create)
    linker.define_func("zicode_resource_output_draft1", "write", wasmtime.FuncType([wasmtime.ValType.i32(), wasmtime.ValType.i32(), wasmtime.ValType.i32()], [wasmtime.ValType.i32()]), output_write, access_caller=True)
    linker.define_func("zicode_resource_output_draft1", "finish", wasmtime.FuncType([wasmtime.ValType.i32()], [wasmtime.ValType.i32()]), output_finish)

    instantiate_started = time.perf_counter_ns()
    instance = linker.instantiate(store, module)
    instantiate_ms = (time.perf_counter_ns() - instantiate_started) / 1_000_000
    start = instance.exports(store)["_start"]
    guest_started = time.perf_counter_ns()
    error = None
    try:
        start(store)
    except Exception as exc:
        error = str(exc)
    guest_ms = (time.perf_counter_ns() - guest_started) / 1_000_000
    remaining_fuel = None
    try:
        remaining_fuel = store.get_fuel()
    except Exception:
        pass
    memory = instance.exports(store).get("memory")
    linear_bytes = memory.data_len(store) if memory else None
    stdout = stdout_path.read_bytes() if stdout_path.exists() else b""
    stderr = stderr_path.read_text(encoding="utf-8", errors="replace") if stderr_path.exists() else ""
    result = {
        "case": case,
        "guestExecutionMs": round(guest_ms, 3),
        "moduleCompileMs": round(module_compile_ms, 3),
        "instantiateMs": round(instantiate_ms, 3),
        "directRuntimeWallMs": round((time.perf_counter_ns() - runtime_started) / 1_000_000, 3),
        "fuelBudget": fuel_budget,
        "fuelRemaining": remaining_fuel,
        "fuelUsed": fuel_budget - remaining_fuel if remaining_fuel is not None else None,
        "linearMemoryBytesAtEnd": linear_bytes,
        "memoryHostLimitBytes": MEMORY_LIMIT,
        "moduleMemoryCeilingBytes": wasm_memory_ceiling(wasm),
        "inputReadCalls": input_calls["calls"],
        "outputCalls": output_call_count,
        "error": error,
        "stderr": stderr[:4096],
    }
    if stdout:
        result["result"] = json.loads(stdout)
    if outputs:
        result["outputResources"] = []
        for index, item in enumerate(outputs):
            mime_type = {1: "image/png", 2: "image/jpeg", 3: "image/webp"}[item["mimeIndex"]]
            payload = bytes(item["bytes"])
            extension = mime_type.rsplit("/", 1)[-1]
            output_path = work / f"output-{index}.{extension}"
            output_path.write_bytes(payload)
            result["outputResources"].append(
                {
                    "mimeType": mime_type,
                    "byteLength": len(payload),
                    "sha256": sha256(payload),
                    "path": str(output_path),
                    "decodedOutput": image_metadata(payload),
                }
            )
    return result


def image_metadata(payload: bytes) -> dict:
    Image = load_image_library()
    from io import BytesIO

    with Image.open(BytesIO(payload)) as image:
        image.load()
        result = {"size": list(image.size), "mode": image.mode, "format": image.format}
        if "A" in image.getbands():
            result["alphaSample"] = image.getpixel((0, 0))[-1]
        return result


def compare_decoded_images(native_path: Path, wasm_path: Path) -> dict:
    Image = load_image_library()
    from PIL import ImageChops

    with Image.open(native_path) as native_image, Image.open(wasm_path) as wasm_image:
        native = native_image.convert("RGBA")
        wasm = wasm_image.convert("RGBA")
        if native.size != wasm.size:
            return {"sameSize": False, "nativeSize": list(native.size), "wasmSize": list(wasm.size)}
        difference = ImageChops.difference(native, wasm)
        channels = difference.split()
        mask = ImageChops.lighter(
            ImageChops.lighter(channels[0].point(lambda value: 255 if value else 0), channels[1].point(lambda value: 255 if value else 0)),
            ImageChops.lighter(channels[2].point(lambda value: 255 if value else 0), channels[3].point(lambda value: 255 if value else 0)),
        )
        changed_pixels = mask.histogram()[255]
        extrema = [channel.getextrema()[1] for channel in channels]
        return {
            "sameSize": True,
            "size": list(native.size),
            "differentPixels": changed_pixels,
            "totalPixels": native.width * native.height,
            "maxChannelDelta": max(extrema),
            "byteIdentical": native.tobytes() == wasm.tobytes(),
        }


def make_format_matrix() -> tuple[Path, dict[str, Path]]:
    Image = load_image_library()
    matrix_dir = BENCH / "format-matrix"
    matrix_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(0x1A6E_2026)
    raw = rng.randbytes(1024 * 1024 * 3)
    source = Image.frombytes("RGB", (1024, 1024), raw)
    del raw
    inputs = {}
    for input_format, pil_format in (("png", "PNG"), ("jpeg", "JPEG"), ("webp", "WEBP")):
        path = matrix_dir / f"input.{input_format}"
        source.save(path, format=pil_format, quality=84, optimize=True)
        inputs[input_format] = path
    return matrix_dir, inputs


def run_native_reference(input_path: Path, output_path: Path, operation: dict) -> dict:
    completed = subprocess.run(
        [
            "cargo", "run", "--offline", "--locked", "-p", "zi-image-core",
            "--example", "image_core_benchmark", "--", str(input_path), str(output_path),
            str(operation["maxWidth"]), operation["format"], str(operation["quality"]),
        ],
        cwd=PROJECT,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(f"native image-core conformance failed: {completed.stderr[-2000:]}")
    payload = output_path.read_bytes()
    return {"path": str(output_path), "sha256": sha256(payload), "byteLength": len(payload), "decodedOutput": image_metadata(payload)}


def run_output_limit_probe(wasmtime, worker: Path, wasm: bytes, mime: str, resource: bytes) -> dict:
    import wasmtime

    result_json = b'{"ok":true,"probe":"output-limit-rejected"}'
    escaped = "".join(f"\\{byte:02x}" if byte in (0x22, 0x5c) or byte < 0x20 else chr(byte) for byte in result_json)
    wat = f'''(module
      (import "wasi_snapshot_preview1" "fd_write" (func $json (param i32 i32 i32 i32) (result i32)))
      (import "zicode_resource_output_draft1" "create" (func $create (param i32 i64) (result i32)))
      (memory (export "memory") 1)
      (data (i32.const 128) "{escaped}")
      (func (export "_start") (local $status i32)
        i32.const 3 i64.const 33554433 call $create local.tee $status
        i32.const -3 i32.ne if unreachable end
        i32.const 64 i32.const 128 i32.store
        i32.const 68 i32.const {len(result_json)} i32.store
        i32.const 1 i32.const 64 i32.const 1 i32.const 80 call $json drop))'''
    probe_module = wasmtime.wat2wasm(wat)
    request = inner_request({}, mime, resource)
    frame = encode_frame(request, probe_module, mime, resource)
    run = run_worker(worker, frame, case="output-limit", timeout=8.0)
    if "result" not in run or run["result"] != {"ok": True, "probe": "output-limit-rejected"}:
        raise RuntimeError(f"Studio output-limit probe failed: {run}")
    return {"result": run["result"], "workerWallMs": run["workerWallMs"], "exitCode": run["exitCode"]}


def run_rejection_probe(worker: Path, wasm: bytes, mime: str, resource: bytes, operation: dict, case: str) -> dict:
    request = inner_request(operation, mime, resource)
    frame = encode_frame(request, wasm, mime, resource)
    run = run_worker(worker, frame, case=case, timeout=12.0)
    result = run.get("result", {})
    return {
        "exitCode": run.get("exitCode"),
        "workerWallMs": run.get("workerWallMs"),
        "result": result,
        "outputResourceCount": len(run.get("outputResources", [])),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=STUDIO_EXE)
    parser.add_argument("--wasm", type=Path, default=WASM)
    args = parser.parse_args()
    worker = args.worker.resolve(strict=True)
    wasm_path = args.wasm.resolve(strict=True)
    ensure_under_target(BENCH)
    if "zicode-studio-desktop" not in str(worker).lower():
        raise RuntimeError("worker executable path does not identify the Studio desktop")
    wasm = wasm_path.read_bytes()
    if len(wasm) > 2 * 1024 * 1024:
        raise RuntimeError("image module exceeds Studio's 2 MiB snapshot ceiling")

    fixtures = make_fixtures()
    wasmtime, ffi, binding_version, dll_hash = load_wasmtime()
    manifest = {
        "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "wasm": {"path": str(wasm_path), "byteLength": len(wasm), "sha256": sha256(wasm), "moduleMemoryCeilingBytes": wasm_memory_ceiling(wasm)},
        "wasmBuildProfile": {"command": "cargo build --locked -p zi-image-wasi --profile plugin --target wasm32-wasip1", "memoryOverride": "crates/zi-image-wasi/build.rs adds 256 MiB max only for wasm32-wasip1; workspace compute default remains 64 MiB", "stackBytes": STACK_LIMIT},
        "studioWorker": {"path": str(worker), "sha256": sha256(worker.read_bytes())},
        "studioResourceProfile": {"hostMemoryBytes": MEMORY_LIMIT, "fuel": IMAGE_RESIZE_FUEL_BUDGET, "executionDeadlineMs": 10000, "maxInputResources": 1, "maxInputAndOutputBytes": RESOURCE_LIMIT, "maxOutputResources": 1, "maxOutputBytes": RESOURCE_LIMIT, "globalWorkerSlots": 2, "perPluginWorkerSlots": 1, "profile": "resource.wasi.image.resize.v1@1.0.0-preview.1"},
        "wasmtime": {"pythonBinding": binding_version, "cApiDllPath": str(WASMTIME_DLL), "cApiDllSha256": dll_hash, "cApiReleaseArchiveUrl": "https://github.com/bytecodealliance/wasmtime/releases/download/v36.0.2/wasmtime-v36.0.2-x86_64-windows-c-api.zip", "cApiReleaseArchiveSha256": "ae93e1949d169ac4412c735c08ecb4dee841023549d83f69c6e98cefca22e734", "memoryHostLimitBytes": MEMORY_LIMIT, "fuelBudget": FUEL_BUDGET, "maxStackBytes": STACK_LIMIT, "optimization": "none"},
        "fixtures": [],
    }

    for label, path, mime, expected_size in fixtures:
        resource = path.read_bytes()
        operation = {"maxWidth": str(2000 if label == "16mp" else 1000), "format": "png" if label == "2mp-transparent" else "webp", "quality": "82"}
        request = inner_request(operation, mime, resource)
        case = {"name": label, "inputPath": str(path), "inputBytes": len(resource), "inputSha256": sha256(resource), "inputMime": mime, "expectedInputSize": list(expected_size) if expected_size else None, "operation": operation}
        direct = run_direct_wasmtime(wasmtime, ffi, wasm, request, mime, resource, label)
        if direct["error"]:
            case["directWasmtimeExtendedFuel"] = run_direct_wasmtime(
                wasmtime, ffi, wasm, request, mime, resource, label + "-extended", 500_000_000
            )
            case["directWasmtimeFuelProbes"] = []
            previous = case["directWasmtimeExtendedFuel"]
            if previous["fuelRemaining"] == 0:
                for budget in (1_000_000_000, 2_000_000_000, 4_000_000_000, 8_000_000_000, 16_000_000_000, 32_000_000_000):
                    probe = run_direct_wasmtime(
                        wasmtime, ffi, wasm, request, mime, resource,
                        f"{label}-fuel-{budget}", budget,
                    )
                    case["directWasmtimeFuelProbes"].append(probe)
                    if not probe["error"]:
                        break
        module_frame = encode_frame(request, wasm, mime, resource)
        worker_result = run_worker(worker, module_frame, case=label, timeout=12.0)
        worker_result["case"] = label
        case["directWasmtime"] = direct
        case["studioWorker"] = worker_result
        candidates = [direct, case.get("directWasmtimeExtendedFuel"), *case.get("directWasmtimeFuelProbes", [])]
        successful = next(
            (
                candidate
                for candidate in candidates
                if candidate
                and not candidate.get("error")
                and candidate.get("result", {}).get("ok") is True
                and candidate.get("outputResources")
            ),
            None,
        )
        if successful:
            native_dir = BENCH / "native"
            native_dir.mkdir(parents=True, exist_ok=True)
            extension = operation["format"]
            native = run_native_reference(
                path,
                native_dir / f"{label}-output.{extension}",
                operation,
            )
            wasi_output = successful["outputResources"][0]
            case["nativeWasiConformance"] = {
                "native": native,
                "wasm": wasi_output,
                "byteIdentical": native["sha256"] == wasi_output["sha256"],
                "decodedPixelComparison": compare_decoded_images(
                    Path(native["path"]), Path(wasi_output["path"])
                ),
                "measuredFuel": successful["fuelUsed"],
                "measuredGuestMs": successful["guestExecutionMs"],
                "measuredLinearMemoryBytes": successful["linearMemoryBytesAtEnd"],
            }
        else:
            case["nativeWasiConformance"] = {"status": "not-compared", "reason": "no complete WASI output under tested fuel budgets"}
        for output in worker_result.get("outputResources", []):
            payload = Path(output["path"]).read_bytes()
            output["decodedOutput"] = image_metadata(payload)
        if label == "2mp-transparent" and successful:
            out = Path(successful["outputResources"][0]["path"])
            Image = load_image_library()
            with Image.open(out) as image:
                case["outputAlpha"] = {"bands": list(image.getbands()), "sample": image.getpixel((0, 0))}
        manifest["fixtures"].append(case)

    matrix_dir, matrix_inputs = make_format_matrix()
    manifest["formatMatrix"] = []
    for input_format, input_path in matrix_inputs.items():
        resource = input_path.read_bytes()
        for output_format in ("png", "jpeg", "webp"):
            operation = {"maxWidth": "800", "format": output_format, "quality": "82"}
            request = inner_request(operation, f"image/{input_format}", resource)
            case_name = f"{input_format}-to-{output_format}"
            direct = run_direct_wasmtime(
                wasmtime, ffi, wasm, request, f"image/{input_format}", resource,
                "format-matrix-" + case_name, 30_000_000_000,
            )
            case = {
                "case": case_name,
                "inputSize": [1024, 1024],
                "inputBytes": len(resource),
                "inputSha256": sha256(resource),
                "operation": operation,
                "directWasmtime": direct,
            }
            worker_frame = encode_frame(request, wasm, f"image/{input_format}", resource)
            case["studioWorker"] = run_worker(
                worker, worker_frame, case="worker-format-matrix-" + case_name, timeout=12.0
            )
            if not direct.get("error") and direct.get("result", {}).get("ok") is True and direct.get("outputResources"):
                native = run_native_reference(
                    input_path,
                    matrix_dir / f"native-{case_name}.{output_format}",
                    operation,
                )
                wasm_output = direct["outputResources"][0]
                case["nativeWasiConformance"] = {
                    "native": native,
                    "wasm": wasm_output,
                    "byteIdentical": native["sha256"] == wasm_output["sha256"],
                    "decodedPixelComparison": compare_decoded_images(
                        Path(native["path"]), Path(wasm_output["path"])
                    ),
                }
            manifest["formatMatrix"].append(case)

    png_resource = next(path.read_bytes() for label, path, mime, _ in fixtures if label == "damaged")
    valid_png = matrix_inputs["png"].read_bytes()
    manifest["probes"] = {
        "outputLimit": run_output_limit_probe(wasmtime, worker, wasm, "image/png", png_resource),
        "invalidParameters": run_rejection_probe(
            worker, wasm, "image/png", valid_png,
            {"maxWidth": "4097", "format": "png", "quality": "82"},
            "invalid-parameters",
        ),
        "mimeMismatch": run_rejection_probe(
            worker, wasm, "image/jpeg", valid_png,
            {"maxWidth": "800", "format": "png", "quality": "82"},
            "mime-mismatch",
        ),
        "cancellation": {"status": "not-exercised", "reason": "The standalone plugin-worker CLI has no request-cancellation control; the Host enforces the ten-second image-profile deadline."},
    }
    output_path = BENCH / "results.json"
    output_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"results": str(output_path), "fixtures": len(manifest["fixtures"]), "wasmSha256": sha256(wasm)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
