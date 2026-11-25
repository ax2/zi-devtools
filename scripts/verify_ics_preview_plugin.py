"""Verify the read-only ICS preview through native and production Windows Worker."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.ics-preview"
CAPABILITY = "devtools.calendar.ics.preview"
PROFILE = "resource.wasi.input.draft2"
MAX_FILE = 16 * 1024
MAX_TEXT = 6 * 1024
MAX_EVENTS_SHOWN = 8
MAX_RESULT = 48 * 1024
MIME = "application/octet-stream"
HEAD = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ZiCode//Synthetic Preview//EN\r\n"
END = "END:VCALENDAR\r\n"


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def calendar(body="", bom=False):
    return (("\ufeff" if bom else "") + HEAD + body + END).encode()


def event(start, summary=None, *, tail="", end=None):
    fields = "DTSTART" + start + "\r\n"
    if end:
        fields += "DTEND" + end + "\r\n"
    if summary is not None:
        fields += f"SUMMARY:{summary}\r\n"
    return "BEGIN:VEVENT\r\n" + fields + tail + "END:VEVENT\r\n"


def time(value, kind, timezone=None):
    return dict(value=value,kind=kind,timezone=timezone)


def entry(summary, start, end=None, *, rule=None, exceptions=False, shortened=False):
    return dict(summary=summary,summaryTruncated=shortened,start=start,end=end,
                recurrenceRule=rule,recurrenceExceptionsPresent=exceptions)


def report(source, entries, count=None, recurring=0):
    count = len(entries) if count is None else count
    return dict(ok=True,sourceByteCount=len(source),eventCount=count,recurringEventCount=recurring,
                recurrenceExpanded=False,truncated=len(entries)<count,events=entries)


def fixtures():
    all_day = event(";VALUE=DATE:20261225", r"节日\,安排", end=";VALUE=DATE:20261226")
    utc = event(":20261225T101500Z", "UTC 项",
                end=":20261225T111500Z", tail="BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\n")
    floating = event(":20261225T132000", r"本地\;标题\n折" + "\r\n 行 ")
    named = event(";TZID=Asia/Shanghai:20261226T090000","时区原样",
                  end=";TZID=Asia/Shanghai:20261226T100000",
                  tail="RRULE:FREQ=WEEKLY;COUNT=3\r\nEXDATE;TZID=Asia/Shanghai:20270102T090000\r\n")
    source = calendar(all_day+utc+floating+named, bom=True)
    expected = [
        entry("节日,安排",time("20261225","all-day"),time("20261226","all-day")),
        entry("UTC 项",time("20261225T101500Z","utc"),time("20261225T111500Z","utc")),
        entry("本地;标题\n折行 ",time("20261225T132000","floating")),
        entry("时区原样",time("20261226T090000","named-timezone","Asia/Shanghai"),
              time("20261226T100000","named-timezone","Asia/Shanghai"),rule="FREQ=WEEKLY;COUNT=3",exceptions=True),
    ]
    yield "four-time-kinds-folded-bom", source, {}, MIME, report(source,expected,recurring=1)

    value = "界" * 172
    raw = calendar(event(":20261225T093000",value))
    yield "summary-clips-at-character-boundary", raw, {}, "text/plain", report(
        raw,[entry("界"*170,time("20261225T093000","floating"),shortened=True)]
    )

    index = time("20261225T093000","floating")
    many = calendar("".join(event(":20261225T093000",str(i)) for i in range(12)))
    yield "count-beyond-eight-displayed", many, {}, MIME, report(
        many,[entry(str(i),index) for i in range(MAX_EVENTS_SHOWN)],12
    )

    name = "Fixture/" + "z"*120
    stamp = ";TZID=" + name + ":20261225T093000"
    rule = "FREQ=DAILY;X-FIXTURE=" + "x"*490
    row = event(stamp,"s"*512,end=stamp,tail="RRULE:"+rule+"\r\n")
    long = calendar(row*8)
    visible = MAX_TEXT//(512+2*(len("20261225T093000")+len(name))+len(rule))
    zone = time("20261225T093000","named-timezone",name)
    yield "displayed-text-budget", long, {}, MIME, report(
        long,[entry("s"*512,zone,zone,rule=rule) for _ in range(visible)],8,8
    )

    empty = calendar("BEGIN:VTODO\r\nSUMMARY:Ignored task\r\nEND:VTODO\r\n")
    yield "valid-empty-calendar", empty, {}, MIME, report(empty,[])

    line = "COMMENT:\r\n"
    pad = MAX_FILE - len(calendar(line))
    largest = calendar("COMMENT:"+"x"*pad+"\r\n")
    assert len(largest) == MAX_FILE
    yield "exact-file-boundary", largest, {}, MIME, report(largest,[])
    yield "above-file-boundary", largest+b"x", {}, MIME, "INPUT_TOO_LARGE"

    size_event = event(":20261225T093000")
    count = (MAX_FILE-len(calendar())) // len(size_event)
    dense = calendar(size_event*count)
    yield "dense-event-file", dense, {}, MIME, report(
        dense,[entry("",index) for _ in range(MAX_EVENTS_SHOWN)],count
    )
    yield "invalid-utf8", b"\xff", {}, MIME, "INVALID_UTF8"
    yield "missing-start", calendar("BEGIN:VEVENT\r\nSUMMARY:no date\r\nEND:VEVENT\r\n"), {}, MIME, "MISSING_START"
    yield "impossible-date", calendar(event(";VALUE=DATE:20260230")), {}, MIME, "INVALID_TIME"
    yield "utc-and-tzid-are-ambiguous", calendar(event(";TZID=Asia/Shanghai:20261225T093000Z")), {}, MIME, "INVALID_TIME"
    yield "duplicate-start", calendar(event(":20261225T093000",tail="DTSTART:20261225T091500\r\n")), {}, MIME, "INVALID_CALENDAR"
    yield "unterminated-event", calendar("BEGIN:VEVENT\r\nDTSTART:20261225T093000\r\n"), {}, MIME, "INVALID_CALENDAR"
    yield "oversized-rrule", calendar(event(":20261225T093000",tail="RRULE:"+"x"*513+"\r\n")), {}, MIME, "PROPERTY_TOO_LARGE"
    yield "wrong-mime", source, {}, "image/png", "UNSUPPORTED_MIME"
    yield "unexpected-arguments", source, {"import":True}, MIME, "INVALID_REQUEST"


def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module), sha256=digest(module)), resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I", len(header)) + header + module + payload


def invoke(command, request):
    result = subprocess.run(command, input=request, capture_output=True, timeout=9,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    if result.returncode:
        raise RuntimeError(f"exit={result.returncode}, {result.stdout[:1200]!r}, {result.stderr[-1500:]!r}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("native", "module", "worker", "evidence"):
        parser.add_argument("--" + name, required=True, type=Path)
    args = parser.parse_args()
    module = args.module.read_bytes()
    results = []
    with TemporaryDirectory() as temp:
        file = Path(temp) / "owned-fixture.ics"
        for case, payload, inputs, mime, expected in fixtures():
            file.write_bytes(payload)
            native, wire = envelopes(module, payload, inputs, mime)
            direct = invoke([args.native, file], native)
            try:
                worker = invoke([args.worker, "--zicode-plugin-worker"], wire)
            except RuntimeError as error:
                raise RuntimeError(f"{case}: {error}") from error
            assert worker["ok"], (case, worker)
            output = worker["result"]
            assert output == direct, (case, "native/WASI mismatch")
            if isinstance(expected, str):
                assert not output["ok"] and output["error"]["code"] == expected, (case, output)
            else:
                assert output == expected, (case, output)
            serialized = compact(output)
            assert len(serialized) <= MAX_RESULT
            results.append(dict(case=case, inputSha256=digest(payload), inputBytes=len(payload),
                                input=inputs, mime=mime, ok=output["ok"], resultSha256=digest(serialized)))
            print(case, "passed", flush=True)
    receipt = dict(passed=len(results), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=results)
    args.evidence.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
