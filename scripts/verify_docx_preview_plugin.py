"""Compare fixed synthetic DOCX vectors in native and production Windows Worker."""

import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo

PLUGIN = "com.zicode.devtools.docx-preview"
CAPABILITY = "devtools.docx.preview.chunks"
PROFILE = "resource.wasi.input.draft2"
MAX_FILE = 128 * 1024
MAX_XML = 128 * 1024
MAX_CHUNKS = 8
MAX_TEXT = 6 * 1024
MAX_RESULT = 48 * 1024
MIME = "application/octet-stream"
NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS_PACKAGE = "http://schemas.openxmlformats.org/package/2006/relationships"
TYPE_LIST = (
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/word/document.xml" '
    'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>'
)
RELS = (
    f'<Relationships xmlns="{NS_PACKAGE}"><Relationship Id="rId1" '
    'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
    'Target="word/document.xml"/></Relationships>'
)


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def w_paragraph(text):
    return f'<w:p><w:r><w:t xml:space="preserve">{escape(text)}</w:t></w:r></w:p>'


def document(body, prefix=""):
    return f'<?xml version="1.0" encoding="UTF-8"?>{prefix}<w:document xmlns:w="{NS}"><w:body>{body}</w:body></w:document>'.encode()


def archive(xml, extras=()):
    file = BytesIO()
    with ZipFile(file, "w", ZIP_DEFLATED) as package:
        def write(name, data, compression=ZIP_DEFLATED):
            entry = ZipInfo(name, date_time=(2026, 11, 25, 0, 0, 0))
            package.writestr(entry, data, compress_type=compression)

        write("[Content_Types].xml", TYPE_LIST)
        write("_rels/.rels", RELS)
        if xml is not None:
            write("word/document.xml", xml)
        for name, contents in extras:
            write(name, contents, ZIP_STORED)
    return file.getvalue()


def report(source, xml, sections):
    all_chunks = []
    for number, content in sections:
        start = 0
        while start < len(content):
            end = min(start + 800, len(content))
            text = content[start:end]
            all_chunks.append(dict(location=f"段落 {number}", ordinal=len(all_chunks) + 1, text=text,
                                   sha256=digest(text.encode())))
            if end == len(content):
                break
            start = end - 100
    shown = []
    budget = MAX_TEXT
    for row in all_chunks[:MAX_CHUNKS]:
        budget -= len(row["text"].encode())
        if budget < 0:
            break
        shown.append(row)
    return dict(ok=True, sourceByteCount=len(source), xmlByteCount=len(xml), nonEmptyParagraphCount=len(sections),
                totalChunks=len(all_chunks), chunkChars=800, overlapChars=100,
                truncated=len(shown)<len(all_chunks), chunks=shown)


def fixtures():
    body = ('<w:p><w:r><w:t>A &amp; B</w:t><w:tab/><w:t>中文</w:t><w:br/><w:t>继续</w:t></w:r></w:p>'
            '<w:p/><w:p></w:p>' + w_paragraph("  最后一段  "))
    xml = document(body)
    source = archive(xml, [
        ("word/_rels/document.xml.rels", b"<not accessed"),
        ("word/vbaProject.bin", b"synthetic bytes only"),
    ])
    # The current paragraph counter follows explicit closing elements. A self-closing
    # empty paragraph contributes no text or position, preserving the native reader.
    yield "paragraphs-literal-and-ignored-parts", source, {}, MIME, report(
        source, xml, [(1, "A & B\t中文\n继续"), (3, "最后一段")],
    )

    long = "中" * 900
    xml = document(w_paragraph(long))
    source = archive(xml)
    yield "deterministic-800-and-100", source, {}, MIME, report(source,xml,[(1,long)])

    sections = [(i, f"Title {i}") for i in range(1, 13)]
    xml = document("".join(w_paragraph(text) for _,text in sections))
    source = archive(xml)
    yield "preview-row-limit", source, {}, MIME, report(source,xml,sections)

    sections = [(i, "界" * 800) for i in range(1, 4)]
    xml = document("".join(w_paragraph(text) for _,text in sections))
    source = archive(xml)
    yield "preview-byte-limit", source, {}, MIME, report(source,xml,sections)

    xml = document("<w:p/>")
    source = archive(xml)
    yield "valid-empty-document", source, {}, MIME, report(source,xml,[])

    xml = document(w_paragraph("One"))
    extra_file = "opaque.bin"
    base_length = len(archive(xml,[(extra_file,b"")]))
    boundary = archive(xml,[(extra_file,b"x" * (MAX_FILE-base_length))])
    assert len(boundary) == MAX_FILE
    yield "exact-file-boundary", boundary, {}, MIME, report(boundary,xml,[(1,"One")])
    yield "file-oversized", boundary+b"x", {}, MIME, "INPUT_TOO_LARGE"

    plain = document(w_paragraph("One") + "<!--" + "-->")
    room = MAX_XML-len(plain)
    xml = document(w_paragraph("One") + "<!--" + ("x"*room) + "-->")
    source = archive(xml)
    assert len(xml) == MAX_XML and len(source) < MAX_FILE
    yield "exact-expanded-body-boundary", source, {}, MIME, report(source,xml,[(1,"One")])
    yield "expanded-body-oversized", archive(xml+b" "), {}, MIME, "XML_TOO_LARGE"

    count_bad = archive(document(w_paragraph("One")),[(f"x/{i}",b"") for i in range(254)])
    yield "entry-count-oversized", count_bad, {}, MIME, "ARCHIVE_ENTRIES_EXCEEDED"
    yield "ordinary-invalid-zip", b"PK\x03\x04not a package", {}, MIME, "INVALID_ARCHIVE"
    yield "missing-word-body", archive(None), {}, MIME, "MISSING_DOCUMENT_BODY"
    yield "truncated-word-xml", archive(document(w_paragraph("One"))[:-8]), {}, MIME, "INVALID_DOCUMENT_XML"
    yield "wrong-xml-root", archive(b"<unrelated></unrelated>"), {}, MIME, "INVALID_DOCUMENT_XML"
    yield "doctype-is-unsupported", archive(document("", "<!DOCTYPE document>")), {}, MIME, "INVALID_DOCUMENT_XML"
    yield "wrong-mime", boundary, {}, "text/plain", "UNSUPPORTED_MIME"
    yield "unexpected-input", boundary, {"shape": True}, MIME, "INVALID_REQUEST"


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
        file = Path(temp) / "owned-fixture.docx"
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
