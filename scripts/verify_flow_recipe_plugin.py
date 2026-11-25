"""Verify a text-flow recipe through native code and the production Windows Worker."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.flow-recipe"
CAPABILITY = "devtools.flow.recipe.inspect"
PROFILE = "resource.wasi.input.draft2"
MIME = "application/json"
MAX_FILE = 64 * 1024
MAX_STEPS = 16
MAX_RESULT = 16 * 1024


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def recipe(version, *actions):
    steps = [
        dict(action=item, version=1) if isinstance(item, str) else item
        for item in actions
    ]
    return compact(dict(version=version, steps=steps))


def step(action, label, type_in="text", type_out="text", effect="transform", ports=()):
    return dict(action=action, version=1, label=label, input=type_in, output=type_out,
                effect=effect, sideOutputs=list(ports))


def report(version, *steps):
    return dict(ok=True, recipeVersion=version,
                input=steps[0]["input"] if steps else "text",
                output=steps[-1]["output"] if steps else "text", steps=list(steps))


def fixtures():
    minify = step("json.minify", "JSON · 压缩")
    parse = step("table.parse_csv", "CSV → 表格", "text", "table")
    export = step("table.export_schema", "表格 → 列结构JSON", "table")
    file = step("file.sha256", "文件 · SHA-256 报告", "file",
                effect="read-selected-file", ports=[dict(id="source-file", label="原始文件引用", kind="file")])
    image = step("image.info", "图片 · 信息报告", "image", effect="process-in-memory-image")
    source = recipe(1, "json.minify")
    yield "plain-text", source, {}, MIME, report(1, minify)
    yield "typed-input-output", recipe(2, "table.parse_csv", "table.export_schema"), {}, MIME, report(2, parse, export)
    yield "file-effect-and-side-output", recipe(3, "file.sha256", "json.minify"), {}, MIME, report(3, file, minify)
    yield "image-effect", recipe(4, "image.info"), {}, MIME, report(4, image)
    yield "empty-recipe", recipe(4), {}, "text/plain", report(4)
    # Both resource bytes and step count are at the advertised limits.
    at_limit = recipe(1, *["json.minify"] * MAX_STEPS).ljust(MAX_FILE, b" ")
    yield "exact-byte-and-step-limit", at_limit, {}, MIME, report(1, *[minify] * MAX_STEPS)
    yield "one-byte-over", at_limit + b" ", {}, MIME, "INPUT_TOO_LARGE"
    yield "one-step-over", recipe(1, *["json.minify"] * (MAX_STEPS + 1)), {}, MIME, "INVALID_RECIPE"
    many = recipe(4, *[dict(action="", version=0)] * 2450)
    assert len(many) <= MAX_FILE
    yield "many-small-steps", many, {}, MIME, "INVALID_RECIPE"
    yield "malformed-json", b'{"version":4,"steps":[}', {}, MIME, "INVALID_RECIPE"
    yield "invalid-utf8", b'{"version":4,"steps":[], "\xff":true}', {}, MIME, "INVALID_RECIPE"
    yield "unsupported-recipe-version", recipe(5), {}, MIME, "INVALID_RECIPE"
    yield "unsupported-action-version", recipe(1, dict(action="json.minify", version=2)), {}, MIME, "INVALID_RECIPE"
    yield "unregistered-action", recipe(4, "unregistered.operation"), {}, MIME, "INVALID_RECIPE"
    yield "incompatible-chain", recipe(2, "table.parse_csv", "json.minify"), {}, MIME, "INVALID_RECIPE"
    yield "external-file-in-recipe", b'{"version":3,"steps":[{"action":"file.sha256","version":1,"path":"other.txt"}]}', {}, MIME, "INVALID_RECIPE"
    yield "unexpected-arguments", source, {"run":True}, MIME, "INVALID_REQUEST"
    yield "wrong-mime", source, {}, "image/png", "UNSUPPORTED_MIME"


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
        file = Path(temp) / "owned-recipe.json"
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
            assert file.read_bytes() == payload
            results.append(dict(case=case, inputSha256=digest(payload), inputBytes=len(payload),
                                input=inputs, mime=mime, ok=output["ok"], resultSha256=digest(serialized)))
            print(case, "passed", flush=True)
    receipt = dict(passed=len(results), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=results)
    args.evidence.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
