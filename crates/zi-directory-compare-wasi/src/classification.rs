//! Metadata and digest classification shared with the independent filesystem tool.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    File,
    Directory,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "文件",
            Self::Directory => "目录",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Difference {
    OnlyLeft,
    OnlyRight,
    TypeMismatch,
    SizeMismatch,
    ContentMismatch,
    Unverified,
    Changed,
    ReadError,
}

impl Difference {
    pub fn label(self) -> &'static str {
        match self {
            Self::OnlyLeft => "仅左侧",
            Self::OnlyRight => "仅右侧",
            Self::TypeMismatch => "类型不同",
            Self::SizeMismatch => "大小不同",
            Self::ContentMismatch => "内容不同",
            Self::Unverified => "未校验内容",
            Self::Changed => "检查中变化",
            Self::ReadError => "读取失败",
        }
    }
}

pub fn different_metadata(
    left_kind: ItemKind,
    left_size: u64,
    right_kind: ItemKind,
    right_size: u64,
) -> Option<Difference> {
    if left_kind != right_kind {
        Some(Difference::TypeMismatch)
    } else if left_kind == ItemKind::File && left_size != right_size {
        Some(Difference::SizeMismatch)
    } else {
        None
    }
}

// The caller only supplies digests validated for the current snapshot.
pub fn different_digests(left: Option<&str>, right: Option<&str>) -> Option<Difference> {
    match (left, right) {
        (Some(left), Some(right)) if left == right => None,
        (Some(_), Some(_)) => Some(Difference::ContentMismatch),
        _ => Some(Difference::Unverified),
    }
}
