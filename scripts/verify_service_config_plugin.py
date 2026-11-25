"""Exercise only synthetic ZiDevTools service YAML through native and Windows production adapters."""

import argparse
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.service-config"
CAPABILITY = "devtools.services.config.preflight"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_FILE = 16 * 1024
MAX_RESULT = 16 * 1024
REDACTED = tuple("FIXTURE_PRIVATE_"+field for field in (
    "ID","REPO","COMMAND","STOP","HEALTH","STATE","ENV_KEY","ENV_VALUE","DESCRIPTION","FILE","TAG",
))


def case_fixture():
    return """state_dir: 'D:/FIXTURE_PRIVATE_STATE'
extra_metadata: ignored by native import
services:
  FIXTURE_PRIVATE_ID:
    name: '工程服务A'
    repo: '~\\FIXTURE_PRIVATE_REPO'
    command: |
      echo FIXTURE_PRIVATE_COMMAND
      echo 第二行
    stop_command: echo FIXTURE_PRIVATE_STOP
    stop_timeout_ms: 100
    description: FIXTURE_PRIVATE_DESCRIPTION
    health_url: https://fixture.invalid/FIXTURE_PRIVATE_HEALTH
    port: 8765
    env:
      FIXTURE_PRIVATE_ENV_KEY: FIXTURE_PRIVATE_ENV_VALUE
      OMIT: null
    config_files: [FIXTURE_PRIVATE_FILE]
    tags: [FIXTURE_PRIVATE_TAG]
  中文服务:
    repo: 'D:\\备份空间'
    command: echo second
    stop_timeout_ms: 60000
"""


def accepted(total, *, state=False, stops=0, health=0, env=0, problems=()):
    return dict(parseable=True,validForImport=total>0 and not problems,serviceCount=total,
                stateDirectoryProvided=state,configuredStopCommands=stops,configuredHealthChecks=health,
                environmentEntries=env,problems=[dict(serviceNumber=i,code=code) for i,code in problems])


def with_size(s, size):
    data=s.encode()
    assert len(data)+1<size
    return data+b"#"+b"."*(size-len(data)-1)


def min_service(detail=""):
    return "services:\n  demo:\n    repo: D:/synthetic\n    command: echo fixture\n"+detail


def fixtures():
    rich=case_fixture()
    good=accepted(2,state=True,stops=1,health=1,env=2)
    for label,mime,s in (
        ("yaml-octet",MIME,rich),("yml-text","text/plain",rich),
        ("bom",MIME,"\ufeff"+rich),("crlf",MIME,rich.replace("\n","\r\n")),
        ("explicit-document-start",MIME,"---\n"+rich),
    ):
        yield label,s.encode(),{},mime,good
    yield "same-native-defaults",min_service().encode(),{},MIME,accepted(1)
    yield "yaml-alias-backed-service",b"default: &good {repo: D:/x, command: echo x}\nservices: { one: *good, two: *good }\n",{},MIME,accepted(2)
    yield "explicit-null-stops-and-env",min_service("    stop_command: null\n    env: {FIXTURE_PRIVATE_ENV_KEY: null}\n").encode(),{},MIME,accepted(1,env=1)
    yield "existing-root-and-service-extensions",min_service("    newKey: arbitrary\n").encode(),{},MIME,accepted(1)
    yield "empty-services-map",b"services: {}",{},MIME,accepted(0)
    yield "empty-selected-command-and-repo",b"services:\n  aa: {repo: D:/x, command: '  '}\n  bb: {repo: '', command: run}\n",{},MIME,accepted(2,
        problems=((1,"MISSING_REPO_OR_COMMAND"),(2,"MISSING_REPO_OR_COMMAND")))
    yield "only-first-problem-and-stable-key-order",b"""services:
  z_empty: {repo: D:/x, command: run, stop_command: ' ', stop_timeout_ms: 0}
  a_empty: {repo: '', command: run, stop_command: ' '}
  m_timeout: {repo: D:/x, command: run, stop_timeout_ms: 60001}
""",{},MIME,accepted(3,stops=2,problems=((1,"MISSING_REPO_OR_COMMAND"),(2,"INVALID_STOP_TIMEOUT"),(3,"EMPTY_STOP_COMMAND")))
    for n in (0,99,60001):
        yield f"stop-outside-range-{n}",min_service(f"    stop_timeout_ms: {n}\n").encode(),{},MIME,accepted(1,
            problems=((1,"INVALID_STOP_TIMEOUT"),))
    for n in (100,5000,60000):
        yield f"valid-stop-timeout-{n}",min_service(f"    stop_timeout_ms: {n}\n").encode(),{},MIME,accepted(1)
    yield "explicitly-reports-configured-but-no-subsystem-check",min_service("    health_url: unusable-address\n    port: 0\n").encode(),{},MIME,accepted(1,health=1)
    for name,s in (
        ("not-yaml","services: ["),
        ("missing-services","repository: ignored"),
        ("wrong-services-type","services: [one]"),
        ("missing-repo","services: {one: {command: echo x}}"),
        ("missing-command","services: {one: {repo: D:/x}}"),
        ("wrong-command-type","services: {one: {repo: D:/x, command: [one]}}"),
        ("wrong-env-type",min_service("    env: {secret: [one]}")),
        ("port-out-of-type-range",min_service("    port: 65536")),
        ("invalid-stop-type",min_service("    stop_timeout_ms: -1")),
        ("multiple-documents",min_service()+"---\n"+min_service()),
        ("unresolved-alias","services: {demo: *absent}"),
        ("duplicate-known-field","services: {demo: {repo: D:/x, command: echo ok, command: echo two}}"),
        ("source-secret-not-in-parser-error","services: {demo: {repo: D:/x, command: [FIXTURE_PRIVATE_COMMAND]}}"),
    ):
        yield name,s.encode(),{},MIME,dict(parseable=False,validForImport=False)
    yield "wrong-file-encoding",b"\xff",{},MIME,"INVALID_UTF8"
    yield "unsupported-mime",rich.encode(),{},"image/png","UNSUPPORTED_MIME"
    yield "unknown-input-option",rich.encode(),{"apply":True},MIME,"INVALID_REQUEST"
    yield "exact-16-kib-comment",with_size(rich,MAX_FILE),{},MIME,good
    yield "over-byte-limit",with_size(rich,MAX_FILE+1),{},MIME,"INPUT_TOO_LARGE"
    rows=lambda n,values:"services:\n"+"".join(f"  s{i:03}: {{{values}}}\n" for i in range(n))
    yield "exact-64-services",rows(64,"repo: D:/Synthetic, command: echo synthetic").encode(),{},MIME,accepted(64)
    yield "one-service-over",rows(65,"repo: D:/Synthetic, command: echo synthetic").encode(),{},MIME,"SERVICE_LIMIT"
    yield "all-64-problems-fit-in-output",rows(64,"repo: D:/x, command: echo x, stop_timeout_ms: 1").encode(),{},MIME,accepted(64,
        problems=tuple((i,"INVALID_STOP_TIMEOUT") for i in range(1,65)))
    rich_fields="repo: D:/Synthetic, command: echo synthetic, env: {VAR1: '中文', VAR2: x, VAR3: null, VAR4: 'plain data'}, tags: [test, local, dev], stop_command: echo stop, stop_timeout_ms: 60000, health_url: 'https://fixture.invalid/'"
    yield "dense-64-services-gives-guidance",with_size(rows(64,rich_fields),MAX_FILE),{},MIME,"STRUCTURE_TOO_DENSE"
    yield "32-services-with-advanced-settings-at-16-kib",with_size(rows(32,rich_fields),MAX_FILE),{},MIME,accepted(32,env=128,stops=32,health=32)
    packed=lambda n: min_service("    tags: ["+",".join("t" for _ in range(n))+"]\n")
    yield "dense-small-scalars-gives-guidance",with_size(packed(7000),MAX_FILE),{},MIME,"STRUCTURE_TOO_DENSE"
    yield "1280-small-scalars-at-16-kib",with_size(packed(1280),MAX_FILE),{},MIME,accepted(1)
    env=lambda n: min_service("    env:\n"+"".join(f"      VAR{i:03}: x\n" for i in range(n)))
    yield "450-env-pairs-at-16-kib",with_size(env(450),MAX_FILE),{},MIME,accepted(1,env=450)
    yield "650-env-pairs-gives-guidance",with_size(env(650),MAX_FILE),{},MIME,"STRUCTURE_TOO_DENSE"
    use_anchor="default: &a {repo: D:/x, command: run, tags: ["+",".join("t" for _ in range(480))+"]}\n"
    use_anchor+="services:\n"+"".join(f"  s{i:03}: *a\n" for i in range(6))
    yield "large-alias-expansion-gives-guidance",with_size(use_anchor,MAX_FILE),{},MIME,"STRUCTURE_TOO_DENSE"
    yield "little-alias-at-16-kib",with_size("default: &good {repo: D:/x, command: run}\nservices: {one: *good, two: *good}\n",MAX_FILE),{},MIME,accepted(2)
    long_command=min_service("    stop_command: echo "+"星"*5000+"\n")
    yield "very-long-unicode-scalar",with_size(long_command,MAX_FILE),{},MIME,accepted(1,stops=1)
    nested=min_service("    tags: &l [*l]\n")
    yield "recursive-alias-is-rejected",nested.encode(),{},MIME,dict(parseable=False,validForImport=False)



def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module),sha256=digest(module)),resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I",len(header)) + header + module + payload


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases, failures=[], []
    with TemporaryDirectory() as folder:
        local=Path(folder)/"services.yml"
        for name,payload,options,mime,expected in fixtures():
            local.write_bytes(payload)
            direct,wire=envelopes(module,payload,options,mime)
            try:
                native=invoke([args.native,local],direct)
                response=invoke([args.worker,"--zicode-plugin-worker"],wire)
                assert response["ok"], response
                worker=response["result"]
                assert worker==native,(name,"native/WASI mismatch")
                assert worker["contractVersion"]==CONTRACT
                data=worker["data"]
                if isinstance(expected,str):
                    assert not worker["ok"] and worker["error"]["code"]==expected, worker
                    assert data is None and worker["error"]["message"]
                else:
                    assert worker["ok"] and worker["error"] is None, worker
                    assert data["text"]
                    for key,wanted in expected.items():
                        assert data[key]==wanted,(key,data,wanted)
                    if name=="only-first-problem-and-stable-key-order":
                        assert "第 1 项 · repo 或 command 为空" in data["text"]
                        assert "第 2 项 · stop_timeout_ms 须为" in data["text"]
                        assert "第 3 项 · stop_command 不能为空" in data["text"]
                    if name=="source-secret-not-in-parser-error":
                        assert "请检查第 " in data["text"]
                assert all(marker.encode() not in compact(worker) for marker in REDACTED)
                assert len(compact(worker))<=MAX_RESULT
                cases.append(dict(case=name,inputSha256=digest(payload),inputBytes=len(payload),
                                  mime=mime,ok=worker["ok"],resultSha256=digest(compact(worker))))
            except Exception as error:
                failures.append(dict(case=name,error=repr(error)[:1000]))
                print(f"FAIL {name} {repr(error)[:160]}",flush=True)
            else:
                print(f"PASS {name}",flush=True)
    proof=dict(passed=len(cases),wasmSha256=digest(module),workerSha256=digest(args.worker.read_bytes()),
               workerProfile=PROFILE,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
