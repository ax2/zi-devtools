"""Native/Worker comparison for the original-image ASCII resource adapter."""

import argparse
import binascii
import json
from pathlib import Path
import random
import struct
import subprocess
import time
import zlib

from PIL import Image, ImageDraw

import benchmark_studio_image_resource as harness

PLUGIN = "com.zicode.devtools.ascii-art-image"
CAPABILITY = "devtools.ascii.image.render"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
PROFILE = "resource.wasi.image.ascii.v1"
TEST_EXECUTION = "resource.wasi.image.resize.v1"
RES_BYTES = 10 * 1024 * 1024
RAMP = frozenset(b"@%#*+=-:. ")


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def fixture_matrix(directory, quick):
    directory.mkdir(parents=True, exist_ok=True)
    small = directory / "transparent.png"
    Image.new("RGBA", (16, 12), (3, 9, 30, 0)).save(small)
    small_rgba = directory / "small-pattern.png"
    with Image.new("RGBA", (41, 33)) as image:
        image.putdata([
            (x * 6 % 256, (x + y) * 11 % 256, y * 9 % 256, (x * y) % 256)
            for y in range(33) for x in range(41)
        ])
        image.save(small_rgba)
    for key, path, mime, width, expected in (
        ("transparent-white", small, "image/png", 8, None),
        ("small-rgba-width-min", small_rgba, "image/png", 8, None),
        ("small-rgba-width-max", small_rgba, "image/png", 120, None),
        ("width-below", small_rgba, "image/png", 7, "INVALID_WIDTH"),
        ("width-above", small_rgba, "image/png", 121, "INVALID_WIDTH"),
        ("width-fraction", small_rgba, "image/png", 8.5, "INVALID_REQUEST"),
        ("width-string", small_rgba, "image/png", "8", "INVALID_REQUEST"),
        ("missing-width", small_rgba, "image/png", {}, "INVALID_REQUEST"),
        ("unexpected-input", small_rgba, "image/png", {"width": 8, "path": "other.png"}, "INVALID_REQUEST"),
        ("unsupported-mime", small_rgba, "application/octet-stream", 80, "UNSUPPORTED_MIME"),
        ("mime-mismatch", small_rgba, "image/jpeg", 80, "IMAGE_MIME_MISMATCH"),
    ):
        yield key, path, mime, width, expected
    empty = directory / "empty.png"
    empty.write_bytes(b"")
    yield "empty", empty, "image/png", 80, "INVALID_IMAGE"
    damaged = directory / "malformed.png"
    damaged.write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDRbroken")
    yield "malformed", damaged, "image/png", 80, "INVALID_IMAGE"
    if quick:
        return

    def patterned(mode, dimensions, path, *, options):
        with Image.new(mode, dimensions, (57, 72, 120, 160) if mode == "RGBA" else (57, 72, 120)) as image:
            draw = ImageDraw.Draw(image)
            for i in range(91):
                x = (i * 97) % dimensions[0]
                y = (i * 167) % dimensions[1]
                color = ((i * 17) % 256, (i * 43) % 256, (i * 53) % 256)
                if mode == "RGBA":
                    color += ((i * 11) % 256,)
                draw.rectangle((x, y, min(x + dimensions[0] // 3, dimensions[0] - 1),
                                min(y + dimensions[1] // 3, dimensions[1] - 1)), fill=color)
            image.save(path, **options)

    png = directory / "20mp-alpha.png"
    patterned("RGBA", (5000, 4000), png, options={"compress_level": 6})
    yield "exact-20mp-png", png, "image/png", 120, None

    def png_chunk(name, payload):
        return struct.pack(">I", len(payload)) + name + payload + struct.pack(">I", binascii.crc32(name + payload))

    png16 = directory / "20mp-rgba16.png"
    compressor = zlib.compressobj()
    compressed = bytearray()
    dark = struct.pack(">4H", 256, 17000, 25000, 65535)
    light = struct.pack(">4H", 45000, 60000, 65400, 25000)
    for row in range(4000):
        edge = 2300 + (row // 16) % 200
        compressed.extend(compressor.compress(b"\x00" + dark * edge + light * (5000 - edge)))
    compressed.extend(compressor.flush())
    png16.write_bytes(b"\x89PNG\r\n\x1a\n" +
                      png_chunk(b"IHDR", struct.pack(">2I5B", 5000, 4000, 16, 6, 0, 0, 0)) +
                      png_chunk(b"IDAT", compressed) + png_chunk(b"IEND", b""))
    yield "exact-20mp-rgba16-png", png16, "image/png", 120, None

    jpeg = directory / "20mp-noise.jpg"
    dimensions = (5000, 4000)
    rng = random.Random(0xA5C11)
    with Image.frombytes("RGB", dimensions, rng.randbytes(dimensions[0] * dimensions[1] * 3)) as image:
        image.save(jpeg, quality=61, subsampling=2, optimize=True)
    assert jpeg.stat().st_size <= RES_BYTES, jpeg.stat().st_size
    yield "large-jpeg-noise", jpeg, "image/jpeg", 120, None

    webp = directory / "20mp-alpha.webp"
    patterned("RGBA", (5000, 4000), webp, options={"lossless": True, "method": 0})
    yield "exact-20mp-alpha-webp", webp, "image/webp", 120, None
    lossy = directory / "16mp-lossy.webp"
    patterned("RGB", (4000, 4000), lossy, options={"quality": 82, "method": 0})
    yield "large-lossy-webp", lossy, "image/webp", 120, None

    above_pixels = directory / "too-many-pixels.png"
    Image.new("RGB", (5001, 4000), (9, 12, 101)).save(above_pixels)
    yield "png-over-20mp", above_pixels, "image/png", 80, "IMAGE_DIMENSION_LIMIT"

    width_edge = directory / "width-20000.png"
    patterned("RGBA", (20_000, 1000), width_edge, options={"compress_level": 6})
    yield "width-20000", width_edge, "image/png", 120, None
    width_over = directory / "width-20001.png"
    Image.new("RGB", (20_001, 960), (9, 12, 101)).save(width_over)
    yield "width-20001", width_over, "image/png", 120, "IMAGE_DIMENSION_LIMIT"
    height_edge = directory / "height-20000.png"
    patterned("RGB", (32, 20_000), height_edge, options={"compress_level": 6})
    yield "height-20000", height_edge, "image/png", 80, None
    height_over = directory / "height-20001.png"
    Image.new("RGB", (32, 20_001), (95, 36, 91)).save(height_over)
    yield "height-20001", height_over, "image/png", 80, "IMAGE_DIMENSION_LIMIT"

    wide = directory / "20mp-single-row.png"
    Image.new("RGB", (20_000_000, 1), (50, 170, 220)).save(wide)
    yield "wide-20mp-single-row", wide, "image/png", 120, "IMAGE_DIMENSION_LIMIT"

    two_rows = directory / "20mp-two-rows.png"
    patterned("RGBA", (10_000_000, 2), two_rows, options={"compress_level": 6})
    yield "wide-20mp-two-rows", two_rows, "image/png", 120, "IMAGE_DIMENSION_LIMIT"

    tall = directory / "20mp-single-column.png"
    Image.new("1", (1, 20_000_000), 1).save(tall)
    assert tall.stat().st_size <= RES_BYTES
    yield "tall-20mp-single-column", tall, "image/png", 120, "IMAGE_DIMENSION_LIMIT"

    def padded_png(size):
        png_bytes = small.read_bytes()
        name = b"ziCX"
        payload = bytes(size - len(png_bytes) - 12)
        chunk = struct.pack(">I", len(payload)) + name + payload
        chunk += struct.pack(">I", binascii.crc32(name + payload))
        return png_bytes[:-12] + chunk + png_bytes[-12:]

    exact = directory / "ten-mib.png"
    exact.write_bytes(padded_png(RES_BYTES))
    assert exact.stat().st_size == RES_BYTES
    yield "exact-10-mib", exact, "image/png", 8, None
    above_bytes = directory / "over-ten-mib.png"
    above_bytes.write_bytes(padded_png(RES_BYTES + 1))
    yield "over-10-mib", above_bytes, "image/png", 8, "INPUT_TOO_LARGE"


def worker_request(digest, mime, binary, width, execution_profile):
    return {
        "workerKind": "wasi",
        "workerPath": "",
        "pluginId": PLUGIN,
        "sceneId": "programming",
        "capabilityId": CAPABILITY,
        # Temporary harness profile only. Production negotiates PROFILE.
        "executionProfile": execution_profile,
        "commandId": None,
        "input": width if isinstance(width, dict) else {"width": width},
        "resourceInputProfile": INPUT,
        "resourceOutputProfile": OUTPUT,
        "resources": [dict(index=0, resourceId="a8d18548-ef2d-4d12-8211-3fd949e1489d",
                           byteLength=len(binary), mimeType=mime, contentVersion=digest(binary))],
    }


def native_run(exe, path, request):
    request = {key: value for key, value in request.items()
               if key not in ("workerKind", "workerPath", "executionProfile")}
    process = subprocess.run([exe, path], input=compact(request), capture_output=True,
                             timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    if process.returncode:
        raise RuntimeError(f"Native exit={process.returncode}: {process.stderr[-2500:]!r}")
    metadata, _, raw = process.stdout.partition(b"\n")
    return json.loads(metadata), raw


def worker_run(harness, exe, frame):
    process = subprocess.Popen([exe, "--zicode-plugin-worker"], cwd=exe.parent,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               creationflags=subprocess.CREATE_NO_WINDOW)
    sampler, memory = harness.process_peak_sampler(process)
    started = time.perf_counter()
    timed_out = False
    try:
        stdout, stderr = process.communicate(frame, timeout=10.0)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.kill()
        stdout, stderr = process.communicate(timeout=3)
    sampler.join(timeout=1)
    evidence = dict(
        workerWallMs=round((time.perf_counter() - started) * 1000, 2),
        peakWorkingSetBytes=memory["peakWorkingSetBytes"],
        memorySamplingError=memory["samplingError"], stderr=stderr.decode("utf-8", errors="replace")[:4096],
        timedOut=timed_out, exitCode=process.returncode,
    )
    if timed_out:
        return None, [], evidence | {"error": "Worker exceeded 10s external deadline"}
    if stdout.startswith(harness.OUTPUT_MAGIC):
        metadata, resources = harness.parse_output_frame(stdout)
    else:
        try:
            metadata = json.loads(stdout)
            resources = []
        except (UnicodeDecodeError, json.JSONDecodeError):
            return None, [], evidence | {"error": f"Unparseable stdout: {stdout[:100]!r}"}
    return metadata, resources, evidence


def check_result(result, parts, width, expected):
    if expected:
        assert set(result) == {"ok", "error"} and result["ok"] is False, result
        assert set(result["error"]) == {"code", "message"} and result["error"]["code"] == expected, result
        assert parts == [], parts
        return
    typ, raw = parts[0]
    assert len(parts) == 1 and typ == "text/plain", parts
    assert result == {"ok": True, "width": width, "height": len(raw) // (width + 1), "byteLength": len(raw)}, result
    assert raw.endswith(b"\n")
    lines = raw[:-1].split(b"\n")
    assert 1 <= len(lines) <= 120
    assert len(lines) == result["height"]
    assert all(len(line) == width and set(line) <= RAMP for line in lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--worker-profile", choices=(TEST_EXECUTION, PROFILE), default=TEST_EXECUTION)
    parser.add_argument("--quick", action="store_true")
    args = parser.parse_args()

    if args.evidence.exists():
        parser.error('verification receipt already exists')
    digest = harness.sha256
    module = args.module.read_bytes()
    records = []
    prefix = args.evidence.parent
    prefix.mkdir(parents=True, exist_ok=True)
    assert 0 < len(module) <= 2 * 1024 * 1024
    assert harness.wasm_memory_ceiling(module) == harness.MEMORY_LIMIT
    for key, source, mime, width, expected in fixture_matrix(prefix / "fixtures", args.quick):
        data = source.read_bytes()
        request = worker_request(digest, mime, data, width, args.worker_profile)
        frame = harness.encode_frame(request, module, mime, data)
        native, native_raw = native_run(args.native, source, request)
        native_parts = [("text/plain", native_raw)] if native_raw else []
        check_result(native, native_parts, width, expected)
        baseline_match = None
        if args.reference and expected != "IMAGE_DIMENSION_LIMIT":
            old, old_raw = native_run(args.reference, source, request)
            if expected:
                baseline_match = old.get("error", {}).get("code") == expected and not old_raw
            else:
                baseline_match = (old, old_raw) == (native, native_raw)
        worker, parts, metrics = worker_run(harness, args.worker, frame)
        record = dict(case=key, inputBytes=len(data), inputSha256=digest(data), requestedWidth=width, mime=mime,
                      expectedError=expected, native=native, worker=worker, metrics=metrics,
                      matchesBaselineNative=baseline_match)
        try:
            check_result(worker, parts, width, expected)
            assert worker == native, (native, worker)
            assert parts == native_parts, "Worker and native text bytes differ"
            assert baseline_match is not False, "Optimized text differs from the pre-optimization native adapter"
            if key in ("transparent-white", "exact-10-mib"):
                assert native_raw == (b" " * 8 + b"\n") * 3
            if key == "height-20000":
                assert native["height"] == 120
            record.update(passed=True, outputSha256=digest(native_raw) if native_raw else None)
        except (AssertionError, TypeError, KeyError, IndexError) as error:
            record.update(passed=False, discrepancy=repr(error), nativeOutputSha256=digest(native_raw))
        records.append(record)
        print(key, record["passed"], metrics["workerWallMs"], worker, flush=True)

    results = dict(
        targetProfile=PROFILE, workerAdapterProfile=args.worker_profile,
        workerLimits=dict(fuel=harness.IMAGE_RESIZE_FUEL_BUDGET, memoryBytes=harness.MEMORY_LIMIT, deadlineMs=10000),
        sha256=dict(worker=digest(args.worker.read_bytes()), wasm=digest(module), native=digest(args.native.read_bytes())),
        moduleMemoryBytes=harness.wasm_memory_ceiling(module),
        referenceNativeSha256=digest(args.reference.read_bytes()) if args.reference else None,
        passed=sum(item["passed"] for item in records), cases=records,
    )
    args.evidence.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if results["passed"] != len(records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
