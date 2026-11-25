"""Run fixed matrix cases through the native adapter and production Windows Worker."""

import argparse
from fractions import Fraction
import json
import math
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.matrix-calculator"
PREFIX = "devtools.matrix."
CONTRACT = "1.0.0-rc.1"
MAX_RESULT = 12 * 1024
BINARY = {"add", "subtract", "multiply", "solve"}


def tsv(rows):
    return "\n".join("\t".join(map(str, row)) for row in rows)


def args(a, b=None, **options):
    return dict(a=a, **options) if b is None else dict(a=a, b=b, **options)


def fixtures():
    a = "1\t2\t3\n4\t5\t6"
    b = "7\t8\n9\t10\n11\t12"
    identity = [[int(i == j) for j in range(8)] for i in range(8)]
    near_dense = [[2 + int(i == j) for j in range(8)] for i in range(8)]
    dense_inverse = [[Fraction(15 if i == j else -2, 17) for j in range(8)] for i in range(8)]
    dense_square = [[36 + int(i == j) for j in range(8)] for i in range(8)]
    huge = 2**127 - 1
    padded = " " * (4096 - len(tsv(near_dense))) + tsv(near_dense)
    long_exact = "/".join((str(huge), str(huge - 1)))
    long_cells = [[long_exact] * 8] * 6
    p, q = 1, 1
    for _ in range(182):
        p, q = q, p + q
    fibonacci = Fraction(q, p)
    yield "exact-fraction-and-decimal-add", "add", args("0.1\t1/3", "0.2\t1/6"), [["3/10", "1/2"]]
    yield "negative-exact-and-scientific-subtract", "subtract", args("-2\t1.2e-1", "3\t0.02"), [[-5, "1/10"]]
    yield "exact-rectangular-multiply", "multiply", args(a, b), [[58, 64], [139, 154]]
    yield "exact-rectangular-transpose-crlf", "transpose", args(" 1\t2\t3\r\n4\t5\t 6\r\n"), [[1,4],[2,5],[3,6]]
    yield "determinant-row-exchange", "determinant", args("0\t1\n2\t4"), [[-2]]
    yield "singular-determinant-zero", "determinant", args("1\t2\n2\t4"), [[0]]
    yield "scalar-exact", "determinant", args("-4/2"), [[-2]]
    yield "inverse-exact-fractions", "inverse", args("2\t1\n1\t2"), [["2/3", "-1/3"], ["-1/3", "2/3"]]
    yield "solve-multiple-right-hand-sides", "solve", args("2\t1\n1\t2", "3\t5\n3\t4"), [[1,2],[1,1]]
    yield "exact-integer-past-javascript-safe-range", "transpose", args(str(huge)), [[huge]]
    yield "exact-nonterminating-fraction", "transpose", args("1/3"), [["1/3"]]
    yield "exact-prefixed-and-signed-elements", "transpose", args("0x10\t+7\t-1/2"), [[16],[7],["-1/2"]]
    yield "approximate-decimal-and-scientific", "add", args("0.1\t1e-8","0.2\t2e-8",precision="approximate"), [[0.3,3e-8]]
    yield "approximate-extreme-finite-and-subnormal", "transpose", args("1.7976931348623157e308\t5e-324",precision="approximate"), [[1.7976931348623157e308],[5e-324]]
    yield "approximate-solve", "solve", args("3\t2\n1\t-1","7\n-1",precision="approximate"), [[1.0],[2.0]]
    yield "approximate-singular-determinant", "determinant", args("1\t2\n2\t4", precision="approximate"), [[0.0]]
    yield "exact-dense-eight-axis-multiply", "multiply", args(tsv(near_dense),tsv(near_dense)), dense_square
    yield "exact-dense-eight-axis-determinant", "determinant", args(tsv(near_dense)), [[17]]
    yield "exact-dense-eight-axis-inverse", "inverse", args(tsv(near_dense)), dense_inverse
    yield "exact-dense-eight-axis-and-eight-rhs", "solve", args(tsv(near_dense),tsv(identity)), dense_inverse
    yield "approximate-dense-eight-axis-and-eight-rhs", "solve", args(tsv(near_dense),tsv(identity),precision="approximate"), [[float(x) for x in row] for row in dense_inverse]
    yield "exact-four-kib-per-matrix", "add", args(padded, padded), [[2 * x for x in row] for row in near_dense]
    yield "exact-long-fractions-and-output", "transpose", args(tsv(long_cells)), [[Fraction(long_exact)] * 6] * 8
    yield "exact-long-euclidean-reduction", "transpose", args(tsv([[fibonacci] * 8] * 6)), [[fibonacci] * 6] * 8
    yield "empty-matrix", "transpose", args(""), "INVALID_MATRIX"
    yield "ragged-matrix", "transpose", args("1\t2\n3"), "INVALID_MATRIX"
    yield "blank-cell", "transpose", args("1\t\t2"), "INVALID_MATRIX"
    yield "blank-row", "transpose", args("1\n\n2"), "INVALID_MATRIX"
    yield "ninth-row", "transpose", args(tsv([[1]] * 9)), "INVALID_MATRIX"
    yield "ninth-column", "transpose", args(tsv([[1] * 9])), "INVALID_MATRIX"
    yield "add-mismatch", "add", args(a,b), "MATRIX_OPERATION"
    yield "subtract-mismatch", "subtract", args(a,b), "MATRIX_OPERATION"
    yield "multiply-mismatch", "multiply", args(a,a), "MATRIX_OPERATION"
    yield "determinant-nonsquare", "determinant", args(a), "MATRIX_OPERATION"
    yield "inverse-nonsquare", "inverse", args(a), "MATRIX_OPERATION"
    yield "solve-nonsquare-a", "solve", args(a,b), "MATRIX_OPERATION"
    yield "solve-rhs-wrong-height", "solve", args("2\t1\n1\t2",b), "MATRIX_OPERATION"
    yield "inverse-singular", "inverse", args("1\t2\n2\t4"), "MATRIX_OPERATION"
    yield "solve-singular", "solve", args("1\t2\n2\t4","1\n2"), "MATRIX_OPERATION"
    yield "solve-approximate-singular", "solve", args("1\t2\n2\t4","1\n2",precision="approximate"), "MATRIX_OPERATION"
    yield "input-i128-overflow", "transpose", args(str(huge+1)), "INVALID_CELL"
    yield "exact-intermediate-overflow", "multiply", args(str(huge),"2"), "MATRIX_OPERATION"
    yield "division-by-zero-in-cell", "transpose", args("1/0"), "INVALID_CELL"
    yield "approximate-mode-rejects-fraction-notation", "transpose", args("1/3",precision="approximate"), "INVALID_CELL"
    yield "approximate-mode-rejects-nan", "transpose", args("NaN",precision="approximate"), "INVALID_CELL"
    yield "approximate-mode-rejects-infinity", "transpose", args("inf",precision="approximate"), "INVALID_CELL"
    yield "approximate-mode-rejects-exponent-overflow", "transpose", args("1e400",precision="approximate"), "INVALID_CELL"
    yield "approximate-mode-rejects-result-overflow", "multiply", args("1e300","1e30",precision="approximate"), "MATRIX_OPERATION"
    yield "no-matrix-expressions", "transpose", args("2+2"), "INVALID_CELL"
    yield "no-complex-numbers", "transpose", args("1+2i"), "INVALID_CELL"
    yield "request-missing-a", "transpose", {"precision":"exact"}, "INVALID_INPUT"
    yield "request-missing-b", "multiply", args("1"), "INVALID_INPUT"
    yield "unexpected-b-in-unary", "transpose", args("1","2"), "INVALID_INPUT"
    yield "unsupported-precision", "transpose", args("1",precision="auto"), "INVALID_INPUT"
    yield "per-matrix-utf8-byte-limit", "transpose", args("数" * 1366), "INPUT_TOO_LARGE"
    yield "per-cell-utf8-byte-limit", "transpose", args("数" * 33), "INVALID_MATRIX"
    yield "reject-oversized-single-cell-using-plugin-limit", "transpose", args("1" * 513), "INVALID_MATRIX"
    yield "request-byte-limit", "transpose", args("x" * 16384), "INPUT_TOO_LARGE"
    yield "unknown-operation", "undefined", {}, "UNKNOWN_OPERATION"


def check(actual, expected):
    assert actual["contractVersion"] == CONTRACT
    if isinstance(expected, str):
        assert not actual["ok"] and actual["error"]["code"] == expected, actual
        return
    assert actual["ok"] and actual["error"] is None, actual
    data = actual["data"]
    approximate = isinstance(expected[0][0], float)
    assert data["approximate"] == approximate
    assert (data["rowCount"], data["columnCount"]) == (len(expected), len(expected[0]))
    assert len(data["values"]) == len(expected)
    for answer, row in zip(expected, data["values"]):
        assert len(row) == len(answer)
        for value, cell in zip(answer, row):
            if approximate:
                assert cell["kind"] == "approx" and math.isclose(float(cell["value"]), value, rel_tol=1e-12), cell
            else:
                val = Fraction(value)
                assert cell == dict(kind="exact", numerator=str(val.numerator), denominator=str(val.denominator)), cell
    pretty = data["text"]
    if approximate:
        assert pretty.startswith("≈ 近似结果\n")
        pretty = pretty.split("\n", 1)[1]
    rows = [row.split("\t") for row in pretty.split("\n")]
    assert len(rows) == len(expected)
    assert all(len(row) == len(expected[0]) for row in rows)
    if not approximate:
        assert [[Fraction(cell) for cell in row] for row in rows] == [
            [Fraction(cell) for cell in row] for row in expected
        ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    settings = parser.parse_args()
    results = []
    failed = []
    for case, operation, inputs, expected in fixtures():
        request = dict(pluginId=PLUGIN, sceneId="programming", capabilityId=PREFIX+operation,
                       commandId=None, input=inputs)
        try:
            native = invoke([settings.native], compact(request))
            worker = invoke([settings.worker, "--zicode-plugin-worker"],
                            compact(dict(workerKind="wasi", workerPath=str(settings.module), **request)))
            assert worker["ok"], worker
            actual = worker["result"]
            assert actual == native, ("native/WASI mismatch", native, actual)
            check(actual, expected)
            encoded = compact(actual)
            assert len(encoded) <= MAX_RESULT
            results.append(dict(case=case, capabilityId=PREFIX+operation, inputSha256=digest(compact(inputs)),
                                ok=actual["ok"], resultSha256=digest(encoded), resultBytes=len(encoded)))
            print(case, "passed", flush=True)
        except Exception as reason:
            failed.append(dict(case=case, message=repr(reason)))
            print(case, repr(reason)[:500], flush=True)
    receipt = dict(passed=len(results), wasmSha256=digest(settings.module.read_bytes()),
                   workerSha256=digest(settings.worker.read_bytes()), workerProfile="compute.wasi.v1",
                   cases=results, failures=failed)
    settings.evidence.write_text(json.dumps(receipt, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    if failed:
        raise SystemExit(f"{len(failed)} conformance cases failed")


if __name__ == "__main__":
    main()
