"""Verify single-image crop against the installed Windows Studio Worker."""
import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import struct
import subprocess

from PIL import Image

PLUGIN = "com.zicode.devtools.image-crop"
CAPABILITY = "devtools.image.crop"
PROFILE = "resource.wasi.image.crop.v1"
WIRE_PROFILE = "resource.wasi.image.resize.v1"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
FORMATS = {"PNG": "image/png", "JPEG": "image/jpeg", "WEBP": "image/webp"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode()


def image_bytes(format, size):
    image = Image.new("RGBA", size, (23, 145, 72, 80))
    if size == (4, 3):
        image.putdata([(40 * x, 60 * y, 10 + x, 44 * (x + y))
                       for y in range(3) for x in range(4)])
    if format == "JPEG":
        image = image.convert("RGB")
    result = BytesIO()
    image.save(result, format=format, **({"lossless": True, "exact": True} if format == "WEBP" else {}))
    return result.getvalue()


def animated_bytes(format):
    first = Image.new("RGBA", (4, 3), (255, 0, 0, 255))
    second = Image.new("RGBA", (4, 3), (0, 0, 255, 255))
    result = BytesIO()
    first.save(result, format=format, save_all=True, append_images=[second], duration=100)
    return result.getvalue()


def vectors():
    png = image_bytes("PNG", (4, 3))
    jpeg = image_bytes("JPEG", (8, 6))
    webp = image_bytes("WEBP", (4, 3))
    yield "png-alpha", png, FORMATS["PNG"], dict(x=0, y=0, width=3, height=2), None
    yield "jpeg-pixels", jpeg, FORMATS["JPEG"], dict(x=2, y=1, width=3, height=4), None
    yield "webp-edge", webp, FORMATS["WEBP"], dict(x=3, y=2, width=1, height=1), None
    yield "png-edge-4096", image_bytes("PNG", (4096, 1)), FORMATS["PNG"], dict(x=4095, y=0, width=1, height=1), None
    yield "png-viewport", image_bytes("PNG", (1280, 900)), FORMATS["PNG"], dict(x=10, y=11, width=960, height=600), None
    yield "invalid-rectangle", png, FORMATS["PNG"], dict(x=3, y=0, width=2, height=1), "INVALID_PARAMETERS"
    yield "zero-width", png, FORMATS["PNG"], dict(x=0, y=0, width=0, height=1), "INVALID_PARAMETERS"
    yield "too-wide", image_bytes("PNG", (4097, 1)), FORMATS["PNG"], dict(x=0, y=0, width=1, height=1), "IMAGE_DIMENSION_LIMIT"
    yield "invalid-image", b"invalid image", FORMATS["PNG"], dict(x=0, y=0, width=1, height=1), "INVALID_IMAGE"
    yield "mismatched-mime", png, FORMATS["JPEG"], dict(x=0, y=0, width=1, height=1), "IMAGE_OPERATION_FAILED"
    for format in ("PNG", "WEBP"):
        yield "animated-" + format.lower(), animated_bytes(format), FORMATS[format], dict(x=0, y=0, width=1, height=1), "INVALID_IMAGE"


def run_worker(worker, module, source, mime, rect):
    request = dict(workerPath="", workerKind="wasi", pluginId=PLUGIN, capabilityId=CAPABILITY,
                   sceneId="isolated-image-crop-proof", input=rect, executionProfile=WIRE_PROFILE,
                   resourceOutputProfile=OUTPUT)
    descriptor = dict(resourceId="5009ade2-15f1-43ec-ac26-b3b9946b6aef", contentVersion=digest(source),
                      mimeType=mime, byteLength=len(source))
    header = compact(dict(profile=INPUT, request=request, resources=[descriptor],
                          module=dict(byteLength=len(module), sha256=digest(module))))
    frame = b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + module + source
    result = subprocess.run([str(worker), "--zicode-plugin-worker"], input=frame, capture_output=True,
                            creationflags=subprocess.CREATE_NO_WINDOW, timeout=10, check=False)
    if result.returncode != 0 or not result.stdout.startswith(b"ZIWO\0\0\0\x01"):
        raise RuntimeError(result.stdout[:256].decode(errors="replace"))
    length = struct.unpack_from("<I", result.stdout, 8)[0]
    return json.loads(result.stdout[12:12+length]), result.stdout[12+length:]


def verify(worker, module):
    rows = []
    for case, source, mime, rect, expected_error in vectors():
        header, output = run_worker(worker, module, source, mime, rect)
        result = header["result"]
        row = dict(case=case, inputSha256=digest(source), inputMimeType=mime, rectangle=rect)
        if expected_error:
            assert result["ok"] is False and result["error"]["code"] == expected_error, (case, result)
            assert header["resources"] == [] and not output, case
            row["error"] = expected_error
        else:
            source_image = Image.open(BytesIO(source))
            x, y, width, height = (rect[key] for key in ("x", "y", "width", "height"))
            assert result == dict(ok=True, sourceWidth=source_image.width, sourceHeight=source_image.height,
                                  width=width, height=height, mimeType="image/png", byteLength=len(output)), (case, result)
            assert len(header["resources"]) == 1 and header["resources"][0]["mimeType"] == "image/png", case
            expected = source_image.crop((x, y, x + width, y + height)).convert("RGBA")
            actual = Image.open(BytesIO(output))
            assert actual.format == "PNG" and actual.size == (width, height), case
            assert actual.convert("RGBA").tobytes() == expected.tobytes(), case
            row["outputSha256"] = digest(output)
            row["outputBytes"] = len(output)
        rows.append(row)
        print(case, "passed")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    module = args.module.read_bytes()
    proof = dict(wasmSha256=digest(module), windowsWorkerSha256=digest(args.worker.read_bytes()),
                 workerProfile=WIRE_PROFILE, cases=verify(args.worker, module))
    args.output.write_bytes(compact(proof) + b"\n")


if __name__ == "__main__":
    main()
