"""Compare deterministic bisection cases in the native adapter and Windows Studio Worker."""

import argparse
import json
import math
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.root-solver"
CAPABILITY = "devtools.roots.bisect"
CONTRACT = "1.0.0-rc.1"
MAX_RESULT = 1024


def args(formula, lower=1, upper=2, **opts):
    return dict(formula=formula,lower=lower,upper=upper,**opts)


def result(root, accuracy=1e-10, max_residual=1e-8, iterations=None, converged=True, min_residual=0):
    return dict(root=root,accuracy=accuracy,maxResidual=max_residual,iterations=iterations,
                converged=converged,minResidual=min_residual)


def fixtures():
    dense = "+x" + "+0"*254 + "-0.3"
    fraction_chain = "x-0.3" + "+(1/433494437-1/433494437)"*50
    prefix_depth = lambda depth: "("*depth+"x-1"+")"*depth
    huge = 1_000_000_000_000
    yield "default-radical", args("x^2-2"), result(math.sqrt(2))
    yield "left-endpoint", args("x-2",2,4), result(2,0,0,0)
    yield "right-endpoint", args("x-4",2,4), result(4,0,0,0)
    yield "midpoint-exact", args("x-3",2,4), result(3,0,0,1)
    yield "one-iteration-nonconverged", args("x^2-2",maxIterations=1,tolerance=1e-12), result(1.5,0,0.25,1,False,0.25)
    yield "native-max-iterations", args("x^2-2",maxIterations=200), result(math.sqrt(2))
    yield "width-tolerance-is-not-residual", args("x-2.345",2.34,2.35,tolerance=0.01), result(2.345,0.005,0.005)
    yield "bisection-radians", args("sin(x)-0.5",0,1), result(math.pi/6)
    yield "bisection-degrees", args("sin(x)-0.5",0,50,angle="degrees"), result(30)
    yield "explicit-real-constants", args("scale*x-shift",0,2,constants={"scale":"3/2","shift":0.75}), result(0.5,0,0,2)
    yield "constants-use-exact-signed-literals", args("scale*x-shift",0,2,constants={"scale":"-3/2","shift":-0.75}), result(0.5,0,0,2)
    yield "native-64-constant-entries", args("x-v63",0,100,constants={f"v{i}":str(i) for i in range(64)}), result(63)
    yield "multiple-roots-left-bracket", args("x^2-4",-3,0), result(-2)
    yield "multiple-roots-right-bracket", args("x^2-4",0,3), result(2)
    yield "multiple-roots-with-central-match", args("x*(x^2-4)",-3,3), result(0,0,0,1)
    yield "no-global-root-search-for-even-bracket", args("x^2-4",-3,3), "ROOT_OPERATION"
    yield "even-multiplicity-may-not-bracket", args("(x-1)^2",0,2), "ROOT_OPERATION"
    yield "pole-at-midpoint-rejected", args("1/x",-1,1), "ROOT_OPERATION"
    yield "domain-error-rejected", args("sqrt(x)-2",-1,10), "ROOT_OPERATION"
    yield "complex-result-rejected", args("sqrt(-1)+x",-1,1), "ROOT_OPERATION"
    yield "non-midpoint-pole-only-width-converges", args("1/(x-0.1)",-1,1), result(0.1,1e-10,math.inf,min_residual=1e9)
    yield "wide-bracket-small-tolerance", args("x-0.3",-huge,huge,tolerance=1e-12,maxIterations=200), result(0.3,1e-12)
    yield "f64-adjacent-ends", args("(x-999999999999.9998)*10000-1/2",999999999999.9998,999999999999.9999,tolerance=1e-12), result(999999999999.9998,0,0.5,1,min_residual=0.5)
    yield "native-512-tokens-wide-bracket", args(dense,-huge,huge,tolerance=1e-12,maxIterations=200), result(0.3,1e-12)
    yield "native-503-tokens-fraction-chain-wide-bracket", args(fraction_chain,-huge,huge,tolerance=1e-12,maxIterations=200), result(0.3,1e-12)
    yield "native-2048-byte-formula", args(" "*(2048-len(dense))+dense,-huge,huge,tolerance=1e-12), result(0.3,1e-12)
    yield "wide-bracket-many-dynamic-functions-budget", args("x-0.3"+"+(sin(x)-sin(x))"*40,-huge,huge,tolerance=1e-12,maxIterations=200), "ROOT_OPERATION"
    yield "narrow-bracket-dynamic-functions", args("x-0.3"+"+(sin(x)-sin(x))"*12,-1,1,tolerance=1e-10), result(0.3)
    yield "native-legal-depth", args(prefix_depth(62),0,2), result(1,0,0,1)
    yield "native-depth-limit", args(prefix_depth(64),0,2), "ROOT_OPERATION"
    yield "native-token-limit", args(dense+"+0",-huge,huge), "ROOT_OPERATION"
    yield "native-byte-limit", args(" "*2045+"x-20"), "ROOT_OPERATION"
    yield "no-empty-formula", args("  "), "ROOT_OPERATION"
    yield "no-assignment", args("x=2"), "ROOT_OPERATION"
    yield "no-inline-unit-conversion", args("x km -> m"), "ROOT_OPERATION"
    yield "missing-symbol", args("scale*x-2"), "ROOT_OPERATION"
    yield "same-endpoints", args("x",1,1), "ROOT_OPERATION"
    yield "reversed-endpoints", args("x",1,-1), "ROOT_OPERATION"
    yield "upper-endpoint-above-bound", args("x",0,huge+1), "ROOT_OPERATION"
    yield "lower-endpoint-below-bound", args("x",-(huge+1),2), "ROOT_OPERATION"
    yield "numeric-endpoints-only", args("x","NaN",2), "INVALID_INPUT"
    yield "tolerance-below-minimum", args("x", -1,2,tolerance=1e-13), "ROOT_OPERATION"
    yield "tolerance-above-maximum", args("x",-1,2,tolerance=0.0100000001), "ROOT_OPERATION"
    yield "negative-tolerance", args("x",-1,2,tolerance=-1e-12), "ROOT_OPERATION"
    yield "iterations-above-bound", args("x",-1,2,maxIterations=201), "ROOT_OPERATION"
    yield "iterations-below-bound", args("x",-1,2,maxIterations=0), "ROOT_OPERATION"
    yield "iterations-must-be-integer", args("x",-1,2,maxIterations=12.5), "INVALID_INPUT"
    yield "invalid-angle-unit", args("x",-1,2,angle="turns"), "INVALID_INPUT"
    yield "constants-cannot-shadow-coordinate", args("x",constants={"x":1}), "INVALID_CONSTANT"
    yield "constants-cannot-shadow-builtins", args("x",constants={"pi":1}), "INVALID_CONSTANT"
    yield "constants-cannot-use-session-ans", args("x",constants={"ans":1}), "INVALID_CONSTANT"
    yield "constants-maximum-count", args("x",constants={f"v{i}":str(i) for i in range(65)}), "INVALID_CONSTANT"
    yield "constants-reject-invalid-name", args("x",constants={"a-b":1}), "INVALID_CONSTANT"
    yield "constants-reject-bad-fraction", args("x",constants={"a":"1/0"}), "INVALID_CONSTANT"
    yield "constants-reject-non-finite", args("x",constants={"a":"NaN"}), "INVALID_CONSTANT"
    yield "constants-value-byte-limit", args("x",constants={"a":" "*64+"1"}), "INVALID_CONSTANT"
    yield "strict-request-shape", args("x",third=1), "INVALID_INPUT"
    yield "request-byte-limit", args("x"*17000), "INPUT_TOO_LARGE"


def check(actual, expected):
    assert actual["contractVersion"] == CONTRACT
    if isinstance(expected,str):
        assert not actual["ok"] and actual["error"]["code"] == expected,actual
        return
    assert actual["ok"] and actual["error"] is None,actual
    data=actual["data"]
    assert data["converged"] is expected["converged"],data
    assert math.isclose(float(data["root"]),expected["root"],abs_tol=expected["accuracy"],rel_tol=0),data
    residual=abs(float(data["residual"]))
    assert math.isfinite(residual) and expected["minResidual"] <= residual <= expected["maxResidual"],data
    assert type(data["iterations"]) is int and 0 <= data["iterations"] <= 200
    if expected["iterations"] is not None:
        assert data["iterations"] == expected["iterations"],data
    assert data["text"].startswith("区间已收敛" if expected["converged"] else "达到迭代上限")
    assert "|f(x)|" in data["text"]


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
            actual=worker["result"]
            assert actual==native,("native/WASI mismatch",native,actual)
            check(actual,expected)
            if name.endswith("-functions-budget"):
                assert "超出插件执行预算" in actual["error"]["message"], actual
            encoded=compact(actual)
            assert len(encoded)<=MAX_RESULT
            cases.append(dict(case=name,capabilityId=CAPABILITY,inputSha256=digest(compact(inputs)),
                              ok=actual["ok"],resultSha256=digest(encoded),resultBytes=len(encoded)))
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
