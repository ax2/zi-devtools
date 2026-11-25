"""Exercise memo text preparation through the native adapter and production Windows Worker."""

import argparse
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.memo-text"
CAPABILITY = "devtools.memo.file.preview"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_BODY = 128 * 1024
MAX_FILE = MAX_BODY + 3
MAX_PREVIEW = 6 * 1024
MAX_RESULT = 48 * 1024
BOM = b"\xef\xbb\xbf"
UNTITLED = "导入的备忘"


def preview(body, title=UNTITLED, *, bom=False):
    raw = body.encode()
    excerpt = raw[:MAX_PREVIEW].decode(errors="ignore")
    shown = len(excerpt.encode())
    truncated = shown < len(raw)
    text = f"{title}\n正文显示 {shown}/{len(raw)} 字节\n\n{excerpt or '（空白正文）'}"
    if truncated:
        text += "\n\n[已省略剩余正文]"
    return dict(title=title, byteLength=len(raw) + (len(BOM) if bom else 0),
                bodyByteLength=len(raw), previewByteLength=shown, hadBom=bom,
                truncated=truncated, text=text)


def fixtures():
    text = "# 摘要\r\n\r\n普通备忘 🧪\r\n保留末尾  \r\n"
    yield "markdown-keeps-source-bytes", text.encode(), {}, "text/markdown", preview(text)
    yield "plain-keeps-lf-tab-and-final-spaces", "一\t二\n三  \n".encode(), {}, "text/plain", preview("一\t二\n三  \n")
    yield "unknown-extension-confirmed-bytes", text.encode(), {}, MIME, preview(text)
    yield "explicit-title-normalized", text.encode(), {"title":"  Spring\t计划  "}, MIME, preview(text, "Spring计划")
    yield "whitespace-title-uses-default", text.encode(), {"title":" \t \n "}, MIME, preview(text)
    yield "explicit-title-at-120-scalar-values", text.encode(), {"title":"🧪"*120}, MIME, preview(text, "🧪"*120)
    yield "title-one-scalar-over", text.encode(), {"title":"🧪"*121}, MIME, "INVALID_REQUEST"
    yield "null-title-is-not-a-title", text.encode(), {"title":None}, MIME, "INVALID_REQUEST"
    yield "unknown-input-is-rejected", text.encode(), {"save":True}, MIME, "INVALID_REQUEST"
    yield "empty-body-valid", b"", {}, "text/plain", preview("")
    yield "bom-only-valid", BOM, {}, "text/plain", preview("", bom=True)
    yield "single-bom-stripped", BOM+"\ufeff二".encode(), {}, "text/markdown", preview("\ufeff二", bom=True)
    yield "empty-looking-whitespace-stays", b"  \r\n\t", {}, MIME, preview("  \r\n\t")
    body = "x"*(MAX_PREVIEW-1)+"星"
    yield "preview-never-splits-unicode", body.encode(), {}, MIME, preview(body)
    body = "🧪"*(MAX_PREVIEW//4)
    yield "exact-preview-limit-is-complete", body.encode(), {}, MIME, preview(body)
    body = "天"*(MAX_BODY//3)+"a"*(MAX_BODY%3)
    payload = body.encode()
    assert len(payload) == MAX_BODY
    assert payload[65536-3] & 0xc0 == 0x80
    yield "exact-body-without-bom", payload, {}, MIME, preview(body)
    yield "exact-body-plus-bom-across-worker-windows", BOM+payload, {}, "text/plain", preview(body, bom=True)
    yield "one-byte-over-without-bom", payload+b"x", {}, MIME, "INPUT_TOO_LARGE"
    yield "one-byte-over-with-bom", BOM+payload+b"x", {}, MIME, "INPUT_TOO_LARGE"
    yield "file-size-without-bom-cannot-use-bom-allowance", payload+b"xyz", {}, MIME, "INPUT_TOO_LARGE"
    yield "invalid-utf8-after-the-visible-preview", payload[:-1]+b"\xc2", {}, MIME, "INVALID_UTF8"
    yield "nul-after-the-visible-preview", BOM+payload[:-1]+b"\0", {}, MIME, "CONTAINS_NUL"
    yield "invalid-utf8-in-late-snapshot-window", b"a"*65536+b"\xff"+b"x"*64000, {}, MIME, "INVALID_UTF8"
    yield "utf16-is-not-muted", b"\xff\xfeT\0X\0T\0", {}, MIME, "INVALID_UTF8"
    yield "nul-near-start", b"Z\0", {}, "text/plain", "CONTAINS_NUL"
    yield "other-selected-resource-type", b"A", {}, "image/png", "UNSUPPORTED_MIME"
    large_escape = "\x01"*MAX_PREVIEW
    yield "reserved-json-output-size-is-enough", large_escape.encode(), {}, MIME, preview(large_escape)
    yield "link-markup-remains-plain-text", b"<a href='https://example.invalid'>label</a>", {}, MIME, preview("<a href='https://example.invalid'>label</a>")


def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module), sha256=digest(module)), resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + module + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    module = args.module.read_bytes()
    rows, failures = [], []
    with TemporaryDirectory() as folder:
        local = Path(folder) / "memo.md"
        for name, payload, options, mime, expected in fixtures():
            local.write_bytes(payload)
            direct, wire = envelopes(module, payload, options, mime)
            if isinstance(expected, str):
                result = dict(contractVersion=CONTRACT, ok=False, data=None,
                              error=dict(code=expected))
            else:
                result = dict(contractVersion=CONTRACT, ok=True,
                              data=expected, error=None)
            try:
                native = invoke([args.native, local], direct)
                worker_response = invoke([args.worker, "--zicode-plugin-worker"], wire)
                assert worker_response["ok"], (name, worker_response)
                worker = worker_response["result"]
                assert worker == native, (name, worker, native)
                if isinstance(expected, str):
                    assert worker["error"]["message"]
                    del worker["error"]["message"]
                assert worker == result, (name, worker, result)
            except Exception as error:
                failures.append(dict(case=name, error=str(error)[:1200]))
                print(f"FAIL {name} {str(error)[:220]}", flush=True)
            else:
                rows.append(dict(case=name, inputSha256=digest(payload), inputBytes=len(payload),
                                 mime=mime, ok=result["ok"]))
                print(f"PASS {name}", flush=True)
    receipt = dict(passed=len(rows), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=rows, failures=failures)
    args.evidence.write_text(json.dumps(receipt, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
