//! Text preparation shared by native memo import and the independent preview.
use std::{error, fmt};

pub mod backup;

pub const MAX_BODY_BYTES: usize = 128 * 1024;
pub const BOM: &[u8] = b"\xef\xbb\xbf";
pub const MAX_FILE_BYTES: usize = MAX_BODY_BYTES + BOM.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportError {
    TooLarge,
    InvalidUtf8,
    ContainsNul,
}

impl ImportError {
    pub fn message(self) -> &'static str {
        match self {
            Self::TooLarge => "文件超过 128 KiB，请先缩小内容",
            Self::InvalidUtf8 => "文件不是 UTF-8；请先用文件编码工具转换后导入",
            Self::ContainsNul => "文件包含空字节，请确认是文本文件",
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}
impl error::Error for ImportError {}

pub struct PreparedMemo<'a> {
    pub title: String,
    pub body: &'a str,
    pub had_bom: bool,
}

pub fn prepare<'a>(source: &'a [u8], title: &str) -> Result<PreparedMemo<'a>, ImportError> {
    let body = source.strip_prefix(BOM).unwrap_or(source);
    if body.len() > MAX_BODY_BYTES {
        return Err(ImportError::TooLarge);
    }
    let body = std::str::from_utf8(body).map_err(|_| ImportError::InvalidUtf8)?;
    if body.as_bytes().contains(&0) {
        return Err(ImportError::ContainsNul);
    }
    let title: String = title
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect();
    let title = title.trim();
    Ok(PreparedMemo {
        title: if title.is_empty() {
            "导入的备忘"
        } else {
            title
        }
        .to_owned(),
        body,
        had_bom: source.starts_with(BOM),
    })
}
