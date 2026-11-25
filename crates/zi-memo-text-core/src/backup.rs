//! Checks fields shared by full backup restore and bounded single-memo extraction.
use crate::MAX_BODY_BYTES;
use anyhow::{Result, ensure};
use serde::{Deserialize, de::IgnoredAny};
use std::collections::HashSet;

pub const FORMAT: &str = "zi-devtools-planner";
pub const VERSION: u32 = 1;

pub fn valid_timestamp(value: i64) -> bool {
    // Whole UTC seconds from 0001-01-01 through 9999-12-31.
    (-62_135_596_800..=253_402_300_799).contains(&value)
}

pub fn validate_memo_fields(
    id: &str,
    revision: i64,
    updated: i64,
    title: &str,
    body: &str,
    tags: &[String],
) -> Result<()> {
    ensure!(uuid::Uuid::parse_str(id)?.to_string() == id, "记录标识无效");
    ensure!(revision >= 0, "记录版本无效");
    ensure!(valid_timestamp(updated), "记录更新时间无效");
    ensure!(
        !title.trim().is_empty()
            && title.chars().count() <= 120
            && !title.chars().any(char::is_control),
        "标题需要 1–120 个字符，不能包含换行"
    );
    ensure!(body.len() <= MAX_BODY_BYTES, "正文最多 128 KiB");
    validate_tags(tags)?;
    Ok(())
}

pub fn validate_tags(tags: &[String]) -> Result<()> {
    ensure!(tags.len() <= 16, "每条记录最多 16 个标签");
    let mut seen = HashSet::new();
    for tag in tags {
        ensure!(
            !tag.trim().is_empty()
                && tag == tag.trim()
                && tag.chars().count() <= 24
                && !tag.chars().any(|c| c.is_control() || c == ',' || c == '#'),
            "标签须为 1–24 个字符，不能包含逗号、# 或控制字符"
        );
        ensure!(seen.insert(tag.to_lowercase()), "标签不能重复");
    }
    ensure!(
        tags.iter().map(String::len).sum::<usize>() <= 1024,
        "标签总长度超过限制"
    );
    Ok(())
}

pub const MAX_EXTRACT_BYTES: usize = 128 * 1024;
pub const MAX_EXTRACT_RECORDS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    format: String,
    version: u32,
    created_at: i64,
    pub records: Vec<Record>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    id: String,
    revision: i64,
    updated: i64,
    pub title: String,
    pub body: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(rename = "pinned")]
    _pinned: bool,
    trash: bool,
    schedule: Option<IgnoredAny>,
    calendar_uid: Option<String>,
}

impl Record {
    pub fn can_export(&self) -> bool {
        self.schedule.is_none() && self.calendar_uid.is_none() && !self.trash
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtractProblem {
    TooLarge,
    TooManyRecords,
    InvalidBackup,
    UnsupportedFormat,
}

pub fn read_snapshot(bytes: &[u8]) -> std::result::Result<Snapshot, ExtractProblem> {
    if bytes.len() > MAX_EXTRACT_BYTES {
        return Err(ExtractProblem::TooLarge);
    }
    let snapshot: Snapshot =
        serde_json::from_slice(bytes).map_err(|_| ExtractProblem::InvalidBackup)?;
    if snapshot.format != FORMAT || snapshot.version != VERSION {
        return Err(ExtractProblem::UnsupportedFormat);
    }
    if snapshot.records.len() > MAX_EXTRACT_RECORDS {
        return Err(ExtractProblem::TooManyRecords);
    }
    if !valid_timestamp(snapshot.created_at) {
        return Err(ExtractProblem::InvalidBackup);
    }
    let mut ids = HashSet::new();
    for row in &snapshot.records {
        if row.revision == 0
            || !ids.insert(&row.id)
            || validate_memo_fields(
                &row.id,
                row.revision,
                row.updated,
                &row.title,
                &row.body,
                &row.tags,
            )
            .is_err()
            || row.calendar_uid.as_ref().is_some_and(|uid| {
                row.schedule.is_none()
                    || uid.is_empty()
                    || uid.len() > 1024
                    || uid.chars().any(char::is_control)
            })
        {
            return Err(ExtractProblem::InvalidBackup);
        }
    }
    Ok(snapshot)
}
