//! Classify confirmed local directory observations without I/O or filesystem access.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod classification;
use classification::{Difference, ItemKind, different_digests, different_metadata};

pub const PLUGIN: &str = "com.zicode.devtools.directory-compare";
pub const CAPABILITY: &str = "devtools.directory-compare.scan";
pub const PROFILE: &str = "resource.local-directory.compare.draft1";
pub const MAX_SNAPSHOT_BYTES: usize = 60 * 1024;
pub const MAX_REPORT_BYTES: usize = 48 * 1024;

const MAX_ROWS: usize = 256;
const MAX_PATH_BYTES: usize = 2048;
const MAX_REASONS: usize = 16;
const MAX_REASON_BYTES: usize = 32;
const INVALID: &str = "INVALID_DIRECTORY_SNAPSHOT";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    schema_version: u32,
    profile: String,
    coverage_complete: bool,
    partial: bool,
    limit_reasons: Vec<String>,
    rows: Vec<Pair>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pair {
    path: String,
    left: Option<Observed>,
    right: Option<Observed>,
    verification: Option<Verification>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observed {
    kind: ItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_bytes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Verification {
    Changed,
    ReadError,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DisplayRow {
    path: String,
    difference: Difference,
    #[serde(skip_serializing_if = "Option::is_none")]
    left: Option<Observed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    right: Option<Observed>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    only_left: usize,
    only_right: usize,
    type_mismatch: usize,
    size_mismatch: usize,
    content_mismatch: usize,
    unverified: usize,
    changed: usize,
    read_error: usize,
}

impl Counts {
    fn increment(&mut self, difference: Difference) {
        let count = match difference {
            Difference::OnlyLeft => &mut self.only_left,
            Difference::OnlyRight => &mut self.only_right,
            Difference::TypeMismatch => &mut self.type_mismatch,
            Difference::SizeMismatch => &mut self.size_mismatch,
            Difference::ContentMismatch => &mut self.content_mismatch,
            Difference::Unverified => &mut self.unverified,
            Difference::Changed => &mut self.changed,
            Difference::ReadError => &mut self.read_error,
        };
        *count += 1;
    }

    fn incomplete(&self) -> bool {
        self.unverified > 0 || self.changed > 0 || self.read_error > 0
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    ok: bool,
    schema_version: u32,
    material: &'static str,
    coverage_complete: bool,
    partial: bool,
    limit_reasons: Vec<String>,
    total_paths: usize,
    same_files: usize,
    same_directories: usize,
    unassessed_one_side: usize,
    difference_count: usize,
    difference_counts: Counts,
    rows: Vec<DisplayRow>,
    projection_truncated: bool,
}

enum Assessment {
    SameFile,
    SameDirectory,
    UnassessedOneSide,
    Difference(Difference),
}

fn valid_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && !path.chars().any(|ch| {
            ch.is_control() || matches!(ch, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn valid_reasons(reasons: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    reasons.len() <= MAX_REASONS
        && reasons.iter().all(|reason| {
            reason.len() <= MAX_REASON_BYTES
                && reason.split('_').all(|part| {
                    !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_lowercase())
                })
                && unique.insert(reason)
        })
}

fn decimal(text: &str) -> Result<u64, &'static str> {
    if text.is_empty()
        || text.len() > 20
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(INVALID);
    }
    text.parse().map_err(|_| INVALID)
}

fn valid_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

impl Observed {
    fn size(&self) -> Result<u64, &'static str> {
        match (
            self.kind,
            self.logical_bytes.as_deref(),
            self.sha256.as_deref(),
        ) {
            (ItemKind::Directory, None, None) => Ok(0),
            (ItemKind::File, Some(size), digest) if digest.is_none_or(valid_digest) => {
                decimal(size)
            }
            _ => Err(INVALID),
        }
    }
}

impl Pair {
    fn assess(&self, coverage_complete: bool) -> Result<Assessment, &'static str> {
        // Validate both sides before examining coverage or explicit problems.
        let left_size = self.left.as_ref().map(Observed::size).transpose()?;
        let right_size = self.right.as_ref().map(Observed::size).transpose()?;
        let (left, right) = (self.left.as_ref(), self.right.as_ref());
        let one_side = match (left, right) {
            (None, None) => return Err(INVALID),
            (Some(_), None) => Some(Difference::OnlyLeft),
            (None, Some(_)) => Some(Difference::OnlyRight),
            (Some(_), Some(_)) => None,
        };
        if one_side.is_some() && !coverage_complete {
            return Ok(Assessment::UnassessedOneSide);
        }
        if let Some(verification) = self.verification {
            return Ok(Assessment::Difference(match verification {
                Verification::Changed => Difference::Changed,
                Verification::ReadError => Difference::ReadError,
            }));
        }
        if let Some(difference) = one_side {
            return Ok(Assessment::Difference(difference));
        }
        let (Some(left), Some(right), Some(left_size), Some(right_size)) =
            (left, right, left_size, right_size)
        else {
            return Err(INVALID);
        };
        if let Some(difference) = different_metadata(left.kind, left_size, right.kind, right_size) {
            return Ok(Assessment::Difference(difference));
        }
        if left.kind == ItemKind::Directory {
            return Ok(Assessment::SameDirectory);
        }
        Ok(
            different_digests(left.sha256.as_deref(), right.sha256.as_deref())
                .map_or(Assessment::SameFile, Assessment::Difference),
        )
    }
}

pub fn analyze(data: &[u8]) -> Result<Report, &'static str> {
    if data.len() > MAX_SNAPSHOT_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let mut snapshot: Snapshot = serde_json::from_slice(data).map_err(|_| INVALID)?;
    if snapshot.schema_version != 1
        || snapshot.profile != PROFILE
        || snapshot.rows.len() > MAX_ROWS
        || !valid_reasons(&snapshot.limit_reasons)
        || snapshot.rows.iter().any(|pair| !valid_path(&pair.path))
    {
        return Err(INVALID);
    }
    snapshot
        .rows
        .sort_unstable_by(|left, right| left.path.cmp(&right.path));
    if snapshot
        .rows
        .windows(2)
        .any(|pair| pair[0].path == pair[1].path)
    {
        return Err(INVALID);
    }
    let mut report = Report {
        ok: true,
        schema_version: 1,
        material: "directory-compare-report",
        coverage_complete: snapshot.coverage_complete,
        partial: snapshot.partial
            || !snapshot.coverage_complete
            || !snapshot.limit_reasons.is_empty(),
        limit_reasons: snapshot.limit_reasons,
        total_paths: snapshot.rows.len(),
        same_files: 0,
        same_directories: 0,
        unassessed_one_side: 0,
        difference_count: 0,
        difference_counts: Counts::default(),
        rows: Vec::new(),
        projection_truncated: false,
    };
    // Leave room for the envelope and reasons. Report the same counters even
    // after the visible, sorted prefix reaches its encoded-output budget.
    let mut remaining = MAX_REPORT_BYTES - 2048;
    for pair in snapshot.rows {
        match pair.assess(report.coverage_complete)? {
            Assessment::SameFile => report.same_files += 1,
            Assessment::SameDirectory => report.same_directories += 1,
            Assessment::UnassessedOneSide => report.unassessed_one_side += 1,
            Assessment::Difference(difference) => {
                report.difference_counts.increment(difference);
                report.difference_count += 1;
                if report.projection_truncated {
                    continue;
                }
                let row = DisplayRow {
                    path: pair.path,
                    difference,
                    left: pair.left,
                    right: pair.right,
                };
                let count = serde_json::to_vec(&row).map_err(|_| "OUTPUT_FAILED")?.len() + 1;
                if count > remaining {
                    report.projection_truncated = true;
                } else {
                    remaining -= count;
                    report.rows.push(row);
                }
            }
        }
    }
    report.partial |= report.difference_counts.incomplete();
    Ok(report)
}
