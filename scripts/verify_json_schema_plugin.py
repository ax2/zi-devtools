"""Verify the JSON Schema candidate with an existing production Windows Worker."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import subprocess
from time import perf_counter

PLUGIN = "com.zicode.devtools.json-schema"
CAPABILITY = "devtools.json-schema.validate"
IPC_LIMIT = 48 * 1024
FIELD_LIMIT = 8192


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def digest(data):
    return sha256(data).hexdigest()


def vectors():
    for draft in (4, 6, 7):
        schema = {
            "$schema": f"http://json-schema.org/draft-0{draft}/schema#",
            "definitions": {"amount": {"type": "integer"}},
            "properties": {"项目/名~": {"$ref": "#/definitions/amount"}},
            "required": ["项目/名~"],
        }
        yield f"draft-{draft}-valid", '{"项目/名~":18446744073709551615}', schema, "校验通过", True
        yield f"draft-{draft}-invalid", '{"项目/名~":"private-fixture"}', schema, "/项目~1名~0", True
    yield "draft-7-default", '"value"', {"if": {"type": "string"}, "then": {"minLength": 6}}, "校验失败", True
    yield "pattern", '"abc12"', {"pattern": "^[a-z]+[0-9]+$"}, "校验通过", True
    yield "invalid-meta", "{}", {"type": "nonsense"}, "VALIDATION_FAILED", False
    yield "malformed-json", "{", {}, "VALIDATION_FAILED", False
    for draft in ("2019-09", "2020-12"):
        yield f"unsupported-{draft}", "1", {"$schema": f"https://json-schema.org/draft/{draft}/schema"}, "Draft 4, 6 and 7", False
    yield "unsupported-nested", "{}", {"properties": {"x": {"$schema": "https://json-schema.org/draft/2020-12/schema"}}}, "Draft 4, 6 and 7", False
    yield "recursive-reference", '{"kids":[{"kids":[]}]}', {
        "definitions": {"node": {"type": "object", "properties": {
            "kids": {"type": "array", "items": {"$ref": "#/definitions/node"}}
        }}}, "$ref": "#/definitions/node"
    }, "校验通过", True
    for name, ref in (("relative", "other.json"), ("file", "file:///fixture/other.json"),
                      ("network", "https://example.invalid/schema.json")):
        yield f"external-{name}", "1", {"$ref": ref}, "已拒绝引用", False
    yield "const-literal", '{"$ref":"https://example.invalid/data"}', {"const": {"$ref": "https://example.invalid/data"}}, "校验通过", True
    yield "max-errors", json.dumps([0] * 101), {"type": "array", "items": {"type": "string"}}, "错误超过 100 条", True
    yield "long-error-path", {"x" * 2000: [0 for _ in range(12)]}, {
        "additionalProperties": {"type": "array", "items": {"type": "string"}}
    }, "校验报告已达到显示上限", True
    yield "field-exact", '"' + "x" * (FIELD_LIMIT - 2) + '"', {"type": "string"}, "校验通过", True
    yield "instance-over-limit", " " * (FIELD_LIMIT + 1), True, "INPUT_TOO_LARGE", False
    yield "schema-over-limit", "null", "{}" + " " * (FIELD_LIMIT - 1), "INPUT_TOO_LARGE", False


def fixtures():
    for name, instance, schema, expected, ok in vectors():
        data = dict(instance=instance if isinstance(instance, str) else compact(instance).decode(), schema=schema if isinstance(schema, str) else compact(schema).decode())
        yield name, data, expected, ok


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("module", "worker", "output"):
        parser.add_argument(f"--{option}", type=Path, required=True)
    args = parser.parse_args()
    module = args.module.resolve()
    worker = args.worker.resolve()
    results = []
    for name, inputs, expected, ok in fixtures():
        request = dict(workerPath=str(module), workerKind="wasi", pluginId=PLUGIN,
                       capabilityId=CAPABILITY, sceneId="programming", input=inputs)
        started = perf_counter()
        child = subprocess.run([str(worker), "--zicode-plugin-worker"], input=compact(request),
                               capture_output=True, timeout=12, check=False,
                               creationflags=subprocess.CREATE_NO_WINDOW)
        response = json.loads(child.stdout)
        if child.returncode or not response["ok"]:
            raise RuntimeError(f"{name}: {response.get('error')}")
        result = response["result"]
        assert result["contractVersion"] == "1.0.0-rc.1" and len(compact(result)) <= IPC_LIMIT, name
        assert result["ok"] == ok, (name, result)
        if ok:
            assert expected in result["data"]["text"] and "private-fixture" not in result["data"]["text"], name
        else:
            assert result["data"] is None and (result["error"]["code"] == expected
                   or expected in result["error"]["message"]), name
        elapsed = round((perf_counter() - started) * 1000)
        results.append(dict(case=name, inputSha256=digest(compact(inputs)), resultSha256=digest(compact(result)),
                            elapsedMs=elapsed))
        print(f"{name}: {elapsed} ms", flush=True)
    args.output.write_bytes(compact(dict(wasmSha256=digest(module.read_bytes()),
                                        workerSha256=digest(worker.read_bytes()), cases=results)) + b"\n")


if __name__ == "__main__":
    main()
