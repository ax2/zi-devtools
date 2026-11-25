"""Check knowledge source preflight through its native adapter and Windows production Worker."""

import argparse
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke
from verify_rename_preview_plugin import folder

PLUGIN = "com.zicode.devtools.knowledge-preflight"
CAPABILITY = "devtools.knowledge.source.preflight"
PROFILE = "resource.wasi.input.draft2"
MIME = "text/plain"
MAX_ENTRIES = 64
MAX_INPUT = 64 * 1024
MAX_RESULT = 24 * 1024
MAX_SHOWN = 24


def report(files, excluded=0, unsupported=0, directories=0):
    counts = dict.fromkeys(("markdown", "text", "html", "pdf", "docx", "image"), 0)
    sorted_files = sorted(files)
    for _, group in sorted_files:
        counts[group] += 1
    eligible = len(sorted_files)
    return dict(ok=True, fileContentsChecked=False, entryCount=eligible+excluded+unsupported+directories,
                eligibleCount=eligible, excludedCount=excluded, unsupportedCount=unsupported,
                directoriesNotScanned=directories, candidatesTruncated=eligible > MAX_SHOWN,
                formatCounts=counts, candidates=[dict(name=name,format=group)
                                                for name, group in sorted_files[:MAX_SHOWN]])


def fixtures():
    files = [("稿件.MD","markdown"), ("appendix.Markdown","markdown"), ("draft.TXT","text"),
             ("site.HTM","html"), ("Guide.HTML","html"), ("账单.Pdf","pdf"),
             ("brief.DoCx","docx"), ("photo.PNG","image"), ("map.jpg","image"),
             ("map-large.JPEG","image"), ("cover.WebP","image")]
    directories = [".GIT", "node_modules", "target", ".venv", "__pycache__", "archive.pdf"]
    names = [file for file, _ in files] + ["animation.gif", "配置.env", ".md", "settings.json"]
    mixed = folder(*names, directories=directories)
    yield "native-format-list-and-default-excludes", mixed, {}, MIME, report(files, 5, 4, 1)
    rest = [(name, group) for name,group in files if name not in {"draft.TXT","稿件.MD","map.jpg"}]
    yield "custom-rule-wildcards-case-and-folder", mixed, dict(excludes="*.MD;draft.txt;archive.pdf/;map.JPG"), MIME, report(rest, 10, 3)
    yield "default-excluded-leaf-file", folder("node_modules","real.pdf"), {}, MIME, report([("real.pdf","pdf")],1)
    yield "directory-with-document-extension", folder(directories=["docs.md"]), {}, MIME, report([],directories=1)
    yield "empty-folder", folder(), {}, MIME, report([])
    yield "no-contents-needed-for-candidate", folder("fake.PDF"), {}, MIME, report([("fake.PDF","pdf")])
    yield "invalid-rule-parent-relative", mixed, dict(excludes="../private.txt"), MIME, "INVALID_RULES"
    yield "unsupported-wildcard", mixed, dict(excludes="*.p*d"), MIME, "INVALID_RULES"
    yield "eight-rule-limit", folder("ok.txt"), dict(excludes=";".join(f"ignored{i}" for i in range(8))), MIME, report([("ok.txt","text")])
    yield "one-rule-over", mixed, dict(excludes=";".join(f"ignored{i}" for i in range(9))), MIME, "RULE_BUDGET_EXCEEDED"
    yield "rule-utf8-budget", mixed, dict(excludes="规"*321), MIME, "RULE_BUDGET_EXCEEDED"
    yield "wrong-snapshot-profile", compact(dict(schemaVersion=1,profile="incoming-file",entries=[])), {}, MIME, "INVALID_SNAPSHOT"
    yield "unknown-entry-type", compact(dict(schemaVersion=1,profile="resource.local-directory.draft1",
                                             entries=[dict(name="test.txt",kind="symlink")])), {}, MIME, "INVALID_SNAPSHOT"
    yield "file-path-escape", folder("../private.txt"), {}, MIME, "INVALID_SNAPSHOT"
    yield "duplicate-casefolded-name", folder("same.MD","SAME.md"), {}, MIME, "INVALID_SNAPSHOT"
    yield "non-text-resource", mixed, {}, "application/octet-stream", "UNSUPPORTED_MIME"
    yield "extra-execution-argument", mixed, dict(scan=True), MIME, "INVALID_REQUEST"
    yield "bad-utf8-snapshot", b"\xff\xfe", {}, MIME, "INVALID_SNAPSHOT"
    names = [f"f{i:02d}.md" for i in range(MAX_ENTRIES)]
    exact = folder(*names).ljust(MAX_INPUT,b" ")
    yield "entry-and-snapshot-byte-limits", exact, {}, MIME, report([(name,"markdown") for name in names])
    yield "one-entry-over", folder(*names,"extra.txt"), {}, MIME, "TOO_MANY_ENTRIES"
    yield "one-snapshot-byte-over", exact + b" ", {}, MIME, "INPUT_TOO_LARGE"

    unicode_names = [f"{i:02d}"+("界"*65)+".txt" for i in range(60)]
    remaining = 12*1024 - sum(len(name.encode("utf-8")) for name in unicode_names)
    unicode_names.append("z"*(remaining-4)+".txt")
    assert sum(len(name.encode("utf-8")) for name in unicode_names) == 12*1024
    large_rules = ";".join(str(i)+"词"*39+("xx" if i==7 else "x") for i in range(8))
    assert len(large_rules.encode("utf-8")) == 960
    yield ("exact-unicode-name-and-rule-byte-limits",folder(*unicode_names),dict(excludes=large_rules),MIME,
           report([(name,"text") for name in unicode_names]))
    yield "one-name-byte-over", folder(*unicode_names,"r"), {}, MIME, "NAME_BUDGET_EXCEEDED"


def envelopes(module, payload, options, mime):
    descriptor = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                      mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=options)
    native = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **descriptor)]))
    header = compact(dict(profile=PROFILE,request=dict(workerKind="wasi",workerPath="",**common),
                          module=dict(byteLength=len(module),sha256=digest(module)),resources=[descriptor]))
    return native, b"ZIRW\0\0\0\x02" + struct.pack("<I",len(header)) + header + module + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args = parser.parse_args()
    module = args.module.read_bytes()
    results = []
    with TemporaryDirectory() as temp:
        file = Path(temp) / "directory-snapshot.json"
        for case,payload,options,mime,expected in fixtures():
            file.write_bytes(payload)
            native_envelope,worker_envelope = envelopes(module,payload,options,mime)
            native = invoke([args.native,file],native_envelope)
            try:
                worker = invoke([args.worker,"--zicode-plugin-worker"],worker_envelope)
            except RuntimeError as error:
                raise RuntimeError(f"{case}: {error}") from error
            assert worker["ok"], (case,worker)
            output = worker["result"]
            assert output == native, (case,"native/WASI mismatch")
            if isinstance(expected,str):
                assert not output["ok"] and output["error"]["code"] == expected, (case,output)
            else:
                assert output == expected, (case,output)
            serialized = compact(output)
            assert len(serialized) <= MAX_RESULT and file.read_bytes() == payload
            results.append(dict(case=case,inputSha256=digest(payload),inputBytes=len(payload),
                                input=options,mime=mime,ok=output["ok"],resultSha256=digest(serialized)))
            print(case,"passed",flush=True)
    receipt = dict(passed=len(results),wasmSha256=digest(module),workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE,cases=results)
    args.evidence.write_text(json.dumps(receipt,indent=2)+"\n",encoding="utf-8")


if __name__ == "__main__":
    main()
