"""Check real Windows Worker clock backup validation against fixed fixtures and its native adapter."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PROFILE = "resource.wasi.input.draft2"
PLUGIN = "com.zicode.devtools.clock-backup"
CAPABILITY = "devtools.clock.backup.inspect"
MAX_FILE = 512 * 1024
MAX_WORK = 224 * 1024
MAX_RESULT = 2048
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
SECRET = "SYNTHETIC-PRIVATE-CLOCK-LABEL"
NOW = "2026-11-26T01:05:02Z"
ZONES = ["UTC","Asia/Shanghai","Asia/Kathmandu","America/New_York","Europe/Amsterdam","America/Asuncion","Pacific/Apia",
         "Pacific/Chatham","Europe/London","Australia/Lord_Howe","America/St_Johns","Africa/Cairo"]


def digest(data):
    return sha256(data).hexdigest()


def compact(data, ascii=False):
    return json.dumps(data,ensure_ascii=ascii,separators=(",",":"),sort_keys=True).encode()


def test_timer(id, word=SECRET):
    return dict(id=id, name=word,total=604800000,cycle=604800000,remaining=1000,running=True,finished=False)


def test_alarm(id, word=SECRET):
    return dict(id=id,name=word,zone="Asia/Kathmandu",at=NOW,daily=True,late=False,time="01:05:02",enabled=True,snooze=None)


def snap(version=3, pattern=None):
    data=dict(schema=version,tool_version="0.7.0-"+SECRET,saved_at=NOW,policy="Continue" if version==3 else "Pause",
              tab=4,zones=ZONES[:3],analog=True,meeting=-720,stopwatch=30000,laps=[0,1000,2000],
              timers=[test_timer(1)],alarms=[test_alarm(33)],focus=test_timer(0),work=25,short=5,long=15,completed=2,
              phase=0,notices=[dict(source={"Timer":1},title=SECRET)])
    if version>=2:
        data["audio"]=dict(enabled=False,muted=True,volume=100,tone="Soft",quiet=True,quiet_start=1439,quiet_end=0)
    if version>=3:
        data["window_sizes"]=[dict(target="world:UTC",width=320,height=260)]
    if pattern is not None:
        word=(SECRET+pattern*80)[:80]
        title=(SECRET+pattern*512)[:512]
        data.update(zones=ZONES,stopwatch=3153600000000,laps=list(range(3153600000000-1023,3153600000000+1)),
                    timers=[test_timer(i,word) for i in range(1,33)],alarms=[test_alarm(i,word) for i in range(33,97)],
                    notices=[dict(source={"Timer":i},title=title) for i in range(1,33)]
                     +[dict(source={"Alarm":i},title=title) for i in range(33,97)]+[dict(source="Focus",title=title)],
                    window_sizes=[dict(target="world:"+zone,width=1600,height=1200) for zone in ZONES]
                     +[dict(target="timer:"+str(i),width=320,height=260) for i in range(1,35)]
                     +[dict(target="stopwatch",width=800,height=460),dict(target="focus",width=800,height=460)])
    return data


def padded(source, size=MAX_FILE):
    assert len(source)<=size,len(source)
    return source+b" "*(size-len(source))


def wire(raw, module, mime="application/octet-stream", changes=None, wrong_digest=False):
    material=dict(resourceId="73646b15-11e5-4ab2-b707-6b91eeba8064",
                  byteLength=len(raw),mimeType=mime,contentVersion="a"*64 if wrong_digest else digest(raw))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input={})
    if changes:
        request.update(changes)
    direct=compact(dict(commandId=None,resourceInputProfile=PROFILE,resources=[dict(index=0,**material)],**request))
    header=compact(dict(profile=PROFILE,request=dict(workerKind="wasi",workerPath="",**request),
                         module=dict(byteLength=len(module),sha256=digest(module)),resources=[material]))
    return direct,b"ZIRW\x00\x00\x00\x02"+struct.pack("<I",len(header))+header+module+raw


def invoke(args, stdin):
    result=subprocess.run(args,input=stdin,capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        decoded=json.loads(result.stdout)
    except ValueError:
        decoded=dict(stdout=result.stdout[:200].decode(errors="replace"),stderr=result.stderr[-500:].decode(errors="replace"))
    return result.returncode,decoded


def frame(body):
    extra=MAX_FILE-len(body)
    assert extra>=0
    padding=(b"\r\n\t "*((extra+3)//4))[:extra]
    return padding[:extra//2]+body+padding[extra//2:]


def internal(body, size):
    assert len(body) <= size
    start=b'"finished":'
    assert start in body
    return body.replace(start,start+b' '*(size-len(body)),1)


def vector(name,data,expected,*,mime=MIME,changes=None,wrong_digest=False):
    return dict(name=name,raw=data if isinstance(data,bytes) else compact(data),expected=expected,
                mime=mime,changes=changes or {},wrongDigest=wrong_digest)


def fixtures():
    for version in (1,2,3):
        yield vector(f"version-{version}",snap(version),dict(schemaVersion=version))
    yield vector("recognized-json-mime",snap(3),dict(schemaVersion=3),mime="application/json")
    yield vector("normal-512-kib-with-mixed-padding",frame(compact(snap(3))),dict(schemaVersion=3))
    maximum=snap(3,"中")
    counts=dict(schemaVersion=3,timerCount=32,alarmCount=64,lapCount=1024,noticeCount=97,zoneCount=12,windowSizeCount=48)
    pretty=json.dumps(maximum,ensure_ascii=False,indent=2).encode()
    yield vector("native-pretty-full-chinese-structure",pretty,counts)
    yield vector("full-bmp-at-exact-work-limit",internal(compact(maximum),MAX_WORK),counts)
    yield vector("full-bmp-work-limit-plus-one",internal(compact(maximum),MAX_WORK+1),"SNAPSHOT_COMPLEXITY")
    max_ascii=compact(snap(3,"x"))
    yield vector("full-ascii-512-kib",frame(max_ascii),counts)
    yield vector("full-ascii-exact-byte-and-work-limit",frame(internal(max_ascii,128*1024)),counts)
    yield vector("both-budgets-combined-plus-one",frame(internal(max_ascii,128*1024+1)),"SNAPSHOT_COMPLEXITY")
    yield vector("full-chinese-with-excess-exterior-padding",frame(compact(maximum)),"SNAPSHOT_COMPLEXITY")
    control=snap(3,"x")
    for notice in control["notices"][:8]:
        notice["title"]="\0"*512
    control_body=compact(control)
    assert control_body.count(b"\\")==4096
    yield vector("legitimate-escaped-small-records",control_body,counts)
    yield vector("escaped-sums-exact-work-limit",internal(control_body,MAX_WORK-6*4096),counts)
    yield vector("escaped-sums-work-limit-plus-one",internal(control_body,MAX_WORK-6*4096+1),"SNAPSHOT_COMPLEXITY")
    dense=snap(3,"🕑")
    for item in dense["timers"]+dense["alarms"]: item["name"]="🕑"*80
    for item in dense["notices"]: item["title"]="🕑"*512
    yield vector("structurally-valid-dense-astral",json.dumps(dense,ensure_ascii=False,indent=2).encode(),"SNAPSHOT_COMPLEXITY")
    yield vector("native-shape-all-unicode-escaped",compact(maximum,ascii=True),"SNAPSHOT_COMPLEXITY")
    yield vector("limit-512-kib-plus-one",padded(compact(snap()),MAX_FILE+1),"INPUT_TOO_LARGE")
    yield vector("safe-future-schema",snap(4),"UNSUPPORTED_VERSION")
    yield vector("malformed-json",b'{"schema":3,',"INVALID_JSON")
    yield vector("invalid-utf8",b'{"schema":3,"tool_version":"\xff"}',"INVALID_JSON")
    yield vector("wrong-root",b'[{"schema":3}]',"INVALID_JSON")
    yield vector("outer-non-json-form-feed",b"\x0c"+compact(snap()),"INVALID_JSON")
    yield vector("outer-illegal-suffix",compact(snap())+b"\nx","INVALID_JSON")
    yield vector("multiple-root-objects",compact(snap())+compact(snap()),"INVALID_JSON")
    yield vector("whitespace-512-kib",b" "*MAX_FILE,"INVALID_JSON")
    yield vector("unknown-json-field",dict(snap(),ownerToken=SECRET),"INVALID_JSON")
    old=snap(1); old["audio"]=snap()["audio"]
    yield vector("v1-cannot-claim-audio",old,"INVALID_SNAPSHOT")
    old=snap(2); old["window_sizes"]=snap()["window_sizes"]
    yield vector("v2-cannot-claim-window-sizes",old,"INVALID_SNAPSHOT")
    old=snap(2); del old["audio"]
    yield vector("v2-needs-audio",old,"INVALID_SNAPSHOT")
    old=snap(); old["audio"]["volume"]=101
    yield vector("audio-outside-bound",old,"INVALID_SNAPSHOT")
    old=snap(); old["window_sizes"]=[dict(target="timer:1",width=320,height=260),dict(target="timer:01",width=320,height=260)]
    yield vector("duplicate-window-target-after-canonicalization",old,"INVALID_SNAPSHOT")
    old=snap(); old["window_sizes"][0]["height"]=1201
    yield vector("invalid-window-size",old,"INVALID_SNAPSHOT")
    old=snap(); old["window_sizes"][0]["target"]="world:Unknown/City"
    yield vector("invalid-window-zone",old,"INVALID_SNAPSHOT")
    yield vector("invalid-known-time-zone",dict(snap(),zones=["Asia/Kathmandu","Null/Zone"]),"INVALID_SNAPSHOT")
    yield vector("duplicate-known-time-zone",dict(snap(),zones=["UTC","UTC"]),"INVALID_SNAPSHOT")
    yield vector("out-of-order-laps",dict(snap(),laps=[0,2000,1000]),"INVALID_SNAPSHOT")
    yield vector("lap-one-above-native-limit",dict(snap(),laps=[0]*1025),"INVALID_JSON")
    yield vector("heavily-malformed-native-lap-list",dict(snap(),laps=[0]*65000),"INVALID_JSON")
    old=snap(); old["notices"][0]["source"]={"Alarm":1}
    yield vector("orphaned-reminder",old,"INVALID_SNAPSHOT")
    old=snap(); old["notices"].append(dict(old["notices"][0]))
    yield vector("duplicate-reminder",old,"INVALID_SNAPSHOT")
    old=snap(); old["alarms"][0]["id"]=1
    yield vector("alarm-id-overlap-timer-id",old,"INVALID_SNAPSHOT")
    old=snap(); old["timers"][0]["name"]="🕑"*81
    yield vector("native-multibyte-name-length-limit",old,"INVALID_SNAPSHOT")
    old=snap(); old["timers"][0]["finished"]=True
    yield vector("running-finished-timer-disagreement",old,"INVALID_SNAPSHOT")
    old=snap(); old["focus"]["id"]=12
    yield vector("native-focus-id-is-zero",old,"INVALID_SNAPSHOT")
    yield vector("bad-mime",snap(),"UNSUPPORTED_MIME",mime="image/png")
    yield vector("unknown-invoke-params",snap(),"INVALID_REQUEST",changes=dict(input={"path":"unused"}))
    yield vector("other-plugin-id",snap(),"INVALID_REQUEST",changes=dict(pluginId="com.example.other"))
    yield vector("other-capability-id",snap(),"INVALID_REQUEST",changes=dict(capabilityId="devtools.clock.wrong"))
    yield vector("altered-snapshot-rejected-before-guest",snap(),dict(schemaVersion=3),wrong_digest=True)


def check(result,row):
    assert result["contractVersion"]==CONTRACT and SECRET not in json.dumps(result,ensure_ascii=False),result
    expected=row["expected"]
    if isinstance(expected,str):
        assert not result["ok"] and result["data"] is None and result["error"]["code"]==expected,result
        return
    assert result["ok"] and result["error"] is None,result
    data=result["data"]
    version=expected["schemaVersion"]
    assert data["byteLength"]==len(row["raw"]) and data["savedAtUtc"]==NOW and data["legacyV1"]==(version==1),data
    assert data["audioSettingsPresent"]==(version>=2) and data["storedTimerPolicy"]==("continue" if version==3 else "pause"),data
    expected_counts=dict(timerCount=1,alarmCount=1,lapCount=3,noticeCount=1,zoneCount=3,windowSizeCount=int(version>=3))
    expected_counts.update(expected)
    for key,value in expected_counts.items():
        assert data[key]==value,(key,value,data)
    assert f"时钟备份 v{version} 通过校验" in data["text"] and NOW in data["text"]


def build_proof(worker,module,native,scratch):
    cases,failures=[],[]
    module_bytes=module.read_bytes()
    with TemporaryDirectory(dir=scratch) as temp:
        file=Path(temp)/"clock.json"
        for row in fixtures():
            direct,packet=wire(row["raw"],module_bytes,row["mime"],row["changes"],row["wrongDigest"])
            try:
                file.write_bytes(row["raw"])
                native_status,native_result=invoke([native,file],direct)
                assert native_status==0,native_result
                actual_status,output=invoke([worker,"--zicode-plugin-worker"],packet)
                if row["wrongDigest"]:
                    assert actual_status!=0 and output["error"]=="Resource Worker content mismatch",output
                else:
                    assert actual_status==0 and output["ok"],output
                    assert output["result"]==native_result,(native_result,output)
                check(native_result,row)
                assert file.read_bytes()==row["raw"]
                result=compact(native_result)
                assert len(result)<=MAX_RESULT
                cases.append(dict(case=row["name"],inputSha256=digest(row["raw"]),inputBytes=len(row["raw"]),mime=row["mime"],
                                  hostRejected=row["wrongDigest"],resultBytes=len(result),resultSha256=digest(result),result=native_result))
            except (AssertionError,KeyError,TypeError,ValueError,subprocess.SubprocessError) as error:
                failures.append(dict(case=row["name"],reason=str(error)[:1100]))
    return dict(passed=len(cases),failures=failures,cases=cases,wasmSha256=digest(module_bytes),
                moduleBytes=len(module_bytes),workerSha256=digest(worker.read_bytes()),workerProfile=PROFILE,
                fileLimit=MAX_FILE,workLimit=MAX_WORK)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","module","native","evidence","scratch"):
        parser.add_argument("--"+name,type=Path,required=True)
    config=parser.parse_args()
    proof=build_proof(config.worker,config.module,config.native,config.scratch)
    config.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print(json.dumps(dict(passed=proof["passed"],failures=proof["failures"]),ensure_ascii=False))
    return bool(proof["failures"])


if __name__=="__main__":
    raise SystemExit(main())
