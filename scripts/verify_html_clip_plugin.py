"""Exercise the saved-HTML extractor using synthetic files and the production Windows Worker."""

import argparse
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.local-html"
CAPABILITY = "devtools.html.local.preview"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "text/html"
MAX_FILE = 16 * 1024
MAX_BODY = 6 * 1024
MAX_MARKUP = 256
MAX_REFERENCES = 512
MAX_TAG_SEPARATORS = 128
MAX_DOCUMENT_SEPARATORS = 768
MAX_RESULT = 16 * 1024


def html(body, title="日报"):
    return f"<!doctype html><title>{title}</title>{body}".encode()


def preview(title, body):
    cutoff = body.encode()[:MAX_BODY].decode(errors="ignore")
    text = f"{title or '无标题'}\n\n{cutoff}"
    if cutoff != body:
        text += "\n\n[已省略剩余正文]"
    return dict(title=title, bodyByteLength=len(body.encode()),
                previewByteLength=len(cutoff.encode()), truncated=cutoff != body,
                text=text)


def fixtures():
    yield "title-and-entities", html("<p>今日 &amp; 明日 &#x2600;</p>"), {}, MIME, preview("日报", "今日 & 明日 ☀")
    yield "utf8-bom", b"\xef\xbb\xbf" + html("<main>中文正文</main>"), {}, MIME, preview("日报", "中文正文")
    yield "without-title", b"<p>untitled</p>", {}, MIME, preview("", "untitled")
    payload = html("""<nav>hidden-nav</nav><header>hidden-header</header><article>
                   <p>公开 <span>阅读</span></p><script src="https://example.invalid/script">hidden-script</script>
                   <style>hidden-style</style><aside>hidden-aside</aside><template>hidden-template</template>
                   <img src="https://example.invalid/image" onerror="hidden-event"><a href="https://example.invalid/link">链接标签</a>
                   </article><footer>hidden-footer</footer><iframe src="https://example.invalid/frame"></iframe>""")
    yield "skips-markup-and-external-sources", payload, {}, MIME, preview("日报", "公开 阅读 链接标签")
    yield "escaped-markup-is-text", html("<p>&lt;em&gt;Hello&lt;/em&gt;</p>"), {}, MIME, preview("日报", "<em>Hello</em>")
    yield "html5-malformed-shape", html("<main><p>第一<p>第二</main>"), {}, MIME, preview("日报", "第一 第二")
    yield "traditional-bare-text", b"only text", {}, MIME, preview("", "only text")
    yield "html-no-visible-body", html("<script>only</script>"), {}, MIME, "INVALID_HTML"
    yield "empty-html", b"", {}, MIME, "INVALID_HTML"
    yield "invalid-utf8", b"<p>\xff</p>", {}, MIME, "INVALID_UTF8"
    yield "wrong-mime", html("<p>Hello</p>"), {}, "application/octet-stream", "UNSUPPORTED_MIME"
    yield "unapproved-option", html("<p>Hello</p>"), {"url": "https://example.invalid/"}, MIME, "INVALID_REQUEST"
    yield "title-scalar-limit", html("<p>Hello</p>", "星" * 181), {}, MIME, preview("星" * 180, "Hello")
    clipped = "X" + "星" * 2200
    yield "truncated-on-utf8-boundary", html(f"<p>{clipped}</p>"), {}, MIME, preview("日报", clipped)
    exact = html("<p>Only this text</p>").ljust(MAX_FILE, b" ")
    yield "exact-file-size", exact, {}, MIME, preview("日报", "Only this text")
    yield "one-byte-over", exact + b" ", {}, MIME, "INPUT_TOO_LARGE"
    # Keep budget-shape tests independent of clipped output. They must still parse the entire source.
    tag_count = 2000
    yield "dense-siblings-at-limit", html("<i>x</i>" * tag_count).ljust(MAX_FILE, b" "), {}, MIME, "HTML_TOO_COMPLEX"
    depth = 1100
    payload = html("<div>" * depth + "inner" + "</div>" * depth).ljust(MAX_FILE, b" ")
    yield "deeply-nested-at-limit", payload, {}, MIME, "HTML_TOO_COMPLEX"
    dense = "&amp;" * 3000
    yield "entities-at-limit", html(f"<p>{dense}</p>").ljust(MAX_FILE, b" "), {}, MIME, "HTML_TOO_COMPLEX"


    references = "&amp;" * MAX_REFERENCES
    title_marks = html("").count(b"<")
    unclosed = "<b>" * (MAX_MARKUP - title_marks) + references
    capped = html(unclosed).ljust(MAX_FILE, b" ")
    assert capped.count(b"<") == MAX_MARKUP and capped.count(b"&") == MAX_REFERENCES
    yield "formatting-at-both-marker-limits", capped, {}, MIME, preview("日报", "&" * MAX_REFERENCES)
    yield "one-markup-marker-over", html("<b>" + unclosed), {}, MIME, "HTML_TOO_COMPLEX"
    yield "one-reference-marker-over", html(unclosed + "&amp;"), {}, MIME, "HTML_TOO_COMPLEX"
    depth_at_limit = (MAX_MARKUP - title_marks) // 2
    nested = "<div>" * depth_at_limit + references + "</div>" * depth_at_limit
    yield "nested-at-marker-budgets", html(nested).ljust(MAX_FILE, b" "), {}, MIME, preview("日报", "&" * MAX_REFERENCES)
    mixed = "<div>" * 100 + ("<i>x</i>" * 76) + references
    yield "mixed-depth-and-siblings", html(mixed).ljust(MAX_FILE, b" "), {}, MIME, preview("日报", " ".join(["x"] * 76 + ["&" * MAX_REFERENCES]))
    def attributes(count, value="A > B / inline=value and spaces"):
        return "".join(f" prop{number:03}='{value}'" for number in range(count))
    at_tag_budget = f"<p{attributes(MAX_TAG_SEPARATORS // 2)}>"
    yield "tag-attribute-budget-and-quoted-delimiters", html(at_tag_budget + "readable").ljust(MAX_FILE, b" "), {}, MIME, preview("日报", "readable")
    yield "one-tag-separator-over", html(at_tag_budget[:-1] + " data-accepted>readable"), {}, MIME, "HTML_TOO_COMPLEX"
    yield "many-distinct-attributes-with-little-markup", html("<p" + attributes(180) + ">text"), {}, MIME, "HTML_TOO_COMPLEX"
    yield "boolean-attribute-boundary", html("<p" + "".join(f" data-test{n}" for n in range(MAX_TAG_SEPARATORS)) + ">text"), {}, MIME, preview("日报", "text")
    yield "one-boolean-attribute-over", html("<p" + "".join(f" data-test{n}" for n in range(MAX_TAG_SEPARATORS + 1)) + ">text"), {}, MIME, "HTML_TOO_COMPLEX"
    # Title close slash plus 767 potential attribute separators in this document.
    aggregate = ("<p" + attributes(63, "x") + ">x") * 6 + "<p data-flag" + attributes(5, "x") + ">tail"
    at_aggregate = ("<title>日报</title>" + aggregate).encode().ljust(MAX_FILE, b" ")
    assert len(at_aggregate) == MAX_FILE
    yield "aggregate-attribute-budget", at_aggregate, {}, MIME, preview("日报", "x x x x x x tail")
    yield "one-document-separator-over", ("<title>日报</title>" + aggregate.replace(" data-flag", " data-extra data-flag")).encode(), {}, MIME, "HTML_TOO_COMPLEX"
    long_form = ("这里是正文。The report contains rendered paragraphs. " * 350).encode()
    clipped_input = long_form[:MAX_FILE-len(html("<p></p>"))].decode(errors="ignore")
    yield "ordinary-long-chinese-english-body", html(f"<p>{clipped_input}</p>").ljust(MAX_FILE, b" "), {}, MIME, preview("日报", clipped_input.strip())


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
        local = Path(folder) / "synthetic.html"
        for name, payload, options, mime, expected in fixtures():
            local.write_bytes(payload)
            direct, wire = envelopes(module, payload, options, mime)
            if isinstance(expected, str):
                result = dict(contractVersion=CONTRACT, ok=False, data=None,
                              error=dict(code=expected))
            else:
                result = dict(contractVersion=CONTRACT, ok=True,
                              data=dict(expected, byteLength=len(payload)), error=None)
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
