//! Log classification shared by the native viewer and its bounded Studio plugin.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

pub mod report;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Fatal,
    Warning,
    Info,
    Debug,
    Trace,
    #[default]
    Unknown,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "错误",
            Self::Fatal => "致命",
            Self::Warning => "警告",
            Self::Info => "信息",
            Self::Debug => "调试",
            Self::Trace => "跟踪",
            Self::Unknown => "未识别",
        }
    }

    pub fn from_first_line(bytes: &[u8]) -> Self {
        if bytes.len() < 3 {
            return Self::Unknown;
        }
        let mut level = Self::Unknown;
        for word in bytes.split(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_') {
            let terms: &[(&[u8], Self)] = match word.len() {
                3 => &[(b"ERR", Self::Error)],
                4 => &[(b"WARN", Self::Warning), (b"INFO", Self::Info)],
                5 => &[
                    (b"FATAL", Self::Fatal),
                    (b"ERROR", Self::Error),
                    (b"DEBUG", Self::Debug),
                    (b"TRACE", Self::Trace),
                ],
                7 => &[(b"WARNING", Self::Warning)],
                8 => &[(b"CRITICAL", Self::Fatal)],
                _ => continue,
            };
            for (token, severity) in terms {
                if word.eq_ignore_ascii_case(token) {
                    if *severity == Self::Fatal {
                        return Self::Fatal;
                    }
                    level = level.min(*severity);
                    break;
                }
            }
        }
        level
    }
}

pub fn parse_timestamp(bytes: &[u8]) -> Option<NaiveDateTime> {
    if bytes.len() < 19 {
        return None;
    }
    let line = String::from_utf8_lossy(bytes);
    let line = line.trim_start();
    let line = line.strip_prefix('[').unwrap_or(line);
    let raw = line.get(..19)?;
    let normalized = raw.replacen('T', " ", 1);
    NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S").ok()
}

pub fn is_continuation(bytes: &[u8]) -> bool {
    let line = String::from_utf8_lossy(bytes);
    let trimmed = line.trim_start();
    trimmed.is_empty()
        || line.starts_with(' ')
        || line.starts_with('\t')
        || trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("Suppressed:")
        || trimmed.starts_with("...")
        || trimmed.starts_with('^')
}
