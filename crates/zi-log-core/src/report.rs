//! Bounded in-memory report for a single confirmed UTF-8 log snapshot.

use crate::{Level, is_continuation, parse_timestamp};
use serde::{Deserialize, Serialize};

pub const MAX_FILE_BYTES: usize = 16 * 1024;
pub const MAX_QUERY_BYTES: usize = 256;
pub const MAX_PREVIEW_BYTES: usize = 512;
pub const MAX_MATCHES: usize = 12;
const MAX_LINE_PREFIX_BYTES: usize = 4096;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub level: Option<Level>,
    pub query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report<'a> {
    pub source_byte_count: usize,
    pub event_count: usize,
    pub match_count: usize,
    pub matches: Vec<Match<'a>>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match<'a> {
    pub event_number: usize,
    pub level: Level,
    pub line_count: usize,
    pub text: &'a str,
    pub preview_truncated: bool,
}

struct Event {
    start: usize,
    level: Level,
    lines: usize,
}

pub fn scan<'a>(bytes: &'a [u8], filter: &Filter) -> Result<Report<'a>, &'static str> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let query = filter.query.as_deref().unwrap_or("");
    if query.len() > MAX_QUERY_BYTES {
        return Err("QUERY_TOO_LARGE");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "INVALID_UTF8")?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut report = Report {
        source_byte_count: bytes.len(),
        event_count: 0,
        match_count: 0,
        matches: Vec::new(),
        truncated: false,
    };
    let mut offset = 0;
    let mut current = None::<Event>;
    for line in text.split_inclusive('\n') {
        let line = line.as_bytes();
        // The standalone index omits only a trailing LF from its classification prefix.
        let prefix = line.strip_suffix(b"\n").unwrap_or(line);
        let prefix = &prefix[..prefix.len().min(MAX_LINE_PREFIX_BYTES)];
        if parse_timestamp(prefix).is_some() || !is_continuation(prefix) || current.is_none() {
            if let Some(event) = current.take() {
                append(
                    &mut report,
                    &text[event.start..offset],
                    event,
                    filter.level,
                    query,
                );
            }
            current = Some(Event {
                start: offset,
                level: Level::from_first_line(prefix),
                lines: 1,
            });
        } else if let Some(event) = &mut current {
            event.lines += 1;
        }
        offset += line.len();
    }
    if let Some(event) = current {
        append(
            &mut report,
            &text[event.start..],
            event,
            filter.level,
            query,
        );
    }
    report.truncated = report.match_count > report.matches.len();
    Ok(report)
}

fn append<'a>(
    report: &mut Report<'a>,
    text: &'a str,
    event: Event,
    level: Option<Level>,
    query: &str,
) {
    report.event_count += 1;
    if level.is_some_and(|level| level != event.level)
        || (!query.is_empty() && !text.contains(query))
    {
        return;
    }
    report.match_count += 1;
    if report.matches.len() < MAX_MATCHES {
        let mut end = MAX_PREVIEW_BYTES.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        report.matches.push(Match {
            event_number: report.event_count,
            level: event.level,
            line_count: event.lines,
            text: &text[..end],
            preview_truncated: end < text.len(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_native_multiline_rules_and_filters_full_event() {
        let text = "\u{feff}2026-11-25T13:00:00 INFO started\r\n\
                    2026-11-25 13:00:01 ERROR failed\r\n\
                    \tat synthetic.Main.run(Main.java:42)\r\n\
                    Caused by: 连接超时\r\n\
                    [2026-11-25 13:00:02] WARNING retry\n\
                    INFO_VAR should stay unknown";
        let defaults = scan(text.as_bytes(), &Filter::default()).unwrap();
        assert_eq!(defaults.event_count, 4);
        assert_eq!(defaults.matches[3].level, Level::Unknown);
        assert!(!defaults.matches[0].text.starts_with('\u{feff}'));
        assert_eq!(defaults.source_byte_count, text.len());
        let filter = Filter {
            level: Some(Level::Error),
            query: Some("连接超时".into()),
        };
        let report = scan(text.as_bytes(), &filter).unwrap();
        assert_eq!(report.event_count, 4);
        assert_eq!(report.match_count, 1);
        assert_eq!(report.matches[0].event_number, 2);
        assert_eq!(report.matches[0].line_count, 3);
        assert!(report.matches[0].text.contains("\r\n\tat "));
    }

    #[test]
    fn distinguishes_report_truncation_from_an_event_preview() {
        let file = format!(
            "INFO {}\n{}",
            "中".repeat(300),
            "INFO short\n".repeat(MAX_MATCHES + 1)
        );
        let report = scan(file.as_bytes(), &Filter::default()).unwrap();
        assert_eq!(report.match_count, MAX_MATCHES + 2);
        assert_eq!(report.matches.len(), MAX_MATCHES);
        assert!(report.truncated);
        assert!(report.matches[0].preview_truncated);
        assert!(report.matches[0].text.len() <= MAX_PREVIEW_BYTES);
        assert!(!report.matches[1].preview_truncated);
        let filter = Filter {
            query: Some("SHORT".into()),
            ..Filter::default()
        };
        assert_eq!(scan(file.as_bytes(), &filter).unwrap().match_count, 0);
    }

    #[test]
    fn rejects_oversized_and_invalid_input() {
        assert_eq!(
            scan(&[0xff], &Filter::default()).unwrap_err(),
            "INVALID_UTF8"
        );
        assert_eq!(
            scan(&vec![0; MAX_FILE_BYTES + 1], &Filter::default()).unwrap_err(),
            "INPUT_TOO_LARGE"
        );
        let filter = Filter {
            query: Some("中".repeat(86)),
            ..Filter::default()
        };
        assert_eq!(scan(b"", &filter).unwrap_err(), "QUERY_TOO_LARGE");
        assert_eq!(scan(b"", &Filter::default()).unwrap().event_count, 0);
    }
}
