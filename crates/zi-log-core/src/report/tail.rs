//! Bounded end-window analysis for one confirmed log snapshot.

use crate::{is_continuation, parse_timestamp};
use serde::Serialize;

use super::{Filter, MAX_FILE_BYTES, MAX_LINE_PREFIX_BYTES, Report, scan_bytes};

pub const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailReport<'a> {
    pub source_byte_count: usize,
    pub window_start_byte: usize,
    pub analyzed_start_byte: usize,
    pub analyzed_end_byte: usize,
    pub leading_skipped_bytes: usize,
    pub trailing_incomplete_utf8_bytes: usize,
    pub ends_with_newline: bool,
    pub partial: bool,
    pub analysis: Report<'a>,
}

pub fn window_start(source_length: usize) -> Result<usize, &'static str> {
    if source_length > MAX_SNAPSHOT_BYTES {
        return Err("SNAPSHOT_TOO_LARGE");
    }
    Ok(source_length.saturating_sub(MAX_FILE_BYTES))
}

pub fn read_start(source_length: usize) -> Result<usize, &'static str> {
    // A single byte preceding the window tells whether its first line is whole.
    Ok(window_start(source_length)?.saturating_sub(1))
}

pub fn scan_tail<'a>(
    bytes: &'a [u8],
    source_length: usize,
    filter: &Filter,
) -> Result<TailReport<'a>, &'static str> {
    let start = window_start(source_length)?;
    let has_lookbehind = start > 0;
    if bytes.len() != source_length - start + usize::from(has_lookbehind) {
        return Err("RESOURCE_READ_FAILED");
    }
    let starts_at_line = !has_lookbehind || bytes[0] == b'\n';
    let bytes = &bytes[usize::from(has_lookbehind)..];
    let after_partial_line = if starts_at_line {
        0
    } else {
        bytes
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(bytes.len(), |offset| offset + 1)
    };
    let candidate = &bytes[after_partial_line..];
    let utf8_end = match std::str::from_utf8(candidate) {
        Ok(_) => candidate.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => return Err("INVALID_UTF8"),
    };
    let end = after_partial_line + utf8_end;
    let mut first_event = after_partial_line;
    if start > 0 {
        // Other complete leading lines may still belong to the clipped event.
        for line in bytes[after_partial_line..end].split_inclusive(|&byte| byte == b'\n') {
            let prefix = line.strip_suffix(b"\n").unwrap_or(line);
            let prefix = &prefix[..prefix.len().min(MAX_LINE_PREFIX_BYTES)];
            if parse_timestamp(prefix).is_some() || !is_continuation(prefix) {
                break;
            }
            first_event += line.len();
        }
    }
    let analysis = scan_bytes(&bytes[first_event..end], filter, start == 0)?;
    Ok(TailReport {
        source_byte_count: source_length,
        window_start_byte: start,
        analyzed_start_byte: start + first_event,
        analyzed_end_byte: start + end,
        leading_skipped_bytes: first_event,
        trailing_incomplete_utf8_bytes: bytes.len() - end,
        ends_with_newline: bytes.last() == Some(&b'\n'),
        partial: start != 0 || end != bytes.len(),
        analysis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_a_partial_front_character_and_orphaned_stack_lines() {
        let front =
            b"\x82\xac in clipped record\r\n\tat synthetic.Old.java\r\nCaused by: clipped\r\n";
        let content = b"2026-11-25 12:21:55 ERROR expected\n";
        let mut tail = [b' '; MAX_FILE_BYTES + 1];
        let len = front.len() + content.len();
        tail[1..1 + front.len()].copy_from_slice(front);
        tail[1 + front.len()..1 + len].copy_from_slice(content);
        let result = scan_tail(&tail, MAX_FILE_BYTES + 100, &Filter::default()).unwrap();
        assert_eq!(result.window_start_byte, 100);
        assert_eq!(result.analyzed_start_byte, 100 + front.len());
        assert_eq!(result.leading_skipped_bytes, front.len());
        assert_eq!(result.analysis.event_count, 1);
        assert_eq!(result.analysis.matches[0].line_count, 2);
        assert!(result.partial);
    }

    #[test]
    fn retains_the_first_event_at_a_proven_line_boundary() {
        let banner = b"INFO complete\n";
        let mut bytes = [b' '; MAX_FILE_BYTES + 1];
        bytes[0] = b'\n';
        bytes[1..banner.len() + 1].copy_from_slice(banner);
        let report = scan_tail(&bytes, MAX_FILE_BYTES + 50, &Filter::default()).unwrap();
        assert_eq!(report.analyzed_start_byte, 50);
        assert_eq!(report.leading_skipped_bytes, 0);
        assert_eq!(report.analysis.matches[0].level, crate::Level::Info);
        assert_eq!(report.analysis.event_count, 1);
    }

    #[test]
    fn only_trims_an_incomplete_suffix_and_reports_unterminated_eof() {
        let bytes = b"INFO logged \xe4\xb8";
        let report = scan_tail(bytes, bytes.len(), &Filter::default()).unwrap();
        assert_eq!(report.trailing_incomplete_utf8_bytes, 2);
        assert_eq!(report.analyzed_end_byte, bytes.len() - 2);
        assert_eq!(report.analysis.matches[0].text, "INFO logged ");
        assert!(report.partial);
        assert!(!report.ends_with_newline);
        let invalid = b"bad\x80\nINFO proper";
        assert_eq!(
            scan_tail(invalid, invalid.len(), &Filter::default()).unwrap_err(),
            "INVALID_UTF8"
        );
        assert_eq!(
            window_start(MAX_SNAPSHOT_BYTES + 1),
            Err("SNAPSHOT_TOO_LARGE")
        );
    }

    #[test]
    fn refuses_mismatched_descriptors_and_does_not_invent_events() {
        assert_eq!(
            scan_tail(b"abc", 4, &Filter::default()).unwrap_err(),
            "RESOURCE_READ_FAILED"
        );
        let bytes = [b'x'; MAX_FILE_BYTES + 1];
        let report = scan_tail(&bytes, MAX_FILE_BYTES + 1, &Filter::default()).unwrap();
        assert_eq!(report.analyzed_start_byte, report.source_byte_count);
        assert_eq!(report.analysis.event_count, 0);
        assert_eq!(report.leading_skipped_bytes, MAX_FILE_BYTES);
    }
}
