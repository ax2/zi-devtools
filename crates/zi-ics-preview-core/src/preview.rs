//! Preview calendar file semantics without timezone conversion or recurrence expansion.

use crate::{prop, properties, text_value, value};
use chrono::{NaiveDate, NaiveDateTime};
use ical::property::Property;
use serde::Serialize;

pub const MAX_FILE_BYTES: usize = 16 * 1024;
const MAX_EVENTS_SHOWN: usize = 8;
const MAX_TEXT_BYTES: usize = 6 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    source_byte_count: usize,
    event_count: usize,
    recurring_event_count: usize,
    recurrence_expanded: bool,
    truncated: bool,
    events: Vec<Event>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    summary: String,
    summary_truncated: bool,
    start: Time,
    end: Option<Time>,
    recurrence_rule: Option<String>,
    recurrence_exceptions_present: bool,
}

#[derive(Debug, Serialize)]
struct Time {
    value: String,
    kind: TimeKind,
    timezone: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum TimeKind {
    AllDay,
    Floating,
    Utc,
    NamedTimezone,
}

type Failure = &'static str;
const INVALID: Failure = "INVALID_CALENDAR";

pub fn read(bytes: &[u8]) -> Result<Report, Failure> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let input = std::str::from_utf8(bytes).map_err(|_| "INVALID_UTF8")?;
    let mut report = Report {
        source_byte_count: bytes.len(),
        event_count: 0,
        recurring_event_count: 0,
        recurrence_expanded: false,
        truncated: false,
        events: Vec::new(),
    };
    let mut stack: Vec<String> = Vec::new();
    let mut calendar = Vec::new();
    let mut event = Vec::new();
    let mut ended = false;
    let mut remaining_text = MAX_TEXT_BYTES;
    for property in
        properties(input, MAX_FILE_BYTES, MAX_FILE_BYTES, MAX_FILE_BYTES).map_err(|_| INVALID)?
    {
        match property.name.as_str() {
            "BEGIN" => {
                let name = component(&property)?;
                if ended || stack.len() >= 8 || (stack.is_empty() && name != "VCALENDAR") {
                    return Err(INVALID);
                }
                if name == "VCALENDAR" && !stack.is_empty() {
                    return Err(INVALID);
                }
                stack.push(name);
            }
            "END" => {
                let name = component(&property)?;
                if stack.pop().as_deref() != Some(name.as_str()) {
                    return Err(INVALID);
                }
                if stack.len() == 1 && name == "VEVENT" {
                    let row = row(&event)?;
                    event.clear();
                    report.event_count += 1;
                    report.recurring_event_count += usize::from(
                        row.recurrence_rule.is_some() || row.recurrence_exceptions_present,
                    );
                    let size = row.text_bytes();
                    if !report.truncated
                        && report.events.len() < MAX_EVENTS_SHOWN
                        && size <= remaining_text
                    {
                        remaining_text -= size;
                        report.events.push(row);
                    } else {
                        report.truncated = true;
                    }
                }
                ended = stack.is_empty();
            }
            _ if stack.len() == 1 => calendar.push(property),
            _ if stack.len() == 2 && stack[1] == "VEVENT" => event.push(property),
            _ if stack.is_empty() => return Err(INVALID),
            _ => {}
        }
    }
    if !ended || value(&calendar, "VERSION").map_err(|_| INVALID)? != Some("2.0") {
        return Err(INVALID);
    }
    Ok(report)
}

fn component(property: &Property) -> Result<String, Failure> {
    let name = property.value.as_deref().ok_or(INVALID)?;
    if property.params.is_some()
        || name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(INVALID);
    }
    Ok(name.to_ascii_uppercase())
}

fn row(properties: &[Property]) -> Result<Event, Failure> {
    let start = prop(properties, "DTSTART")
        .map_err(|_| INVALID)?
        .ok_or("MISSING_START")?;
    let end = prop(properties, "DTEND")
        .map_err(|_| INVALID)?
        .map(time)
        .transpose()?;
    let mut summary = text_value(
        value(properties, "SUMMARY")
            .map_err(|_| INVALID)?
            .unwrap_or(""),
    )
    .map_err(|_| INVALID)?;
    let summary_truncated = summary.len() > 512;
    if summary_truncated {
        let mut end = 512;
        while !summary.is_char_boundary(end) {
            end -= 1;
        }
        summary.truncate(end);
    }
    let recurrence_rule = value(properties, "RRULE").map_err(|_| INVALID)?;
    if recurrence_rule.is_some_and(|rule| rule.len() > 512) {
        return Err("PROPERTY_TOO_LARGE");
    }
    Ok(Event {
        summary,
        summary_truncated,
        start: time(start)?,
        end,
        recurrence_rule: recurrence_rule.map(str::to_owned),
        recurrence_exceptions_present: properties
            .iter()
            .any(|p| matches!(p.name.as_str(), "RDATE" | "EXDATE" | "RECURRENCE-ID")),
    })
}

impl Event {
    fn text_bytes(&self) -> usize {
        self.summary.len()
            + self.start.text_bytes()
            + self.end.as_ref().map_or(0, Time::text_bytes)
            + self.recurrence_rule.as_ref().map_or(0, String::len)
    }
}
impl Time {
    fn text_bytes(&self) -> usize {
        self.value.len() + self.timezone.as_ref().map_or(0, String::len)
    }
}

fn parameter<'a>(property: &'a Property, key: &str) -> Result<Option<&'a str>, Failure> {
    let mut matches = property
        .params
        .iter()
        .flatten()
        .filter(|(name, _)| name.eq_ignore_ascii_case(key));
    let Some((_, values)) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() || values.len() != 1 || values[0].is_empty() {
        return Err("INVALID_TIME");
    }
    Ok(Some(&values[0]))
}

fn time(property: &Property) -> Result<Time, Failure> {
    const BAD: Failure = "INVALID_TIME";
    let value = property.value.as_deref().ok_or(BAD)?;
    let zone = parameter(property, "TZID")?;
    if zone.is_some_and(|zone| zone.len() > 128) {
        return Err("PROPERTY_TOO_LARGE");
    }
    let kind = match parameter(property, "VALUE")?.unwrap_or("DATE-TIME") {
        "DATE"
            if zone.is_none() && value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            NaiveDate::parse_from_str(value, "%Y%m%d").map_err(|_| BAD)?;
            TimeKind::AllDay
        }
        "DATE-TIME" => {
            let local = value.strip_suffix('Z').unwrap_or(value);
            if local.len() != 15
                || local.as_bytes()[8] != b'T'
                || !local
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
            {
                return Err(BAD);
            }
            NaiveDateTime::parse_from_str(local, "%Y%m%dT%H%M%S").map_err(|_| BAD)?;
            match (local == value, zone) {
                (false, None) => TimeKind::Utc,
                (true, None) => TimeKind::Floating,
                (true, Some(_)) => TimeKind::NamedTimezone,
                (false, Some(_)) => return Err(BAD),
            }
        }
        _ => return Err(BAD),
    };
    Ok(Time {
        value: value.to_owned(),
        kind,
        timezone: zone.map(str::to_owned),
    })
}
