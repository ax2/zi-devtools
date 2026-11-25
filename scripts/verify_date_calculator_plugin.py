"""Compare date-tool results from the native adapter and production Windows Worker."""

import argparse
from datetime import date
import json
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.date-calculator"
PREFIX = "devtools.date."
CONTRACT = "1.0.0-rc.1"
MAX_RESULT = 2 * 1024
OPERATIONS = ["difference", "offset", "workday_count", "workday_offset"]
WEEKDAY = ["星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日"]


def span(start, end):
    return dict(start=start,end=end)


def offset(start, amount, unit="days", **options):
    return dict(start=start,amount=amount,unit=unit,**options)


def business_offset(start, amount):
    return dict(start=start,amount=amount)


def count(number, weekday=False):
    if weekday:
        return dict(days=number,weekdaysOnly=True,text=f"{number} 工作日（周一至周五）")
    return dict(days=number,text=f"{number} 天")


def day_result(value, weekday=False):
    label = WEEKDAY[date.fromisoformat(value).weekday()]
    result = dict(date=value,weekday=label,text=f"{value} · {label}")
    if weekday:
        result["weekdaysOnly"] = True
    return result


def fixtures():
    first, last = date.min.isoformat(), date.max.isoformat()
    yield "leap-difference","difference",span("2024-02-28","2024-03-01"),count(2)
    yield "signed-difference","difference",span("2024-03-01","2024-02-28"),count(-2)
    yield "positive-entire-range","difference",span(first,last),count((date.max-date.min).days)
    yield "negative-entire-range","difference",span(last,first),count((date.min-date.max).days)
    yield "invalid-century-date","difference",span("1900-02-29","1900-03-01"),"INVALID_DATE"
    yield "year-zero-rejected","difference",span("0000-01-01",first),"INVALID_DATE"
    yield "invalid-unpadded-date","difference",span("2026-1-02","2026-01-04"),"INVALID_DATE"
    yield "integer-offset-day","offset",offset("2000-02-28",1),day_result("2000-02-29")
    yield "end-of-month-explicit-error","offset",offset("2024-01-31",1,"months"),"INVALID_OFFSET"
    yield "month-end-clamp","offset",offset("2024-01-31",1,"months",clamp=True),day_result("2024-02-29")
    yield "reverse-month-end-clamp","offset",offset("2024-03-31",-1,"months",clamp=True),day_result("2024-02-29")
    yield "leap-year-clamp","offset",offset("2024-02-29",1,"years",clamp=True),day_result("2025-02-28")
    yield "year-rollover","offset",offset("2026-12-31",1),day_result("2027-01-01")
    yield "upper-date-bound","offset",offset(last,1),"INVALID_OFFSET"
    yield "lower-date-bound","offset",offset(first,-1),"INVALID_OFFSET"
    yield "amount-budget","offset",offset("2026-01-01",4_000_001),"INVALID_OFFSET"
    yield "noninteger-amount","offset",offset("2026-01-01",1.5),"INVALID_INPUT"
    yield "weekday-default-excludes-start","workday_count",span("2026-10-05","2026-10-09"),count(4,True)
    yield "weekday-include-start","workday_count",dict(**span("2026-10-05","2026-10-09"),includeStart=True),count(5,True)
    yield "same-weekend-count","workday_count",dict(**span("2026-10-10","2026-10-10"),includeStart=True),count(0,True)
    yield "reversed-weekday-count-rejected","workday_count",span(last,first),"INVALID_RANGE"
    yield "full-span-weekday-count","workday_count",dict(**span(first,last),includeStart=True),count(2_608_615,True)
    yield "offset-across-weekend","workday_offset",business_offset("2026-10-09",1),day_result("2026-10-12",True)
    yield "offset-back-across-weekend","workday_offset",business_offset("2026-10-12",-1),day_result("2026-10-09",True)
    yield "offset-zero-keeps-weekend","workday_offset",business_offset("2026-10-10",0),day_result("2026-10-10",True)
    yield "full-span-forward-weekdays","workday_offset",business_offset(first,2_608_614),day_result(last,True)
    yield "full-span-reverse-weekdays","workday_offset",business_offset(last,-2_608_614),day_result(first,True)
    yield "one-weekday-beyond-range","workday_offset",business_offset(last,-2_608_615),"INVALID_OFFSET"
    yield "extra-argument-rejected","difference",dict(**span(first,last),open=True),"INVALID_INPUT"
    yield "request-size-budget","difference",span("X"*4200,last),"INPUT_TOO_LARGE"
    yield "unknown-operation","undefined",{},"UNKNOWN_OPERATION"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args = parser.parse_args()
    results = []
    for case,operation,inputs,expected in fixtures():
        request = dict(pluginId=PLUGIN,sceneId="programming",capabilityId=PREFIX+operation,
                       commandId=None,input=inputs)
        native = invoke([args.native],compact(request))
        worker = invoke(
            [args.worker,"--zicode-plugin-worker"],
            compact(dict(workerKind="wasi",workerPath=str(args.module),**request)),
        )
        assert worker["ok"], (case,worker)
        actual = worker["result"]
        assert actual == native, (case,"native/WASI mismatch")
        assert actual["contractVersion"] == CONTRACT
        if isinstance(expected,str):
            assert not actual["ok"] and actual["error"]["code"] == expected, (case,actual)
        else:
            assert actual["ok"] and actual["data"] == expected and actual["error"] is None, (case,actual)
        encoded = compact(actual)
        assert len(encoded) <= MAX_RESULT
        results.append(dict(case=case,capabilityId=PREFIX+operation,inputSha256=digest(compact(inputs)),
                            ok=actual["ok"],resultSha256=digest(encoded)))
        print(case,"passed",flush=True)
    receipt = dict(passed=len(results),wasmSha256=digest(args.module.read_bytes()),
                   workerSha256=digest(args.worker.read_bytes()),workerProfile="compute.wasi.v1",cases=results)
    args.evidence.write_text(json.dumps(receipt,indent=2)+"\n",encoding="utf-8")


if __name__ == "__main__":
    main()
