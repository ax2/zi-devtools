"""Check fixed one-dimensional statistics through the native adapter and production Windows Worker."""

import argparse
from fractions import Fraction
import json
import math
from pathlib import Path
import random

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.numeric-statistics"
PREFIX = "devtools.statistics."
CONTRACT = "1.0.0-rc.1"
MAX_RESULT = 1024


def values(text, precision=None):
    return {"values": text} if precision is None else {"values": text, "precision": precision}


def fixtures():
    max_int = 2**127 - 1
    integers = ",".join(map(str, range(1, 65)))
    offsets = "1000000000000,1000000000001,1000000000002"
    square = "1,2,3"
    shuffle = random.Random(29)
    fib = [1, 1]
    for _ in range(182):
        fib.append(fib[-1] + fib[-2])
    long_fractions = [Fraction(fib[i], fib[i-1]) for i in range(len(fib)-49,len(fib))]
    shuffle.shuffle(long_fractions)
    yield "exact-sum-fractions-and-decimals", "sum", values("1/3, 0.1, 1/6, -0.2"), (4,"2/5")
    yield "exact-mean-single-element", "mean", values("-7/3"), (1,"-7/3")
    yield "exact-mean-mixed-delimiters", "mean", values("1/3\t2/3\r\n1\r\n"), (3,"2/3")
    yield "exact-odd-median-unsorted", "median", values("3, 0, -2, 1/3, -1/6"), (5,0)
    yield "exact-even-median", "median", values("5,1,2,4"), (4,3)
    yield "exact-median-close-fractions", "median", values(",".join(map(str,long_fractions))), (49,sorted(long_fractions)[24])
    yield "exact-median-close-negative-fractions", "median", values(",".join(map(str,[-value for value in long_fractions]))), (49,-sorted(long_fractions)[24])
    yield "exact-large-distant-values-sort", "median", values(f"{max_int}, -{max_int}, 1/4"), (3,"1/4")
    yield "exact-large-average-without-sum-overflow", "mean", values(f"{max_int},{max_int}"), (2,max_int)
    yield "exact-max-values-sum", "sum", values(integers), (64,2080)
    yield "exact-max-values-mean", "mean", values(integers), (64,"65/2")
    yield "exact-max-values-median", "median", values(integers), (64,"65/2")
    yield "exact-max-values-population-variance", "population_variance", values(integers), (64,"1365/4")
    yield "exact-max-values-sample-variance", "sample_variance", values(integers), (64,"1040/3")
    yield "exact-max-values-population-deviation", "population_standard_deviation", values(integers), (64,math.sqrt(1365/4))
    yield "exact-max-values-sample-deviation", "sample_standard_deviation", values(integers), (64,math.sqrt(1040/3))
    yield "exact-population-variance", "population_variance", values(square), (3,"2/3")
    yield "exact-sample-variance", "sample_variance", values(square), (3,1)
    yield "minimum-population-variance", "population_variance", values("1/3"), (1,0)
    yield "minimum-sample-variance", "sample_variance", values("1/3,2/3"), (2,"1/18")
    yield "minimum-population-deviation", "population_standard_deviation", values("1/3"), (1,0.0)
    yield "minimum-sample-deviation", "sample_standard_deviation", values("1,3"), (2,math.sqrt(2))
    yield "population-deviation-is-approximate", "population_standard_deviation", values(square), (3,math.sqrt(2/3))
    yield "sample-deviation-is-approximate", "sample_standard_deviation", values(square), (3,1.0)
    yield "exact-variance-centered-at-large-offset", "population_variance", values(offsets), (3,"2/3")
    yield "approx-variance-centered-at-large-offset", "population_variance", values(offsets,"approximate"), (3,2/3)
    yield "approx-add-decimal-and-scientific", "sum", values("0.1,2e-1","approximate"), (2,0.3)
    yield "approx-range-finite-maximum", "median", values("1.7976931348623157e308","approximate"), (1,1.7976931348623157e308)
    yield "approx-range-finite-subnormal", "median", values("5e-324","approximate"), (1,5e-324)
    yield "exact-parse-prefixed-and-signed", "sum", values("0xA,0b11,0o11,-2"), (4,20)
    yield "exact-four-kib-text", "sum", values(" "*(4096-len(integers))+integers), (64,2080)
    yield "one-value-is-not-sample-variance", "sample_variance", values("1"), "STATISTICS_OPERATION"
    yield "one-value-is-not-sample-deviation", "sample_standard_deviation", values("0","approximate"), "STATISTICS_OPERATION"
    yield "more-than-64-values", "sum", values(integers+",65"), "INVALID_COUNT"
    yield "no-values", "sum", values(""), "INVALID_VALUE"
    yield "no-blank-middle-field", "mean", values("1,,2"), "INVALID_VALUE"
    yield "no-blank-trailing-field", "mean", values("1,2,"), "INVALID_VALUE"
    yield "no-blank-last-row", "mean", values("1\n2\n\n"), "INVALID_VALUE"
    yield "zero-denominator", "mean", values("1/0"), "INVALID_VALUE"
    yield "no-expressions", "mean", values("1+2"), "INVALID_VALUE"
    yield "no-complex-values", "mean", values("1+2i"), "INVALID_VALUE"
    yield "no-auto-precision", "mean", values("1","auto"), "INVALID_INPUT"
    yield "approximately-no-symbolic-fractions", "mean", values("1/2","approximate"), "INVALID_VALUE"
    yield "approx-nan", "sum", values("NaN","approximate"), "INVALID_VALUE"
    yield "approx-infinity", "mean", values("1,-inf","approximate"), "INVALID_VALUE"
    yield "approx-exponent-overflow", "mean", values("1e400","approximate"), "INVALID_VALUE"
    yield "approx-arithmetic-overflow", "sum", values("1e308,1e308","approximate"), "STATISTICS_OPERATION"
    yield "exact-integer-overflow", "mean", values(str(max_int+1)), "INVALID_VALUE"
    yield "exact-arithmetic-overflow", "sum", values(f"{max_int},1"), "STATISTICS_OPERATION"
    yield "exact-variance-overflow", "population_variance", values(f"{max_int},0"), "STATISTICS_OPERATION"
    yield "cell-byte-bound", "mean", values("数"*33), "INPUT_TOO_LARGE"
    yield "text-byte-bound", "mean", values("数"*1366), "INPUT_TOO_LARGE"
    yield "request-byte-bound", "mean", values("x"*16384), "INPUT_TOO_LARGE"
    yield "missing-input-values", "mean", {}, "INVALID_INPUT"
    yield "unexpected-operand", "mean", {"values":"1","b":"2"}, "INVALID_INPUT"
    yield "unsupported-operation", "mode", {}, "UNKNOWN_OPERATION"


def check(result, expected):
    assert result["contractVersion"] == CONTRACT
    if isinstance(expected,str):
        assert not result["ok"] and result["error"]["code"] == expected, result
        return
    count, answer = expected
    assert result["ok"] and result["error"] is None, result
    data = result["data"]
    assert data["count"] == count
    if isinstance(answer,float):
        assert data["approximate"] and data["value"]["kind"] == "approx"
        assert data["text"].startswith("≈ ")
        assert math.isclose(float(data["value"]["value"]),answer,rel_tol=1e-12),data
    else:
        answer=Fraction(answer)
        assert data["approximate"] is False
        assert data["value"]==dict(kind="exact",numerator=str(answer.numerator),denominator=str(answer.denominator))
        assert Fraction(data["text"]) == answer


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    settings=parser.parse_args()
    passed = []
    failed = []
    for name, operation, inputs, expected in fixtures():
        request=dict(pluginId=PLUGIN,sceneId="programming",capabilityId=PREFIX+operation,commandId=None,input=inputs)
        try:
            native=invoke([settings.native],compact(request))
            worker=invoke([settings.worker,"--zicode-plugin-worker"],
                          compact(dict(workerKind="wasi",workerPath=str(settings.module),**request)))
            assert worker["ok"],worker
            result=worker["result"]
            assert result==native,("native/WASI mismatch",native,result)
            check(result,expected)
            encoded=compact(result)
            assert len(encoded)<=MAX_RESULT
            passed.append(dict(case=name,capabilityId=PREFIX+operation,inputSha256=digest(compact(inputs)),
                               ok=result["ok"],resultSha256=digest(encoded),resultBytes=len(encoded)))
            print(name,"passed",flush=True)
        except Exception as problem:
            failed.append(dict(case=name,message=repr(problem)))
            print(name,repr(problem)[:750],flush=True)
    receipt=dict(passed=len(passed),wasmSha256=digest(settings.module.read_bytes()),
                 workerSha256=digest(settings.worker.read_bytes()),workerProfile="compute.wasi.v1",
                 cases=passed,failures=failed)
    settings.evidence.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    if failed:
        raise SystemExit(f"{len(failed)} conformance cases failed")


if __name__ == "__main__":
    main()
