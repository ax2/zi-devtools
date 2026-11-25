"""Compare native and real Windows Worker file review with native report-shaped fixtures."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN="com.zicode.devtools.jdk-report"
CAPABILITY="devtools.java.environment.report.review"
TITLE="JDK 环境报告查看"
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


def capture(version,vendor='Sample OpenJDK',arch='amd64',javac=True):
    fields={'java.version':version,'java.home':r'X:\fixture-private\jdk'}
    if vendor is not None:fields['java.vendor']=vendor
    if arch is not None:fields['os.arch']=arch
    return dict(executable=r'X:\fixture-private\jdk\bin\java.exe',properties=fields,javacPresent=javac)


def export(selected=None,compared=None,mismatch=None):
    selected=selected or capture('21.0.3+9')
    difference=None if compared is None else selected['properties']['java.version']!=compared['properties']['java.version']
    return dict(selected=selected,comparison=compared,javaHome=r'X:\fixture-private\configured',javaHomeMismatch=mismatch,
        pathJava=r'Y:\fixture-private\bin\java.exe',mavenEntry=r'Z:\fixture-private\mvn.cmd',gradleEntry=None,
        versionMismatch=difference,notes='fixture-private raw capture notes',
        generatedBy=dict(toolId='java-environment',toolVersion='1.1.1',runtimeFingerprint=fingerprint()))


def fixtures():
    single=export()
    single_checks=(dict(selectedVersion='21.0.3+9',comparedVersion=None,versionMismatch=None,javaHomeMismatch=None,
                        pathJavaReported=True,mavenEntryReported=True,gradleEntryReported=False),
                   ('本机文件中的历史快照','对比环境：未采集','版本字符串：未判定 · 捕获时 JAVA_HOME：未判定'))
    yield 'one-snapshot-unknown-scope',compact(single),JSON,single_checks
    yield 'bom-and-octet',b'\xef\xbb\xbf'+compact(single),OCTET,single_checks
    two=export(compared=capture('17.0.12',vendor=None,arch='aarch64',javac=False),mismatch=False)
    two_checks=(dict(selectedVersion='21.0.3+9',comparedVersion='17.0.12',versionMismatch=True,javaHomeMismatch=False),
                ('主环境：21.0.3+9 · 供应商 Sample OpenJDK · 架构 amd64 · javac 已记录',
                 '对比环境：17.0.12 · 供应商 未报告 · 架构 aarch64 · javac 未记录','版本字符串：不同 · 捕获时 JAVA_HOME：一致'))
    yield 'two-captures-version-difference',compact(two),JSON,two_checks
    same=export(compared=capture('21.0.3+9',vendor='其他公司',arch=None,javac=False),mismatch=True)
    yield 'same-version-different-vendor',compact(same),TEXT,(dict(comparedVersion='21.0.3+9',versionMismatch=False,javaHomeMismatch=True),
            ('供应商 其他公司 · 架构 未报告','版本字符串：相同 · 捕获时 JAVA_HOME：不一致'))
    suffix=export(compared=capture('21.0.3+10'),mismatch=True)
    yield 'build-suffix-is-compared-exactly',compact(suffix),JSON,(dict(versionMismatch=True),('21.0.3+10',))
    entries=export()
    entries['pathJava']=None
    entries['mavenEntry']=None
    entries['gradleEntry']=r'Z:\fixture-private\gradle.exe'
    yield 'captured-entry-presence-only',compact(entries),JSON,(dict(pathJavaReported=False,mavenEntryReported=False,gradleEntryReported=True),
            ('捕获时入口 · Java 未记录 · Maven 未记录 · Gradle 已记录',))
    wide=dict(single,notes='')
    extra=32*1024-len(compact(wide))
    wide['notes']='A'*extra
    exact=compact(wide)
    assert len(exact)==32*1024
    yield 'exact-file-byte-limit-notes-omitted',exact,JSON,single_checks
    dropped=export(selected=capture('1.8.0_412',vendor=None,arch=None))
    yield 'original-jdk8-labels',compact(dropped),JSON,(dict(selectedVersion='1.8.0_412'),('供应商 未报告 · 架构 未报告',))
    other=dict(single,generatedBy=dict(single['generatedBy'],toolId='django-environment'))
    invalid_hash=dict(single,generatedBy=dict(single['generatedBy'],runtimeFingerprint=dict(fingerprint(),sha256='0'*64)))
    bad_version=dict(single,generatedBy=dict(single['generatedBy'],toolVersion='1.01.1'))
    malformed=dict(single,selected=dict(single['selected'],javacPresent='yes'))
    for name,raw,mime,error in [
        ('more-than-32-kib',exact+b' ',JSON,'INPUT_LIMIT'),
        ('wrong-media',compact(single),'image/png','INVALID_RESOURCE'),
        ('invalid-utf8',b'{"invalid":"\xff"}',JSON,'INVALID_REPORT'),
        ('missing-native-envelope',b'{"selected":{}}',JSON,'INVALID_REPORT'),
        ('unexpected-private-field',compact(dict(single,private='fixture-private')),JSON,'INVALID_REPORT'),
        ('untyped-boolean',compact(malformed),JSON,'INVALID_REPORT'),
        ('wrong-tool-id',compact(other),JSON,'INVALID_PROVENANCE'),
        ('changed-fingerprint',compact(invalid_hash),JSON,'INVALID_PROVENANCE'),
        ('bad-tool-version',compact(bad_version),JSON,'INVALID_PROVENANCE'),
        ('comparison-flag-without-two',compact(dict(single,versionMismatch=False)),JSON,'INCONSISTENT_COMPARISON'),
        ('altered-comparison-claim',compact(dict(two,versionMismatch=False)),JSON,'INCONSISTENT_COMPARISON'),
        ('unsafe-vendor-path',compact(export(selected=capture('21',vendor=r'X:\fixture-private\ignored'))),JSON,'INVALID_FIELD'),
        ('unsafe-control-character',compact(export(selected=capture('21',arch='arm\nprivate'))),JSON,'INVALID_FIELD'),
        ('oversize-version',compact(export(selected=capture('v'*97))),JSON,'INVALID_FIELD'),
        ('empty-version',compact(export(selected=capture('  '))),JSON,'INVALID_FIELD'),
    ]:
        yield name,raw,mime,error


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
