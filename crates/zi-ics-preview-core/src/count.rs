//! Bounded offline conversion for a single explicitly shared COUNT recurrence.
use crate::{prop, properties, recurrence, value};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use ical::property::Property;
use serde::Serialize;

pub const MAX_FILE_BYTES: usize = 16 * 1024;
const MAX_PHYSICAL_LINES: usize = 512;
const MAX_RULE_BYTES: usize = 512;
type Error = &'static str;
const INVALID: Error = "INVALID_CALENDAR";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Component {
    Calendar,
    Event,
    Alarm,
}

impl Component {
    fn parse(property: &Property) -> Result<Self, Error> {
        if property.params.is_some() {
            return Err(INVALID);
        }
        let value = property.value.as_deref().ok_or(INVALID)?;
        if value.eq_ignore_ascii_case("VCALENDAR") {
            Ok(Self::Calendar)
        } else if value.eq_ignore_ascii_case("VEVENT") {
            Ok(Self::Event)
        } else if value.eq_ignore_ascii_case("VALARM") {
            Ok(Self::Alarm)
        } else {
            Err("UNSUPPORTED_COMPONENT")
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    count: u32,
    frequency: &'static str,
    time_kind: &'static str,
    start: String,
    last_start: String,
    equivalent_until: Option<String>,
    clamp_missing_day: bool,
    text: String,
}

pub fn read(bytes: &[u8]) -> Result<Report, Error> {
    use Component::*;

    if bytes.len() > MAX_FILE_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let input = std::str::from_utf8(bytes).map_err(|_| "INVALID_UTF8")?;
    let mut stack = Vec::new();
    let mut calendar = Vec::new();
    let mut event = Vec::new();
    let mut ended = false;
    let mut seen_event = false;
    for property in properties(input, MAX_FILE_BYTES, MAX_PHYSICAL_LINES, MAX_FILE_BYTES)
        .map_err(|_| INVALID)?
    {
        match property.name.as_str() {
            "BEGIN" => {
                let component = Component::parse(&property)?;
                match (stack.as_slice(), component) {
                    ([], Calendar) if !ended => {}
                    ([Calendar], Event) if !seen_event => seen_event = true,
                    ([Calendar], Event) => return Err("SINGLE_EVENT_REQUIRED"),
                    ([Calendar, Event], Alarm) => {}
                    _ => return Err(INVALID),
                }
                stack.push(component);
            }
            "END" => {
                if stack.pop() != Some(Component::parse(&property)?) {
                    return Err(INVALID);
                }
                ended = stack.is_empty();
            }
            _ => match stack.as_slice() {
                [Calendar] => calendar.push(property),
                [Calendar, Event] => event.push(property),
                [Calendar, Event, Alarm] => {}
                _ => return Err(INVALID),
            },
        }
    }

    if !ended || value(&calendar, "VERSION").map_err(|_| INVALID)? != Some("2.0") {
        return Err(INVALID);
    }
    if !seen_event {
        return Err("SINGLE_EVENT_REQUIRED");
    }
    if prop(&calendar, "X-WR-TIMEZONE")
        .map_err(|_| INVALID)?
        .is_some()
    {
        return Err("UNSUPPORTED_TIME");
    }
    for (key, supported) in [("CALSCALE", "GREGORIAN"), ("METHOD", "PUBLISH")] {
        if value(&calendar, key)
            .map_err(|_| INVALID)?
            .is_some_and(|candidate| !candidate.eq_ignore_ascii_case(supported))
        {
            return Err("UNSUPPORTED_CALENDAR");
        }
    }
    for name in ["EXDATE", "RDATE", "RECURRENCE-ID", "EXRULE", "DURATION"] {
        if prop(&event, name).map_err(|_| INVALID)?.is_some() {
            return Err("UNSUPPORTED_EXCEPTION");
        }
    }
    if value(&event, "STATUS")
        .map_err(|_| INVALID)?
        .is_some_and(|status| !status.eq_ignore_ascii_case("CONFIRMED"))
    {
        return Err("UNSUPPORTED_EVENT_STATUS");
    }
    let start = prop(&event, "DTSTART")
        .map_err(|_| INVALID)?
        .ok_or("INVALID_START")?;
    let (start, all_day) = start_time(start)?;
    if !(recurrence::MIN_YEAR..=recurrence::MAX_YEAR).contains(&start.year()) {
        return Err("INVALID_START");
    }
    let rrule = prop(&event, "RRULE")
        .map_err(|_| INVALID)?
        .ok_or("COUNT_REQUIRED")?;
    if !valid_kind(rrule, "RECUR") {
        return Err("INVALID_RULE");
    }
    let rule = rrule.value.as_deref().ok_or("INVALID_RULE")?;
    if rule.len() > MAX_RULE_BYTES {
        return Err("INVALID_RULE");
    }
    let conversion = recurrence::parse_rule(start, all_day, rule).map_err(|_| "INVALID_RULE")?;
    let count = conversion.count.ok_or("COUNT_REQUIRED")?;

    let time_kind = if all_day { "all-day" } else { "floating" };
    let time_label = if all_day { "全天" } else { "浮动时间" };
    let frequency = match count.frequency {
        recurrence::Repeat::Daily => "daily",
        recurrence::Repeat::Weekly => "weekly",
        recurrence::Repeat::Monthly => "monthly",
        recurrence::Repeat::Yearly => "yearly",
        recurrence::Repeat::Once => unreachable!("COUNT has a source frequency"),
    };
    let last = count.last.and_time(start.time());
    let when = |value: NaiveDateTime| {
        if all_day {
            value.format("%Y-%m-%d").to_string()
        } else {
            value.format("%Y-%m-%d %H:%M").to_string()
        }
    };
    let equivalent_until = (count.count != 1).then(|| {
        let pattern = if all_day { "%Y%m%d" } else { "%Y%m%dT%H%M%S" };
        last.format(pattern).to_string()
    });
    let start = when(start);
    let last_start = when(last);
    let completion = equivalent_until.as_ref().map_or_else(
        || "仅一次发生，转换后可使用单次日程。".to_owned(),
        |until| {
            format!("将原 RRULE 的 COUNT 替换为 UNTIL={until}，保留其他条件。末次开始计入重复。")
        },
    );
    let text = format!(
        "{time_label} · {} · {} 次（包含首次）\n首次开始 {start}\n最后开始 {last_start}\n{completion}",
        count.frequency.label(),
        count.count,
    );
    Ok(Report {
        count: count.count,
        frequency,
        time_kind,
        start,
        last_start,
        equivalent_until,
        clamp_missing_day: conversion.clamp_missing_day,
        text,
    })
}

fn valid_kind(property: &Property, kind: &str) -> bool {
    property.params.as_deref().is_none_or(|params| {
        matches!(params, [(key, values)] if key == "VALUE"
            && matches!(values.as_slice(), [value] if value.eq_ignore_ascii_case(kind)))
    })
}

fn start_time(property: &Property) -> Result<(NaiveDateTime, bool), Error> {
    let raw = property.value.as_deref().ok_or("INVALID_START")?;
    let date_only = property.params.is_some() && valid_kind(property, "DATE");
    if raw.ends_with('Z') || (!date_only && !valid_kind(property, "DATE-TIME")) {
        return Err("UNSUPPORTED_TIME");
    }
    if date_only {
        if raw.len() != 8 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("INVALID_START");
        }
        let date = NaiveDate::parse_from_str(raw, "%Y%m%d").map_err(|_| "INVALID_START")?;
        return Ok((date.and_time(NaiveTime::MIN), true));
    }
    if raw.len() != 15
        || raw.as_bytes()[8] != b'T'
        || !raw
            .bytes()
            .enumerate()
            .all(|(i, byte)| i == 8 || byte.is_ascii_digit())
    {
        return Err("INVALID_START");
    }
    let start = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S").map_err(|_| "INVALID_START")?;
    if start.second() != 0 || start.nanosecond() != 0 {
        return Err("INVALID_START");
    }
    Ok((start, false))
}
