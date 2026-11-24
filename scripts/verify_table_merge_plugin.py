"""Verify independent table formats in native Rust and the production Windows Worker."""
import argparse
import csv
from hashlib import sha256
import io
import itertools
import json
from pathlib import Path
import subprocess
from time import perf_counter

PLUGIN = "com.zicode.devtools.table-merge"
PREFIX = "devtools.table-merge."
MODES = ("left", "inner", "append", "columns")
FORMATS = ("csv", "tsv", "json")
FIELD_BYTES = 8192
IPC_BYTES = 48 * 1024


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def digest(value):
    return sha256(value).hexdigest()


def encode(format, headers, rows):
    if format == "json":
        return compact(dict(headers=headers, rows=rows)).decode()
    buffer = io.StringIO()
    writer = csv.writer(buffer, delimiter="," if format == "csv" else "\t", lineterminator="\n")
    writer.writerow(headers)
    writer.writerows(rows)
    return buffer.getvalue()


def inputs(left, right, left_format, right_format, **keys):
    return dict(left=left, right=right, leftFormat=left_format, rightFormat=right_format, **keys)


def fixtures():
    for left_format, right_format, mode in itertools.product(FORMATS, FORMATS, MODES):
        left = encode(left_format, ["id", "label"], [["match", "左"]])
        right = encode(right_format, ["label", "id"], [["右", "match"]])
        data = inputs(left, right, left_format, right_format)
        if mode in ("left", "inner"):
            data.update(leftKey="id", rightKey="id")
        table = (dict(headers=["id", "label"], rows=[["match", "左"], ["match", "右"]])
                 if mode == "append" else
                 dict(headers=["id", "label", "label_right1", "id_right1"],
                      rows=[["match", "左", "右", "match"]]))
        yield f"{left_format}-{right_format}-{mode}", mode, data, table

    data = inputs("id,label\n1,CSV\n2,missing", '[{"id":1,"value":1},{"id":"1","value":{"n":18446744073709551615}}]',
                  "csv", "json", leftKey="id", rightKey="id")
    yield "typed-keys-array-left", "left", data, dict(headers=["id", "label", "id_right1", "value"],
                                                     rows=[["1", "CSV", "1", {"n": 18446744073709551615}],
                                                           ["2", "missing", None, None]])
    yield "json-array-then-tsv", "append", inputs('[{"value":-9223372036854775808},{"value":9223372036854775807}]',
              "value\n001", "json", "tsv"), dict(headers=["value"], rows=[[-9223372036854775808], [9223372036854775807], ["001"]])
    yield "typed-keys-no-match", "inner", inputs("id\tlabel\n1\tleft", '[{"id":1}]', "tsv", "json"), dict(
        headers=["id", "label", "id_right1"], rows=[])
    yield "quoted-delimiters", "columns", inputs('左,备注\n"a,b","双""引号"', '右\t文本\n"a\tb"\t换行', "csv", "tsv"), dict(
        headers=["左", "备注", "右", "文本"], rows=[["a,b", '双"引号', "a\tb", "换行"]])
    yield "exact-spaced-keys", "inner", inputs(" id ,id\nexact,left", '[{" id ":"exact","id":"right"}]',
              "csv", "json", leftKey=" id ", rightKey=" id "), dict(
        headers=[" id ", "id", " id _right1", "id_right1"], rows=[["exact","left","exact","right"]])
    yield "trimmed-key-fallback", "inner", inputs("id\nexact", '[{"id":"exact"}]',
              "csv", "json", leftKey=" id ", rightKey=" id "), dict(
        headers=["id","id_right1"], rows=[["exact","exact"]])
    yield "exact-field-boundary", "append", inputs("id\n"+"x"*(FIELD_BYTES-3), '[{"id":"last"}]', "csv", "json"), dict(
        headers=["id"], rows=[["x"*(FIELD_BYTES-3)], ["last"]])
    yield "left-byte-overflow", "append", inputs("id\n"+"x"*(FIELD_BYTES-2), "id\n1", "csv", "tsv"), "每张表最多 8 KiB"
    yield "right-byte-overflow", "append", inputs("id\n1", "id\n"+"中"*2730, "csv", "tsv"), "每张表最多 8 KiB"
    yield "unsupported-format", "append", inputs("id\n1", "id\n2", "xml", "csv"), "请求格式或表格格式无效"
    yield "duplicate-column", "columns", inputs("id,id\n1,2", '[{"ref":1}]', "csv", "json"), "左表无法解析"
    yield "invalid-json", "columns", inputs("id\n1", '{"headers":[],"rows":[]}', "csv", "json"), "右表无法解析"
    yield "missing-left-key", "inner", inputs("id\n1", '[{"id":"1"}]', "csv", "json", leftKey="missing"), "左表不存在指定关联列"
    yield "wrong-append-headers", "append", inputs("id\n1", '[{"different":"1"}]', "csv", "json"), "列名集合相同"
    yield "wrong-column-row-count", "columns", inputs("id\n1", '[{"ref":1},{"ref":2}]', "csv", "json"), "行数相同"
    left = encode("csv", ["id", "v"], [["same", "a"*160] for _ in range(40)])
    right = encode("tsv", ["id", "w"], [["same", "b"*160] for _ in range(40)])
    yield "bounded-join-expansion", "left", inputs(left, right, "csv", "tsv"), "合并结果超出工具结果上限"


def run(args):
    worker_path = args.worker.resolve()
    module_path = args.module.resolve()
    native_path = args.native.resolve()
    records = []
    for case, mode, data, expected in fixtures():
        capability = PREFIX + mode
        request = dict(pluginId=PLUGIN, sceneId="programming", capabilityId=capability, input=data)
        native = subprocess.run([str(native_path)], input=compact(dict(request, commandId=None)),
                                capture_output=True, check=True, timeout=5, creationflags=subprocess.CREATE_NO_WINDOW)
        started = perf_counter()
        proc = subprocess.run([str(worker_path), "--zicode-plugin-worker"],
                              input=compact(dict(request, workerPath=str(module_path), workerKind="wasi")),
                              capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
        wall_ms = round((perf_counter()-started)*1000)
        response = json.loads(proc.stdout)
        if proc.returncode or not response["ok"]:
            raise RuntimeError(f"{case}: {response.get('error')}")
        result = response["result"]
        assert result == json.loads(native.stdout), case
        assert result["contractVersion"] == "1.0.0-rc.1" and len(compact(result)) <= IPC_BYTES, case
        if isinstance(expected, dict):
            assert result["ok"] and result["error"] is None, case
            outcome = json.loads(result["data"]["text"])
            assert outcome["data"] == expected, case
            assert isinstance(outcome["csv"], str) and outcome["report"], case
        else:
            assert not result["ok"] and result["data"] is None and expected in result["error"]["message"], (case, result)
        records.append(dict(case=case, operation=capability, inputSha256=digest(compact(data)),
                            outputSha256=digest(compact(result)), elapsedMs=wall_ms))
        print(f"{case}: {wall_ms} ms", flush=True)
    args.output.write_bytes(compact(dict(wasmSha256=digest(module_path.read_bytes()),
                                        workerSha256=digest(worker_path.read_bytes()),
                                        nativeAdapterSha256=digest(native_path.read_bytes()), cases=records)) + b"\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("worker", "module", "native", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    run(parser.parse_args())
