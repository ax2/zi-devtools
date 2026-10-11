//! Shared line comparison with the standalone application's original limits.
//!
//! The algorithm uses a monotonic clock and may approximate after 500 ms.
//! A report budget limits formatted UTF-8 output, not algorithm memory or time.
use std::{fmt, fmt::Write, time::Duration};

use similar::TextDiff;

pub const MAX_INPUT_BYTES: usize = 128 * 1024;
pub const MAX_INPUT_LINES: usize = 5_000;
const IDENTICAL: &str = "两份文本完全相同";

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
    };
    if before == after {
        output
            .write_str(IDENTICAL)
            .map_err(|_| CompareError::ReportTooLarge)?;
    } else {
        let diff = TextDiff::configure()
            .timeout(Duration::from_millis(500))
            .diff_lines(before, after);
        write!(
            output,
            "{}",
            diff.unified_diff()
                .context_radius(3)
                .header("原文", "修改后")
        )
        .map_err(|_| CompareError::ReportTooLarge)?;
    }
    Ok(output.text)
}

struct ReportWriter {
    text: String,
    limit: usize,
}

impl Write for ReportWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.len() > self.limit - self.text.len() {
            return Err(fmt::Error);
        }
        self.text.push_str(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
