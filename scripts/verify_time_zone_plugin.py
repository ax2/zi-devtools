"""Check frozen IANA timezone inputs in the native adapter and the real Windows Worker."""
import argparse
import json
from pathlib import Path

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.time-zone"
CAPABILITY = "devtools.time-zone.convert"
CONTRACT = "1.0.0-rc.1"
IANA = "2025b"
MAX_REQUEST = 8192
MAX_RESULT = 4096


def args(local, source="UTC", zones=None, **options):
    return dict(localTime=local, sourceZone=source, targetZones=["UTC"] if zones is None else zones, **options)


def fixture(name, inputs, expected, **wire):
    return dict(name=name, inputs=inputs, expected=expected, wire=wire)


def unique(utc, offset, *rows):
    return dict(status="unique", utc=utc, sourceOffset=offset, rows=rows)


def candidate(utc, offset):
    return dict(utc=utc, offset=offset)


def ambiguous(earlier, later, selected=None):
    picked = earlier if selected == "earlier" else later if selected == "later" else None
    return dict(status="ambiguous", earlier=earlier, later=later, selectedChoice=selected,
                utc=picked and picked["utc"], sourceOffset=picked and picked["offset"])


def gap(before, after):
    return dict(status="nonexistent", previousValid=before, nextValid=after, utc=None, rows=())


def request(row):
    result = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", commandId=None, input=row["inputs"])
    result.update(row["wire"])
    return result


def fixtures():
    def row(zone, local, offset):
        return zone, local, offset
    yield fixture("native-rollover", args("2026-01-01 09:00","Asia/Shanghai",["America/Los_Angeles","Europe/London"]),
                  unique("2026-01-01T01:00:00+00:00","+08:00",row("America/Los_Angeles","2025-12-31 17:00:00","-08:00"),
                         row("Europe/London","2026-01-01 01:00:00","+00:00")))
    targets = ["UTC","Asia/Kolkata","Australia/Adelaide","Pacific/Chatham","Canada/Newfoundland",
               "America/Los_Angeles","Pacific/Kiritimati","America/St_Johns"]
    yield fixture("maximum-eight-zones-fractional-offsets",
                  args("2026-01-01 00:30:15","Asia/Kathmandu",targets),
                  unique("2025-12-31T18:45:15+00:00","+05:45",
                         row("UTC","2025-12-31 18:45:15","+00:00"), row("Asia/Kolkata","2026-01-01 00:15:15","+05:30"),
                         row("Australia/Adelaide","2026-01-01 05:15:15","+10:30"),row("Pacific/Chatham","2026-01-01 08:30:15","+13:45"),
                         row("Canada/Newfoundland","2025-12-31 15:15:15","-03:30"),row("America/Los_Angeles","2025-12-31 10:45:15","-08:00"),
                         row("Pacific/Kiritimati","2026-01-01 08:45:15","+14:00"),row("America/St_Johns","2025-12-31 15:15:15","-03:30")))
    earlier = candidate("2026-11-01T05:30:00+00:00","-04:00")
    later = candidate("2026-11-01T06:30:00+00:00","-05:00")
    for choice in [None,"earlier","later"]:
        expect = ambiguous(earlier,later,choice)
        expect["rows"] = () if choice is None else (row("UTC", "2026-11-01 "+("05" if choice=="earlier" else "06")+":30:00","+00:00"),)
        options = {} if choice is None else dict(ambiguousChoice=choice)
        yield fixture("new-york-overlap-"+(choice or "unselected"),
                      args("2026-11-01 01:30","America/New_York",**options),expect)
    lh_early = candidate("2026-04-04T14:45:00+00:00","+11:00")
    lh_late = candidate("2026-04-04T15:15:00+00:00","+10:30")
    yield fixture("lord-howe-thirty-minute-overlap",args("2026-04-05 01:45","Australia/Lord_Howe"),
                  dict(**ambiguous(lh_early,lh_late),rows=()))
    yield fixture("lord-howe-explicit-later",args("2026-04-05 01:45","Australia/Lord_Howe",ambiguousChoice="later"),
                  dict(**ambiguous(lh_early,lh_late,"later"),rows=(row("UTC","2026-04-04 15:15:00","+00:00"),)))
    yield fixture("new-york-skip", args("2026-03-08 02:30","America/New_York"),
                  gap("2026-03-08 01:59","2026-03-08 03:00"))
    yield fixture("lord-howe-thirty-minute-skip", args("2026-10-04 02:15","Australia/Lord_Howe"),
                  gap("2026-10-04 01:59","2026-10-04 02:30"))
    yield fixture("kathmandu-fifteen-minute-skip",args("1986-01-01 00:05","Asia/Kathmandu"),
                  gap("1985-12-31 23:59","1986-01-01 00:15"))
    for local in ("00:00","12:00","23:59"):
        yield fixture("apia-skipped-date-"+local, args("2011-12-30 "+local,"Pacific/Apia"),
                      gap("2011-12-29 23:59","2011-12-31 00:00"))
    yield fixture("historical-second-offset",args("1910-01-01 00:00","Asia/Kathmandu"),
                  unique("1909-12-31T18:18:44+00:00","+05:41",row("UTC","1909-12-31 18:18:44","+00:00")))
    yield fixture("iana-2025b-coyhaique-distinct-from-santiago",args("2026-07-01 10:15","America/Coyhaique",["America/Santiago"]),
                  unique("2026-07-01T13:15:00+00:00","-03:00",row("America/Santiago","2026-07-01 09:15:00","-04:00")))
    yield fixture("paraguay-permanent-minus-three",args("2026-06-01 00:00","America/Asuncion"),
                  unique("2026-06-01T03:00:00+00:00","-03:00",row("UTC","2026-06-01 03:00:00","+00:00")))
    yield fixture("explicit-iana-etc-gmt-alias",args("2026-06-01 00:00"," Etc/GMT+5 ",[" Etc/UTC "]),
                  unique("2026-06-01T05:00:00+00:00","-05:00",row("Etc/UTC","2026-06-01 05:00:00","+00:00")))
    yield fixture("choice-has-no-effect-on-unique-local",args("2000-02-29 18:43:28",ambiguousChoice="later"),
                  dict(**unique("2000-02-29T18:43:28+00:00","+00:00",row("UTC","2000-02-29 18:43:28","+00:00")),selectedChoice=None))
    yield fixture("extended-native-calendar-low",args("-262143-01-01 00:00:00","UTC",["Pacific/Apia","America/Los_Angeles"]),
                  unique("-262143-01-01T00:00:00+00:00","+00:00",row("Pacific/Apia","-262143-01-01 12:33:04","+12:33"),
                         row("America/Los_Angeles","-262144-12-31 16:07:02","-07:53")))
    yield fixture("extended-native-calendar-high",args("+262142-12-31 23:59:59","UTC",["Pacific/Kiritimati"]),
                  unique("+262142-12-31T23:59:59+00:00","+00:00",row("Pacific/Kiritimati","+262143-01-01 13:59:59","+14:00")))
    for invalid in ("1900-02-29 13:00","2026-11-31 13:00","2026-11-01 01:30:61","2026-11-01T01:30","2026-11-01 01:30 +04:00","１２月",""):
        yield fixture("invalid-date-"+invalid,args(invalid),"INVALID_LOCAL_TIME")
    for name,source in [("missing-location","UTC+05:30"),("unknown-iana","Mars/Olympus"),("local-name","深圳"),("source-byte-budget","U"*65)]:
        yield fixture(name,args("2026-01-01 00:00",source),"INVALID_SOURCE_ZONE")
    yield fixture("target-unknown",args("2026-01-01 00:00",zones=["Asia/Shanghai","GMT+06:00"]),"INVALID_TARGET_ZONE")
    yield fixture("target-byte-budget",args("2026-01-01 00:00",zones=[" " * 64+"UTC"]),"INVALID_TARGET_ZONE")
    yield fixture("duplicate-trimmed-iana-name",args("2026-01-01 00:00",zones=["UTC"," UTC "]),"DUPLICATE_TARGET")
    yield fixture("target-count-zero",args("2026-01-01 00:00",zones=[]),"TARGET_COUNT")
    yield fixture("target-count-nine",args("2026-01-01 00:00",zones=targets+["Europe/London"]),"TARGET_COUNT")
    yield fixture("target-type",args("2026-01-01 00:00",zones=["UTC",7]),"INVALID_INPUT")
    yield fixture("unknown-field",args("2026-01-01 00:00",offset="+05:00"),"INVALID_INPUT")
    yield fixture("wrong-ambiguity-choice",args("2026-01-01 00:00",ambiguousChoice="automatic"),"INVALID_INPUT")
    yield fixture("missing-required-field",{"sourceZone":"UTC","targetZones":["UTC"]},"INVALID_INPUT")
    yield fixture("foreign-plugin",args("2026-01-01 00:00"),"INVALID_REQUEST",pluginId="com.example.foreign")
    yield fixture("foreign-capability",args("2026-01-01 00:00"),"UNKNOWN_OPERATION",capabilityId="devtools.time-zone.wrong")
    yield fixture("no-command-use",args("2026-01-01 00:00"),"INVALID_REQUEST",commandId="start")
    empty = fixture("request-exact-8192",args(""),"INVALID_LOCAL_TIME")
    overhead = len(compact(request(empty)))
    empty["inputs"]["localTime"] = "x"*(MAX_REQUEST - overhead)
    assert len(compact(request(empty)))==MAX_REQUEST
    yield empty
    yield fixture("request-8193",args("x"*(MAX_REQUEST-overhead+1)),"INPUT_TOO_LARGE")


def check(result, expected):
    assert result["contractVersion"]==CONTRACT
    if isinstance(expected,str):
        assert result["ok"] is False and result["data"] is None and result["error"]["code"]==expected,result
        return
    assert result["ok"] and result["error"] is None,result
    data=result["data"]
    assert data["tzdbVersion"]==IANA and data["sourceZone"] and data["sourceLocal"],data
    assert data["sourceZone"] in data["text"] and f"IANA {IANA}" in data["text"],data
    for key,value in expected.items():
        if key=="rows":
            assert tuple((row["zone"],row["local"],row["offset"]) for row in data["rows"])==value,data
            assert all(row["zone"] in data["text"] and row["local"] in data["text"] for row in data["rows"]),data
        else:
            assert data[key]==value,(key,value,data)
    assert len(data["rows"])<=8
    if data["utc"] is None:
        assert data["sourceOffset"] is None and data["sourceAbbreviation"] is None and not data["rows"],data
    else:
        assert data["utc"].endswith("+00:00") and data["utc"] in data["text"],data


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","module","native","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    config=parser.parse_args()
    reports, failures=[],[]
    for row in fixtures():
        name=row["name"]
        message=request(row)
        try:
            native=invoke([config.native],compact(message))
            worker=invoke([config.worker,"--zicode-plugin-worker"],
                          compact(dict(workerKind="wasi",workerPath=str(config.module),**message)))
            assert worker["ok"],worker
            result=worker["result"]
            assert result==native,("native/WASI mismatch",native,result)
            check(result,row["expected"])
            serialized=compact(result)
            assert len(serialized)<=MAX_RESULT
            reports.append(dict(case=name,requestSha256=digest(compact(message)),resultBytes=len(serialized),
                                resultSha256=digest(serialized),result=result))
        except (AssertionError,KeyError,TypeError,ValueError) as error:
            failures.append(dict(case=name,reason=str(error)[:1000]))
    proof=dict(cases=reports,failures=failures,passed=len(reports),ianaVersion=IANA,workerProfile="compute.wasi.v1",
               wasmSha256=digest(config.module.read_bytes()),moduleBytes=config.module.stat().st_size,
               workerSha256=digest(config.worker.read_bytes()))
    config.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print(json.dumps(dict(passed=len(reports),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
