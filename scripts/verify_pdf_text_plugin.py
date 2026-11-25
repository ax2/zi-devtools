"""Check synthetic PDF pages through the native adapter and production Windows Worker."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory
import zlib

PLUGIN = "com.zicode.devtools.pdf-text"
CAPABILITY = "devtools.pdf-text.page"
PROFILE = "resource.wasi.input.draft2"
MAX_BYTES = 128 * 1024
MAX_PAGES = 32
MAX_TEXT = 6 * 1024
MAX_RESULT = 48 * 1024
MIME = "application/octet-stream"


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pdf(pages, *, compressed=False, comment_pad=0, image=False, encrypted=False):
    count = len(pages)
    font = 3 + 2 * count
    image_resource = f" /XObject << /Im1 {font+1} 0 R >>" if image else ""
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [" + b" ".join(f"{3+i} 0 R".encode() for i in range(count))
        + b"] /Count " + str(count).encode() + b" >>",
    ]
    for index in range(count):
        objects.append(f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 120]"
                       f" /Resources << /Font << /F1 {font} 0 R >>{image_resource} >>"
                       f" /Contents {count+3+index} 0 R >>".encode())
    for value in pages:
        content = value if isinstance(value, bytes) else (
            b"" if value is None else b"BT /F1 12 Tf 20 60 Td (" + value.encode("ascii") + b") Tj ET"
        )
        body = zlib.compress(content) if compressed else content
        objects.append(b"<< /Length " + str(len(body)).encode()
                       + (b" /Filter /FlateDecode" if compressed else b"") + b" >>\nstream\n" + body + b"\nendstream")
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    if image:
        objects.append(b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1"
                       b" /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\x55\nendstream")
    trailer = b""
    if encrypted:
        token = b"11" * 32
        objects.append(b"<< /Filter /Standard /V 1 /R 2 /O <" + token + b"> /U <" + token + b"> /P -4 >>")
        trailer = f" /Encrypt {len(objects)} 0 R /ID [<31313131> <31313131>]".encode()
    raw = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    if comment_pad:
        raw.extend(b"%" + (b"p" * comment_pad) + b"\n")
    offsets = []
    for number, item in enumerate(objects, 1):
        offsets.append(len(raw))
        raw.extend(f"{number} 0 obj\n".encode() + item + b"\nendobj\n")
    startxref = len(raw)
    raw.extend(f"xref\n0 {len(offsets)+1}\n".encode() + b"0000000000 65535 f \n")
    raw.extend(b"".join(f"{offset:010d} 00000 n \n".encode() for offset in offsets))
    raw.extend(b"trailer\n<< /Size " + str(len(offsets)+1).encode() + b" /Root 1 0 R"
               + trailer + b" >>\nstartxref\n" + str(startxref).encode() + b"\n%%EOF\n")
    return bytes(raw)


def report(source, page, count, status, content="", truncated=False):
    return dict(ok=True, page=page, pageCount=count, sourceByteCount=len(source), status=status,
                content=content, truncated=truncated)


def fixtures():
    plain = pdf(["First page only", "Second page fixture"])
    yield "selected-second-page", plain, {"page":2}, MIME, report(plain, 2, 2, "text", "Second page fixture\n")
    compressed = pdf(["First page only", "Second page fixture"], compressed=True)
    yield "flate-second-page", compressed, {"page":2}, MIME, report(compressed, 2, 2, "text", "Second page fixture\n")
    blank = pdf(["Only first", None])
    yield "blank-page", blank, {"page":2}, MIME, report(blank, 2, 2, "blank")
    image = pdf([b"q 100 0 0 100 0 0 cm /Im1 Do Q"], image=True)
    yield "image-without-text", image, {"page":1}, MIME, report(image, 1, 1, "image-without-text")
    vector = pdf([b"0 0 m 50 50 l S"])
    yield "vector-without-text", vector, {"page":1}, MIME, report(vector, 1, 1, "no-extractable-text")
    text = "A" * (MAX_TEXT + 80)
    long = pdf([text])
    yield "text-preview-bound", long, {"page":1}, MIME, report(long, 1, 1, "text", text[:MAX_TEXT], True)
    padding = MAX_BYTES - len(plain) - 2
    for _ in range(6):
        boundary = pdf(["First page only", "Second page fixture"], comment_pad=padding)
        if len(boundary) == MAX_BYTES:
            break
        padding += MAX_BYTES - len(boundary)
    assert len(boundary) == MAX_BYTES
    yield "exact-128kib", boundary, {"page":2}, MIME, report(boundary, 2, 2, "text", "Second page fixture\n")
    yield "over-128kib", boundary + b"x", {"page":2}, MIME, "INPUT_TOO_LARGE"
    # This fixture declares the standard encryption dictionary; no decryption is attempted.
    yield "encrypted-container-marker", pdf(["Inaccessible"], encrypted=True), {"page":1}, MIME, "ENCRYPTED_PDF"
    yield "broken-pdf", b"%PDF-1.4\ntruncated", {"page":1}, MIME, "MALFORMED_PDF"
    yield "plain-text", b"Not a PDF", {"page":1}, MIME, "MALFORMED_PDF"
    yield "nonexistent-page", plain, {"page":3}, MIME, "PAGE_NOT_FOUND"
    yield "nonpositive-page", plain, {"page":0}, MIME, "PAGE_NOT_FOUND"
    yield "wrong-mime", plain, {"page":1}, "image/png", "UNSUPPORTED_MIME"
    yield "undeclared-option", plain, {"page":1,"other":True}, MIME, "INVALID_REQUEST"
    yield "more-than-32-pages", pdf([None]*(MAX_PAGES+1)), {"page":1}, MIME, "PAGE_LIMIT"


def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module), sha256=digest(module)), resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + module + payload


def invoke(command, request):
    result = subprocess.run(command, input=request, capture_output=True, timeout=9,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if result.returncode:
        raise RuntimeError(f"exit={result.returncode}, {result.stdout[:1200]!r}, {result.stderr[-1500:]!r}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    module = args.module.read_bytes()
    results = []
    with TemporaryDirectory() as temp:
        file = Path(temp) / "owned-fixture.pdf"
        for case, payload, inputs, mime, expected in fixtures():
            file.write_bytes(payload)
            native, wire = envelopes(module, payload, inputs, mime)
            direct = invoke([args.native, file], native)
            try:
                worker = invoke([args.worker, "--zicode-plugin-worker"], wire)
            except RuntimeError as error:
                raise RuntimeError(f"{case}: {error}") from error
            assert worker["ok"], (case, worker)
            output = worker["result"]
            assert output == direct, (case, "native/WASI mismatch")
            if isinstance(expected, str):
                assert not output["ok"] and output["error"]["code"] == expected, (case, output)
            else:
                assert output == expected, (case, output)
            serialized = compact(output)
            assert len(serialized) <= MAX_RESULT
            results.append(dict(case=case, inputSha256=digest(payload), inputBytes=len(payload),
                                input=inputs, mime=mime, ok=output["ok"], resultSha256=digest(serialized)))
            print(case, "passed", flush=True)
    receipt = dict(passed=len(results), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=results)
    args.evidence.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
