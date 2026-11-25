"""Verify standalone arithmetic in the native adapter and production Windows Worker."""

import argparse
from fractions import Fraction
import json
import math
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.expression-calculator"
CAPABILITY = "devtools.calculator.evaluate"
CONTRACT = "1.0.0-rc.1"
MAX_RESULT = 1024


def args(formula, **options):
    return {"formula":formula,**options}


def fixtures():
    maximum=2**127-1
    minimum=-(2**127)
    largest_factorial=math.factorial(33)
    dense="+1"*256
    heavy_combination="0"+"+(comb(127,63)-comb(127,63))"*30
    fib=[1,1]
    for _ in range(182):
        fib.append(fib[-1]+fib[-2])
    yield "native-exact-decimal",args("0.1+0.2"),Fraction(3,10)
    yield "native-exact-fraction",args("1/3+1/6"),Fraction(1,2)
    yield "native-continued-rational",args("1/3"),Fraction(1,3)
    yield "unary-power-precedence",args("-2^2"),-4
    yield "right-associative-power",args("2^3^2"),512
    yield "negative-exact-exponent",args("2^-3"),Fraction(1,8)
    yield "ordinary-parentheses",args("(128+64)*3"),576
    yield "native-exact-scientific-notation",args("1e-3+0.002"),Fraction(3,1000)
    yield "signed-round",args("round(-1.5)"),-2
    yield "fraction-floor",args("floor(-1/3)"),-1
    yield "exact-fraction-ceil",args("ceil(7/3)"),3
    yield "radians-default",args("sin(pi/6)"),0.5
    yield "degrees-option",args("sin(30)",angle="degrees"),0.5
    yield "real-scientific-functions",args("sqrt(16)+log10(1000)+exp(0)"),8.0
    yield "full-typed-complex-sqrt",args("sqrt(-4)"),(0.0,2.0)
    yield "native-complex-multiplication",args("(2+3*i)*(2-3*i)"),(13.0,0.0)
    yield "native-complex-power",args("i^2"),(-1.0,0.0)
    yield "native-complex-real-part",args("real(conj(2+3*i))"),2.0
    yield "native-complex-imaginary-part",args("imag(conj(2+3*i))"),-3.0
    yield "native-complex-polar-phase",args("arg(1+i)",angle="degrees"),45.0
    yield "programmer-hex-bit-and",args("0xff & 0x0f"),15
    yield "programmer-binary-shift",args("0b1010 << 2"),40
    yield "programmer-octal-or",args("0o10 | 2"),10
    yield "programmer-bit-not",args("~0"),-1
    yield "programmer-128-sign-bit",args("1 << 127"),minimum
    yield "programmer-arithmetic-right-shift",args("(-8) >> 2"),-2
    yield "programmer-xor",args("xor(0xff,0x0f)"),240
    yield "programmer-remainder",args("(-17)%5"),-2
    yield "unit-length",args("5 km -> m"),5000
    yield "unit-mass",args("2 kg -> g"),2000
    yield "unit-time-expression",args("1+2 h -> s"),10800
    yield "unit-storage-exact",args("1 GiB -> MB"),Fraction(2**30,1_000_000)
    yield "unit-temperature-offset",args("32 F -> C"),0
    yield "unit-negative-temperature",args("-40 C -> F"),-40
    yield "native-exact-rational-comparison",args(f"min({maximum}/2,({maximum}-2)/3)"),Fraction(maximum-2,3)
    yield "largest-exact-integer",args(str(maximum)),maximum
    yield "smallest-exact-per-request-literal",args("value",constants={"value":str(minimum)}),minimum
    yield "explicit-fraction-and-float-constants",args("radius*turn",constants={"radius":"3/2","turn":0.25}),0.375
    yield "coordinate-is-ordinary-in-expression",args("x+1/2",constants={"x":"1/3"}),Fraction(5,6)
    yield "constants-count-64",args("v63",constants={f"v{i}":str(i) for i in range(64)}),63
    yield "formula-composes-statistics",args("mean(1/3,2/3)+comb(5,2)"),Fraction(21,2)
    yield "zero-factorial",args("fact(0)"),1
    yield "largest-exact-factorial",args("fact(33)"),largest_factorial
    yield "native-permutation",args("perm(5,3)"),60
    yield "native-poker-combination",args("comb(52,5)"),2598960
    yield "near-int128-binomial",args("comb(127,63)"),math.comb(127,63)
    yield "native-zero-selections-max-n",args("comb(10000,0)"),1
    yield "signed-gcd",args("gcd(-48,18)"),6
    yield "signed-lcm",args("lcm(-12,18)"),36
    yield "gcd-i128-minimum",args("gcd(n,2)",constants={"n":str(minimum)}),2
    yield "gcd-zero-pair",args("gcd(0,0)"),0
    yield "lcm-zero",args("lcm(0,8)"),0
    yield "dense-512-token-add",args(dense),256
    yield "max-2048-byte-formula",args(" "*(2048-len(dense))+dense),256
    yield "dense-combination-workload",args(heavy_combination),0
    yield "dense-neighboring-integer-gcd",args("+".join(["gcd(a,b)"]*73),constants={"a":str(fib[-1]),"b":str(fib[-2])}),73
    yield "native-legal-depth",args("("*63+"1"+")"*63),1
    yield "one-expression-only",args("1;2"),"EXPRESSION_OPERATION"
    yield "no-assignment",args("x=2"),"EXPRESSION_OPERATION"
    yield "no-multiple-values",args("1 2"),"EXPRESSION_OPERATION"
    yield "no-session-ans",args("ans+1"),"EXPRESSION_OPERATION"
    yield "no-unknown-name",args("missing*2"),"EXPRESSION_OPERATION"
    yield "no-unknown-function",args("unlisted(3)"),"EXPRESSION_OPERATION"
    yield "no-empty-input",args("  "),"EXPRESSION_OPERATION"
    yield "no-over-depth",args("("*64+"1"+")"*64),"EXPRESSION_OPERATION"
    yield "no-over-token-budget",args(dense+"+1"),"EXPRESSION_OPERATION"
    yield "no-over-byte-budget",args(" "*2048+"1"),"EXPRESSION_OPERATION"
    yield "non-ascii-numeral-rejected-by-native-grammar",args("２+３"),"EXPRESSION_OPERATION"
    yield "native-precise-overflow-does-not-approximate",args(f"{maximum}+1"),"EXPRESSION_OPERATION"
    yield "division-by-zero",args("1/0"),"EXPRESSION_OPERATION"
    yield "non-finite-result",args("exp(1000)"),"EXPRESSION_OPERATION"
    yield "bitwise-requires-exact-integer",args("1.5 & 1"),"EXPRESSION_OPERATION"
    yield "bitwise-shift-limit",args("1<<128"),"EXPRESSION_OPERATION"
    yield "unit-dimension-mismatch",args("5 kg -> min"),"EXPRESSION_OPERATION"
    yield "unknown-unit",args("5 XQ -> m"),"EXPRESSION_OPERATION"
    yield "native-factorial-overflow",args("fact(34)"),"EXPRESSION_OPERATION"
    yield "native-combination-input-limit",args("comb(10001,1)"),"EXPRESSION_OPERATION"
    yield "native-combination-range",args("comb(4,5)"),"EXPRESSION_OPERATION"
    yield "native-combination-overflow",args("comb(200,100)"),"EXPRESSION_OPERATION"
    yield "nonintegral-gcd",args("gcd(1/2,2)"),"EXPRESSION_OPERATION"
    yield "native-lcm-overflow",args(f"lcm({maximum},2)"),"EXPRESSION_OPERATION"
    yield "reserved-constant-name",args("pi",constants={"pi":"1"}),"INVALID_CONSTANT"
    yield "no-presented-ans-history",args("ans",constants={"ans":"2"}),"INVALID_CONSTANT"
    yield "too-many-constants",args("1",constants={f"v{i}":str(i) for i in range(65)}),"INVALID_CONSTANT"
    yield "constant-invalid-rational",args("a",constants={"a":"1/0"}),"INVALID_CONSTANT"
    yield "constant-non-finite-string",args("a",constants={"a":"NaN"}),"INVALID_CONSTANT"
    yield "constant-string-byte-limit",args("a",constants={"a":" "*64+"1"}),"INVALID_CONSTANT"
    yield "constant-bool-is-not-a-number",args("a",constants={"a":True}),"INVALID_INPUT"
    yield "unknown-angle",args("1",angle="turns"),"INVALID_INPUT"
    yield "no-unexpected-input-property",args("1",command="run"),"INVALID_INPUT"
    yield "request-16-kib-bound",args("x"*17000),"INPUT_TOO_LARGE"


def check(result, expected):
    assert result["contractVersion"] == CONTRACT
    if isinstance(expected,str):
        assert not result["ok"] and result["error"]["code"] == expected, result
        return
    assert result["ok"] and result["error"] is None, result
    data=result["data"]
    value=data["value"]
    if isinstance(expected,tuple):
        assert data["approximate"] and value["kind"]=="complex"
        assert data["text"].startswith("≈ ")
        assert math.isclose(float(value["real"]),expected[0],rel_tol=1e-12,abs_tol=1e-12)
        assert math.isclose(float(value["imaginary"]),expected[1],rel_tol=1e-12,abs_tol=1e-12)
    elif isinstance(expected,float):
        assert data["approximate"] and value["kind"]=="approx"
        assert data["text"].startswith("≈ ")
        assert math.isclose(float(value["value"]),expected,rel_tol=1e-12,abs_tol=1e-12)
    else:
        exact=Fraction(expected)
        assert data["approximate"] is False
        assert value==dict(kind="exact",numerator=str(exact.numerator),denominator=str(exact.denominator))
        assert Fraction(data["text"])==exact


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    config=parser.parse_args()
    cases,failed=[],[]
    for name,inputs,expected in fixtures():
        request=dict(pluginId=PLUGIN,sceneId="programming",capabilityId=CAPABILITY,commandId=None,input=inputs)
        try:
            native=invoke([config.native],compact(request))
            worker=invoke([config.worker,"--zicode-plugin-worker"],
                          compact(dict(workerKind="wasi",workerPath=str(config.module),**request)))
            assert worker["ok"],worker
            result=worker["result"]
            assert result==native,("native/WASI mismatch",native,result)
            check(result,expected)
            encoded=compact(result)
            assert len(encoded)<=MAX_RESULT
            cases.append(dict(case=name,capabilityId=CAPABILITY,inputSha256=digest(compact(inputs)),
                              ok=result["ok"],resultSha256=digest(encoded),resultBytes=len(encoded)))
            print(name,"passed",flush=True)
        except Exception as error:
            failed.append(dict(case=name,message=repr(error)))
            print(name,repr(error)[:1000],flush=True)
    proof=dict(passed=len(cases),wasmSha256=digest(config.module.read_bytes()),
               workerSha256=digest(config.worker.read_bytes()),workerProfile="compute.wasi.v1",cases=cases,failures=failed)
    config.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    if failed:
        raise SystemExit(f"{len(failed)} conformance cases failed")


if __name__ == "__main__":
    main()
