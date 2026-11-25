"""Compare native and real Windows Worker file review with native report-shaped fixtures."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN="com.zicode.devtools.python-env-report"
CAPABILITY="devtools.django.environment.report.review"
TITLE="Python / Django 环境报告查看"
INPUT="resource.wasi.input.draft2"
OUTPUT="resource.wasi.output.draft1"
JSON="application/json"
TEXT="text/plain"
OCTET="application/octet-stream"


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def fingerprint():
    fields=dict(appVersion='0.82.0',os='windows',architecture='x86_64',buildProfile='release')
    domain='ZiDevTools runtime environment v1\x00'
    sha=digest((domain+'\x00'.join(fields.values())).encode())
    return dict(schema=1,**fields,sha256=sha)


def capture(version="3.12.8", *, venv=True, distribution="5.1.7", module=True, scripts=True):
    root = r"X:\fixture-private\venv" if venv else r"Y:\fixture-private\base"
    return dict(executable=root + r"\python.exe", version=version, prefix=root,
                basePrefix=r"Y:\fixture-private\base", scriptsPath=root + r"\Scripts" if scripts else None,
                virtualEnvironment=venv,
                django=None if distribution is None else dict(version=distribution,
                    location=root + r"\site-packages\django"),
                djangoImportOrigin=root + r"\modules\django\__init__.py" if module else None)


def export(selected=None, comparison=None, *, admin=True, mismatch=True):
    selected = capture() if selected is None else selected
    scripts = selected["scriptsPath"]
    return dict(selected=selected, comparison=comparison, pathPython=r"Y:\fixture-private\python.exe",
                pathDjangoAdmin=r"Z:\fixture-private\django-admin.exe" if admin else None,
                expectedSiblingDjangoAdmin=scripts + r"\django-admin.exe" if scripts is not None else None,
                entryLayoutMismatch=mismatch if admin and scripts is not None else None,
                notes="fixture-private raw capture notes",
                generatedBy=dict(toolId="django-environment", toolVersion="1.1.1",
                                 runtimeFingerprint=fingerprint()))


def summary(python, venv, distribution, module):
    return dict(pythonVersion=python, virtualEnvironment=venv,
                djangoDistributionVersion=distribution, djangoModuleOriginReported=module)


def fixtures():
    single = export(admin=False)
    single_checks = (
        dict(selected=summary("3.12.8", True, "5.1.7", True), comparison=None,
             pathPythonReported=True, pathDjangoAdminReported=False,
             expectedSiblingDjangoAdminReported=True, entryLayoutMismatch=None),
        ("本机文件中的历史快照", "对比环境：未采集", "捕获时 django-admin 入口位置：未判定"),
    )
    yield "one-snapshot-unknown-layout", compact(single), JSON, single_checks
    yield "bom-and-octet", b"\xef\xbb\xbf" + compact(single), OCTET, single_checks

    double = export(capture(module=False), capture("3.13.2", venv=False, distribution=None), mismatch=True)
    double_checks = (
        dict(selected=summary("3.12.8", True, "5.1.7", False),
             comparison=summary("3.13.2", False, None, True), entryLayoutMismatch=True),
        ("主环境：Python 3.12.8 · 虚拟环境 是 · Django 分发 5.1.7 · 模块来源 未记录",
         "对比环境：Python 3.13.2 · 虚拟环境 否 · Django 分发 未记录 · 模块来源 已记录",
         "捕获时 django-admin 入口位置：不同"),
    )
    yield "two-captures-independent-distribution-and-module", compact(double), JSON, double_checks

    same_layout = export(capture("3.13.0rc2", venv=False, distribution="1!4.2.11+mirror_2"), mismatch=False)
    yield "same-entry-and-version-suffixes", compact(same_layout), TEXT, (
        dict(selected=summary("3.13.0rc2", False, "1!4.2.11+mirror_2", True), entryLayoutMismatch=False),
        ("捕获时 django-admin 入口位置：未记录差异", "1!4.2.11+mirror_2"),
    )
    no_scripts = export(capture(distribution=None, module=False, scripts=False))
    no_scripts["pathPython"] = None
    yield "missing-python-and-expected-scripts", compact(no_scripts), JSON, (
        dict(selected=summary("3.12.8", True, None, False),
             pathPythonReported=False, pathDjangoAdminReported=True,
             expectedSiblingDjangoAdminReported=False, entryLayoutMismatch=None),
        ("PATH 中的 Python 未记录", "预期脚本位置 未记录"),
    )

    wide = dict(single, notes="")
    wide["notes"] = " " * (32 * 1024 - len(compact(wide)))
    exact = compact(wide)
    assert len(exact) == 32 * 1024
    yield "exactly-32-kib", exact, JSON, single_checks
    unicode_paths = export(capture(module=True))
    unicode_paths["selected"]["djangoImportOrigin"] = "fixture-private\n本机程序"
    unicode_paths["notes"] = "fixture-private 中文\U0001f642"
    yield "unicode-notes-and-paths-hidden", compact(unicode_paths), JSON, (
        dict(selected=summary("3.12.8", True, "5.1.7", True)),
        ("模块来源 已记录",),
    )

    wrong_tool = dict(single, generatedBy=dict(single["generatedBy"], toolId="java-environment"))
    bad_hash = dict(single, generatedBy=dict(single["generatedBy"],
                    runtimeFingerprint=dict(fingerprint(), sha256="0" * 64)))
    bad_tool_version = dict(single, generatedBy=dict(single["generatedBy"], toolVersion="1.01.1"))
    typed_bool = dict(single, selected=dict(single["selected"], virtualEnvironment="yes"))
    invalid_origin = dict(single, selected=dict(single["selected"], djangoImportOrigin=["fixture-private"]))
    invalid_dist = dict(single, selected=dict(single["selected"], django={"version":"5.0"}))
    duplicate = compact(single).replace(b'"version":"3.12.8"', b'"version":"3.12.8","version":"3.12.8"', 1)
    for name, raw, mime, code in [
        ("more-than-32-kib", exact + b" ", JSON, "INPUT_LIMIT"),
        ("wrong-media", compact(single), "image/png", "INVALID_RESOURCE"),
        ("invalid-utf8", b'{"notes":"\xff"}', JSON, "INVALID_REPORT"),
        ("missing-native-envelope", b'{"selected":{}}', JSON, "INVALID_REPORT"),
        ("unknown-private-key", compact(dict(single, private="fixture-private")), JSON, "INVALID_REPORT"),
        ("invalid-venv-flag", compact(typed_bool), JSON, "INVALID_REPORT"),
        ("invalid-origin-type", compact(invalid_origin), JSON, "INVALID_REPORT"),
        ("missing-distribution-location", compact(invalid_dist), JSON, "INVALID_REPORT"),
        ("duplicate-version-key", duplicate, JSON, "INVALID_REPORT"),
        ("wrong-tool-id", compact(wrong_tool), JSON, "INVALID_PROVENANCE"),
        ("changed-fingerprint", compact(bad_hash), JSON, "INVALID_PROVENANCE"),
        ("bad-tool-version", compact(bad_tool_version), JSON, "INVALID_PROVENANCE"),
        ("layout-claim-with-no-entry", compact(dict(single, entryLayoutMismatch=False)), JSON, "INCONSISTENT_LAYOUT"),
        ("layout-missing-with-both-entries", compact(dict(double, entryLayoutMismatch=None)), JSON, "INCONSISTENT_LAYOUT"),
        ("unsafe-executable-version", compact(export(capture(r"3.12:\fixture-private"))), JSON, "INVALID_FIELD"),
        ("unsafe-distribution-version", compact(export(capture(distribution="5.0\nhttp://fixture-private"))), JSON, "INVALID_FIELD"),
        ("overlong-version", compact(export(capture("3" * 97))), JSON, "INVALID_FIELD"),
        ("empty-version", compact(export(capture(" "))), JSON, "INVALID_FIELD"),
    ]:
        yield name, raw, mime, code



def verify(case,worker,native,module,scratch):
    name,source,mime,expected=case
    report,blob=run_worker(worker,module,case)
    assert (report,blob)==run_native(native,case,scratch),report
    serialized=compact(report)
    assert b'fixture-private' not in serialized and b'runtimeFingerprint' not in serialized
    if isinstance(expected,str):
        assert not report['ok'] and report['error']['code']==expected and blob is None,report
        error=expected
    else:
        fields,snippets=expected
        assert report['ok'] and report['error'] is None and blob,report
        assert all(report['data'][field]==value for field,value in fields.items()),report
        assert all(snippet in report['data']['text'] for snippet in snippets),report
        assert blob.decode('utf-8')==report['data']['text']
        assert b'fixture-private' not in blob and b'runtimeFingerprint' not in blob
        error=None
    return dict(case=name,mime=mime,inputBytes=len(source),sourceSha256=digest(source),code=error,
                outputBytes=len(blob) if blob else 0,outputSha256=digest(blob) if blob else None)


def adapter_header(case):
    _,source,mime,_=case
    resource=dict(resourceId="4fad13fe-807b-431b-8bbd-f032b26ee77e",mimeType=mime,byteLength=len(source),contentVersion=digest(source))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input={},resourceOutputProfile=OUTPUT)
    return request,resource


def run_native(native,case,scratch):
    request,resource=adapter_header(case)
    source,target=scratch/"source.json",scratch/"target.txt"
    source.write_bytes(case[1])
    envelope=dict(**request,commandId=None,resourceInputProfile=INPUT,resources=[dict(index=0,**resource)])
    process=subprocess.run([native,source,target],input=compact(envelope),capture_output=True,check=True,
                           timeout=12,creationflags=subprocess.CREATE_NO_WINDOW)
    assert source.read_bytes()==case[1]
    return json.loads(process.stdout),target.read_bytes() if target.exists() else None


def run_worker(worker,module,case):
    request,resource=adapter_header(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),resources=[resource],
                        module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case[1]
    process=subprocess.run([worker,"--zicode-plugin-worker"],input=frame,capture_output=True,
                           timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert process.returncode==0 and process.stdout.startswith(b"ZIWO\0\0\0\x01"),process.stdout[:350]
    end=12+struct.unpack_from("<I",process.stdout,8)[0]
    wire=json.loads(process.stdout[12:end])
    blob=process.stdout[end:]
    if wire["resources"]:
        resource,=wire["resources"]
        assert resource["mimeType"]==TEXT and resource["byteLength"]==len(blob) and resource["contentVersion"]==digest(blob)
    else:
        assert not blob
    return wire["result"],blob if blob else None


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            scratch=Path(temporary)/case[0]
            scratch.mkdir()
            try:
                cases.append(verify(case,args.worker,args.native,module,scratch))
            except (AssertionError,subprocess.SubprocessError,ValueError,TypeError) as error:
                failures.append(dict(case=case[0],message=str(error)[:500]))
    proof=dict(wasmSha256=digest(module),wasmBytes=len(module),workerSha256=digest(args.worker.read_bytes()),
               inputProfile=INPUT,outputProfile=OUTPUT,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(cases),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
