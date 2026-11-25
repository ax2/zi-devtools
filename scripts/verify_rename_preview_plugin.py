"""Verify the rename preview with native code and the production Windows Worker."""

import argparse
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke
import json

PLUGIN = "com.zicode.devtools.rename-preview"
CAPABILITY = "devtools.files.rename.preview"
PROFILE = "resource.wasi.input.draft2"
DIRECTORY = "resource.local-directory.draft1"
MIME = "text/plain"
MAX_ENTRIES = 64
MAX_INPUT = 64 * 1024
MAX_RESULT = 48 * 1024


def folder(*names, directories=()):
    return compact(dict(schemaVersion=1, profile=DIRECTORY, entries=[
        *[dict(name=name, kind="file") for name in names],
        *[dict(name=name, kind="directory") for name in directories],
    ]))


def row(source, target, status="proposed", issue=None):
    return dict(source=source, target=target, status=status, issue=issue)


def report(count, *rows):
    proposed = sum(entry["status"] == "proposed" for entry in rows)
    unchanged = sum(entry["status"] == "unchanged" for entry in rows)
    return dict(ok=True, previewOnly=True, entryCount=count, fileCount=len(rows), proposedCount=proposed,
                unchangedCount=unchanged, blockedCount=len(rows)-proposed-unchanged,
                rows=list(rows))


def fixtures():
    base = folder("File.txt", "原始.tar.gz", ".readme", "NOEXT", directories=["Docs"])
    yield "no-change", base, {}, MIME, report(5, row("File.txt", "File.txt", "unchanged"),
        row("原始.tar.gz", "原始.tar.gz", "unchanged"), row(".readme", ".readme", "unchanged"),
        row("NOEXT", "NOEXT", "unchanged"))
    yield "apply-prefix-suffix-preserve-final-extension", base, dict(prefix="新-", suffix="-保存"), MIME, report(5,
        row("File.txt","新-File-保存.txt"), row("原始.tar.gz","新-原始.tar-保存.gz"),
        row(".readme","新-.readme-保存"), row("NOEXT","新-NOEXT-保存"))
    yield "case-only", folder("File.txt"), dict(find="File", replacement="file", caseSensitive=True), MIME, report(1,
        row("File.txt", "file.txt", "case-only"))
    yield "different-targets-after-replacement", folder("one-one.txt","two-two.txt"), dict(find="-", replacement=""), MIME, report(2,
        row("one-one.txt","oneone.txt"), row("two-two.txt","twotwo.txt"))
    yield "duplicate-after-overlapping-removal", folder("a.txt","aa.txt","aaa.txt"), dict(find="aa", replacement=""), MIME, report(3,
        row("a.txt","a.txt","unchanged"), row("aa.txt",".txt"), row("aaa.txt","a.txt","occupied"))
    yield "same-target-from-two-originals", folder("ab.txt","aab.txt"), dict(find="a", replacement=""), MIME, report(2,
        row("ab.txt","b.txt","duplicate"), row("aab.txt","b.txt","duplicate"))
    yield ("target-is-existing-directory", folder("draft.txt", directories=["FINAL.TXT"]),
           dict(find="draft", replacement="final"), MIME, report(2,row("draft.txt","final.txt","occupied")))
    yield "occupied-source-is-not-overwritten", folder("a.txt","ab.txt"), dict(find="b", replacement=""), MIME, report(2,
        row("a.txt","a.txt","unchanged"), row("ab.txt","a.txt","occupied"))
    yield "windows-device-target", folder("source.txt"), dict(find="source", replacement="CON"), MIME, report(1,
        row("source.txt",None,"invalid","目标文件名是 Windows 保留设备名"))
    yield "path-not-a-name", folder("../escape.txt"), {}, MIME, "INVALID_SNAPSHOT"
    yield "case-duplicate-snapshot-is-ambiguous", folder("FILE.txt","file.txt"), {}, MIME, "INVALID_SNAPSHOT"
    yield "wrong-profile", compact(dict(schemaVersion=1,profile="unverified",entries=[])), {}, MIME, "INVALID_SNAPSHOT"
    yield "wrong-kind", compact(dict(schemaVersion=1,profile=DIRECTORY,entries=[dict(name="x",kind="symlink")])), {}, MIME, "INVALID_SNAPSHOT"
    yield "extra-path-not-allowed", compact(dict(schemaVersion=1, profile=DIRECTORY, entries=[],path="C:/")), {}, MIME, "INVALID_SNAPSHOT"
    yield "empty-folder", folder(), {}, MIME, report(0)
    names = [f"p{index}.txt" for index in range(MAX_ENTRIES)]
    exact = folder(*names).ljust(MAX_INPUT, b" ")
    yield "exact-snapshot-byte-and-entry-limit", exact, dict(suffix="-new"), MIME, report(MAX_ENTRIES,
        *[row(name, name.removesuffix(".txt")+"-new.txt") for name in names])
    yield "one-entry-over", folder(*names, "extra"), {}, MIME, "TOO_MANY_ENTRIES"
    yield "one-byte-over", exact+b" ", {}, MIME, "INPUT_TOO_LARGE"
    yield "option-over-bytes-within-128-codepoints", folder("x"), dict(suffix="测"*43), MIME, "RULE_TOO_LARGE"
    yield "unexpected-write-parameter", base, dict(apply=True), MIME, "INVALID_REQUEST"
    yield "not-a-text-snapshot", base, {}, "application/octet-stream", "UNSUPPORTED_MIME"
    long_names = [f"{index}"+("界"*240)+".txt" for index in range(MAX_ENTRIES)]
    assert len(folder(*long_names)) < MAX_INPUT
    yield "aggregate-long-name-limit", folder(*long_names), {}, MIME, "NAME_BUDGET_EXCEEDED"
    unicode_names = [f"{index:02d}"+("界"*65)+".txt" for index in range(60)]
    unicode_names.append("末"*25)
    remaining = 12*1024 - sum(len(name.encode("utf-8")) for name in unicode_names)
    assert 0 <= remaining <= 255
    unicode_names.extend(["r"*remaining] if remaining else [])
    assert sum(len(name.encode("utf-8")) for name in unicode_names) == 12*1024
    yield "exact-aggregate-unicode-no-op", folder(*unicode_names), {}, MIME, report(len(unicode_names),
        *[row(name,name,"unchanged") for name in unicode_names])
    yield "exact-aggregate-unicode-rename", folder(*unicode_names), dict(prefix="复-"), MIME, report(len(unicode_names),
        *[row(name,"复-"+name) for name in unicode_names])
    yield "aggregate-one-byte-over", folder(*unicode_names,"z"), {}, MIME, "NAME_BUDGET_EXCEEDED"
    worst_names = [str(i)+"a"*188 for i in range(MAX_ENTRIES)]
    assert sum(len(name) for name in worst_names) <= 12*1024
    issue = "目标文件名超过 255 个 UTF-16 单元"
    yield ("case-insensitive-replacement-amplification", folder(*worst_names),
           dict(find="a",replacement="z"*128), MIME, report(MAX_ENTRIES,*[row(name,None,"invalid",issue) for name in worst_names]))

    small_names = [str(index)+"-"+"a"*4 for index in range(MAX_ENTRIES)]
    ascii_targets = [name.replace("a","r"*63) for name in small_names]
    assert sum(map(len,ascii_targets)) <= 16*1024
    yield ("near-target-budget", folder(*small_names), dict(find="a",replacement="r"*63), MIME,
           report(MAX_ENTRIES,*[row(name,target) for name,target in zip(small_names,ascii_targets)]))
    yield ("aggregate-target-amplification", folder(*small_names), dict(find="a",replacement="替"*42), MIME,
           "TARGET_BUDGET_EXCEEDED")

def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module), sha256=digest(module)), resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + module + payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    module = args.module.read_bytes()
    results = []
    with TemporaryDirectory() as temp:
        file = Path(temp) / "directory-snapshot.json"
        for case, payload, inputs, mime, expected in fixtures():
            file.write_bytes(payload)
            native_envelope, worker_envelope = envelopes(module, payload, inputs, mime)
            native = invoke([args.native, file], native_envelope)
            worker = invoke([args.worker, "--zicode-plugin-worker"], worker_envelope)
            assert worker["ok"], (case, worker)
            output = worker["result"]
            assert output == native, (case, "native/WASI mismatch")
            if isinstance(expected, str):
                assert not output["ok"] and output["error"]["code"] == expected, (case, output)
            else:
                assert output == expected, (case, output)
            serialized = compact(output)
            assert len(serialized) <= MAX_RESULT and file.read_bytes() == payload
            results.append(dict(case=case, inputSha256=digest(payload), inputBytes=len(payload),
                                input=inputs, mime=mime, ok=output["ok"], resultSha256=digest(serialized)))
            print(case, "passed", flush=True)
    receipt = dict(passed=len(results), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=results)
    args.evidence.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
