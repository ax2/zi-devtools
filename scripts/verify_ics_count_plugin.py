"""Replay synthetic COUNT calendars through native code and the production Windows Worker."""

import argparse
from datetime import date
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.ics-count"
CAPABILITY = "devtools.calendar.ics.count_cutoff"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_FILE = 16 * 1024
MAX_RESULT = 2048


def calendar(start="DTSTART:20260301T090000", rule="FREQ=DAILY;COUNT=3", *, extras=(),
             calendar_fields=(), duplicate=False, include_event=True):
    fields = ["BEGIN:VCALENDAR", "VERSION:2.0", "CALSCALE:GREGORIAN", *calendar_fields]
    if include_event:
        event = ["BEGIN:VEVENT", "UID:fixture@invalid", "SUMMARY: HIDDEN-FIXTURE",
                 "DESCRIPTION:IGNORED-FIXTURE", start]
        if rule is not None:
            event.append(rule if rule.startswith("RRULE") else f"RRULE:{rule}")
        event.extend([*extras, "END:VEVENT"])
        fields.extend(event)
        if duplicate:
            fields.extend(event)
    return ("\r\n".join([*fields, "END:VCALENDAR"]) + "\r\n").encode()


def report(count, frequency, start, last, until, *, kind="floating", clamp=False):
    frequency_label = {"daily": "每天", "weekly": "每周", "monthly": "每月", "yearly": "每年"}[frequency]
    time = "全天" if kind == "all-day" else "浮动时间"
    completion = ("仅一次发生，转换后可使用单次日程。" if until is None
                  else f"将原 RRULE 的 COUNT 替换为 UNTIL={until}，保留其他条件。末次开始计入重复。")
    text = f"{time} · {frequency_label} · {count} 次（包含首次）\n首次开始 {start}\n最后开始 {last}\n{completion}"
    return dict(count=count, frequency=frequency, timeKind=kind, start=start, lastStart=last,
                equivalentUntil=until, clampMissingDay=clamp, text=text)


def fixtures():
    base = report(3, "daily", "2026-03-01 09:00", "2026-03-03 09:00", "20260303T090000")
    yield "floating-daily-includes-dtstart", calendar(), {}, MIME, base
    yield "floating-daily-plain-mime", calendar(), {}, "text/plain", base
    yield "all-day-daily", calendar("DTSTART;VALUE=DATE:20260301"), {}, MIME, report(
        3, "daily", "2026-03-01", "2026-03-03", "20260303", kind="all-day")
    yield "count-one-keeps-source-frequency-in-report", calendar(rule="FREQ=YEARLY;COUNT=1"), {}, MIME, report(
        1, "yearly", "2026-03-01 09:00", "2026-03-01 09:00", None)
    yield "supported-with-value-parameter", calendar(rule="RRULE;VALUE=RECUR:FREQ=DAILY;COUNT=3"), {}, MIME, base
    yield "folded-rule", calendar(rule="RRULE:FREQ=DAILY;\r\n COUNT=3"), {}, MIME, base
    yield "bom-and-folded-rule", b"\xef\xbb\xbf" + calendar(rule="RRULE:FREQ=DAILY;\r\n COUNT=3"), {}, MIME, base
    yield "weekly-same-dtstart-weekday", calendar("DTSTART:20261002T093000",
        "FREQ=WEEKLY;COUNT=3;BYDAY=FR;WKST=SU;INTERVAL=1"), {}, MIME, report(
        3, "weekly", "2026-10-02 09:30", "2026-10-16 09:30", "20261016T093000")
    yield "monthly-missing-day-is-skipped", calendar("DTSTART;VALUE=DATE:20260131",
        "FREQ=MONTHLY;COUNT=3;BYMONTHDAY=31"), {}, MIME, report(
        3, "monthly", "2026-01-31", "2026-05-31", "20260531", kind="all-day")
    yield "monthly-clamped-missing-day", calendar("DTSTART;VALUE=DATE:20260131",
        "FREQ=MONTHLY;COUNT=2;BYMONTHDAY=28,29,30,31;BYSETPOS=-1"), {}, MIME, report(
        2, "monthly", "2026-01-31", "2026-02-28", "20260228", kind="all-day", clamp=True)
    yield "yearly-leap-is-skipped", calendar("DTSTART;VALUE=DATE:20240229", "FREQ=YEARLY;COUNT=2"), {}, MIME, report(
        2, "yearly", "2024-02-29", "2028-02-29", "20280229", kind="all-day")
    yield "yearly-clamp-at-february", calendar("DTSTART;VALUE=DATE:20240229",
        "FREQ=YEARLY;COUNT=2;BYMONTH=2;BYMONTHDAY=28,29;BYSETPOS=-1"), {}, MIME, report(
        2, "yearly", "2024-02-29", "2025-02-28", "20250228", kind="all-day", clamp=True)
    horizon = (date(2099,12,31)-date(1901,1,1)).days + 1
    yield "exact-daily-horizon", calendar("DTSTART;VALUE=DATE:19010101", f"FREQ=DAILY;COUNT={horizon}"), {}, MIME, report(
        horizon, "daily", "1901-01-01", "2099-12-31", "20991231", kind="all-day")
    yield "daily-horizon-plus-one", calendar("DTSTART;VALUE=DATE:19010101", f"FREQ=DAILY;COUNT={horizon + 1}"), {}, MIME, "INVALID_RULE"
    yield "monthly-full-budget", calendar("DTSTART;VALUE=DATE:19010131", "FREQ=MONTHLY;COUNT=1393"), {}, MIME, report(
        1393, "monthly", "1901-01-31", "2099-12-31", "20991231", kind="all-day")
    yield "monthly-horizon-plus-one", calendar("DTSTART;VALUE=DATE:19010131", "FREQ=MONTHLY;COUNT=1394"), {}, MIME, "INVALID_RULE"
    yield "rule-upper-u32-count", calendar(rule="FREQ=DAILY;COUNT=4294967295"), {}, MIME, "INVALID_RULE"
    yield "annual-horizon-outside", calendar("DTSTART;VALUE=DATE:20960229", "FREQ=YEARLY;COUNT=2"), {}, MIME, "INVALID_RULE"
    yield "alarm-ignored", calendar(extras=("BEGIN:VALARM","ACTION:DISPLAY","DESCRIPTION:private-reminder","TRIGGER:-PT5M",
        "END:VALARM")), {}, MIME, base
    yield "count-and-until", calendar(rule="FREQ=DAILY;COUNT=3;UNTIL=20260304T090000"), {}, MIME, "INVALID_RULE"
    yield "no-count", calendar(rule="FREQ=DAILY;UNTIL=20260304T090000"), {}, MIME, "COUNT_REQUIRED"
    yield "no-rule", calendar(rule=None), {}, MIME, "COUNT_REQUIRED"
    yield "duplicate-rule-count", calendar(rule="FREQ=DAILY;COUNT=3;COUNT=3"), {}, MIME, "INVALID_RULE"
    yield "second-rrule", calendar(extras=("RRULE:FREQ=DAILY;COUNT=4",)), {}, MIME, "INVALID_CALENDAR"
    yield "zero-count", calendar(rule="FREQ=DAILY;COUNT=0"), {}, MIME, "INVALID_RULE"
    yield "negative-count", calendar(rule="FREQ=DAILY;COUNT=-1"), {}, MIME, "INVALID_RULE"
    yield "unsupported-frequency", calendar(rule="FREQ=HOURLY;COUNT=2"), {}, MIME, "INVALID_RULE"
    yield "unsupported-interval", calendar(rule="FREQ=DAILY;COUNT=3;INTERVAL=2"), {}, MIME, "INVALID_RULE"
    yield "nonmatching-weekday", calendar(rule="FREQ=WEEKLY;COUNT=3;BYDAY=MO"), {}, MIME, "INVALID_RULE"
    yield "several-weekdays", calendar(rule="FREQ=WEEKLY;COUNT=3;BYDAY=SU,MO"), {}, MIME, "INVALID_RULE"
    yield "yearly-month-gap-invalid", calendar(rule="FREQ=YEARLY;COUNT=2;BYMONTHDAY=1"), {}, MIME, "INVALID_RULE"
    yield "bare-date-is-ambiguous", calendar("DTSTART:20260301"), {}, MIME, "INVALID_START"
    yield "nonminute-start", calendar("DTSTART:20260301T093012"), {}, MIME, "INVALID_START"
    yield "invalid-start-year", calendar("DTSTART;VALUE=DATE:19001231"), {}, MIME, "INVALID_START"
    yield "utc-is-not-floating", calendar("DTSTART:20260301T090000Z"), {}, MIME, "UNSUPPORTED_TIME"
    yield "tzid-is-not-floating", calendar("DTSTART;TZID=Asia/Shanghai:20260301T090000"), {}, MIME, "UNSUPPORTED_TIME"
    yield "calendar-tz-hint", calendar(calendar_fields=("X-WR-TIMEZONE:Asia/Shanghai",)), {}, MIME, "UNSUPPORTED_TIME"
    yield "tz-component-rejected", calendar(calendar_fields=("BEGIN:VTIMEZONE","TZID:Named","END:VTIMEZONE")), {}, MIME, "UNSUPPORTED_COMPONENT"
    yield "invitation-policy", calendar(calendar_fields=("METHOD:REQUEST",)), {}, MIME, "UNSUPPORTED_CALENDAR"
    yield "exception-date", calendar(extras=("EXDATE:20260302T090000",)), {}, MIME, "UNSUPPORTED_EXCEPTION"
    yield "additional-date", calendar(extras=("RDATE:20260304T090000",)), {}, MIME, "UNSUPPORTED_EXCEPTION"
    yield "example-cancelled", calendar(extras=("STATUS:CANCELLED",)), {}, MIME, "UNSUPPORTED_EVENT_STATUS"
    yield "two-events-ambiguous", calendar(duplicate=True), {}, MIME, "SINGLE_EVENT_REQUIRED"
    yield "no-event", calendar(include_event=False), {}, MIME, "SINGLE_EVENT_REQUIRED"
    yield "utf8-invalid", b"\xff\xfe", {}, MIME, "INVALID_UTF8"
    yield "unrelated-mime", calendar(), {}, "text/html", "UNSUPPORTED_MIME"
    yield "unknown-user-option", calendar(), {"save": True}, MIME, "INVALID_REQUEST"
    # The large irrelevant body shares the exact input limit with a full-horizon computation.
    long_month = dict(start="DTSTART;VALUE=DATE:19010131", rule="FREQ=MONTHLY;COUNT=1393")
    simple = calendar(**long_month)
    added = MAX_FILE - len(simple) - len("X-LONG:\r\n".encode())
    exact = calendar(**long_month, extras=("X-LONG:"+"a"*added,))
    assert len(exact) == MAX_FILE
    yield "exact-16k-plus-full-month-horizon", exact, {}, MIME, report(
        1393, "monthly", "1901-01-31", "2099-12-31", "20991231", kind="all-day")
    yield "oversize-ics", exact + b" ", {}, MIME, "INPUT_TOO_LARGE"
    yield "too-many-physical-lines", calendar(extras=("",)*512), {}, MIME, "INVALID_CALENDAR"


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
    rows, failures = [], []
    with TemporaryDirectory() as folder:
        local = Path(folder) / "synthetic.ics"
        for name, payload, options, mime, expected in fixtures():
            local.write_bytes(payload)
            direct, wire = envelopes(module, payload, options, mime)
            if isinstance(expected, str):
                result = dict(contractVersion=CONTRACT, ok=False, data=None,
                              error=dict(code=expected))
            else:
                result = dict(contractVersion=CONTRACT, ok=True,
                              data=expected, error=None)
            try:
                native = invoke([args.native, local], direct)
                worker_response = invoke([args.worker, "--zicode-plugin-worker"], wire)
                assert worker_response["ok"], (name, worker_response)
                worker = worker_response["result"]
                assert worker == native, (name, worker, native)
                if isinstance(expected, str):
                    assert worker["error"]["message"]
                    del worker["error"]["message"]
                assert worker == result, (name, worker, result)
            except Exception as error:
                failures.append(dict(case=name, error=str(error)[:1200]))
                print(f"FAIL {name} {str(error)[:220]}", flush=True)
            else:
                rows.append(dict(case=name, inputSha256=digest(payload), inputBytes=len(payload),
                                 mime=mime, ok=result["ok"]))
                print(f"PASS {name}", flush=True)
    receipt = dict(passed=len(rows), wasmSha256=digest(module), workerSha256=digest(args.worker.read_bytes()),
                   workerProfile=PROFILE, cases=rows, failures=failures)
    args.evidence.write_text(json.dumps(receipt, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
