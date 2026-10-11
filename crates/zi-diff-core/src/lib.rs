//! Shared line comparison with the standalone application's original limits.
//!
//! The algorithm uses a monotonic clock and may approximate after 500 ms.
//! A report budget limits formatted UTF-8 output, not algorithm memory or time.
use std::{fmt, fmt::Write, time::Duration};

use similar::TextDiff;

mod disjoint;

pub const MAX_INPUT_BYTES: usize = 128 * 1024;
pub const MAX_INPUT_LINES: usize = 5_000;
const IDENTICAL: &str = "两份文本完全相同";

#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum ComparePhase {
    DiffComputed,
    ReportFormatted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareError {
    InputTooLarge,
    ReportTooLarge,
}

impl fmt::Display for CompareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "每份文本最多 128 KiB、5000 行",
            Self::ReportTooLarge => "差异报告超出输出字节预算",
        })
    }
}

impl std::error::Error for CompareError {}

/// Compare complete inputs; never return a truncated report as success.
/// `None` preserves the standalone application's full output.
pub fn compare_text(
    before: &str,
    after: &str,
    max_report_bytes: Option<usize>,
) -> Result<String, CompareError> {
    compare_text_observed(before, after, max_report_bytes, |_| {})
}

/// Diagnostic observer; ordinary comparisons instantiate a no-op closure.
#[doc(hidden)]
pub fn compare_text_observed(
    before: &str,
    after: &str,
    max_report_bytes: Option<usize>,
    observe: impl FnMut(ComparePhase),
) -> Result<String, CompareError> {
    compare_text_budgets_observed(before, after, max_report_bytes, None, observe)
}

/// Optional escaped JSON string-content budget, excluding its two quotes.
#[doc(hidden)]
pub fn compare_text_budgets_observed(
    before: &str,
    after: &str,
    max_report_bytes: Option<usize>,
    max_json_string_bytes: Option<usize>,
    mut observe: impl FnMut(ComparePhase),
) -> Result<String, CompareError> {
    if before.len() > MAX_INPUT_BYTES
        || after.len() > MAX_INPUT_BYTES
        || before.lines().count() > MAX_INPUT_LINES
        || after.lines().count() > MAX_INPUT_LINES
    {
        return Err(CompareError::InputTooLarge);
    }
    let mut output = ReportWriter {
        text: String::new(),
        limit: max_report_bytes.unwrap_or(usize::MAX),
        json_remaining: max_json_string_bytes,
    };
    if before == after {
        output
            .write_str(IDENTICAL)
            .map_err(|_| CompareError::ReportTooLarge)?;
    } else {
        if let Some(plan) = disjoint::plan(before, after) {
            observe(ComparePhase::DiffComputed);
            disjoint::write_report(before, after, plan, &mut output)
                .map_err(|_| CompareError::ReportTooLarge)?;
            observe(ComparePhase::ReportFormatted);
            return Ok(output.text);
        }
        let diff = TextDiff::configure()
            .timeout(Duration::from_millis(500))
            .diff_lines(before, after);
        observe(ComparePhase::DiffComputed);
        write!(
            output,
            "{}",
            diff.unified_diff()
                .context_radius(3)
                .header("原文", "修改后")
        )
        .map_err(|_| CompareError::ReportTooLarge)?;
    }
    observe(ComparePhase::ReportFormatted);
    Ok(output.text)
}

struct ReportWriter {
    text: String,
    limit: usize,
    json_remaining: Option<usize>,
}

const JSON_ESCAPE_EXTRA: [u8; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 32 {
        table[index] = 5;
        index += 1;
    }
    table[b'"' as usize] = 1;
    table[b'\\' as usize] = 1;
    table[b'\x08' as usize] = 1;
    table[b'\x0c' as usize] = 1;
    table[b'\n' as usize] = 1;
    table[b'\r' as usize] = 1;
    table[b'\t' as usize] = 1;
    table
};

impl Write for ReportWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.len() > self.limit - self.text.len() {
            return Err(fmt::Error);
        }
        if let Some(remaining) = &mut self.json_remaining {
            // serde_json's default string escaping is additive over UTF-8 bytes.
            // Chunks are bounded by the validated input/report sizes.
            let escaped = value.bytes().fold(value.len(), |size, byte| {
                size + usize::from(JSON_ESCAPE_EXTRA[usize::from(byte)])
            });
            if escaped > *remaining {
                return Err(fmt::Error);
            }
            *remaining -= escaped;
        }
        self.text.push_str(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
