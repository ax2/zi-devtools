//! Analyze the digest snapshot of a directory explicitly shared through Studio.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub mod groups;

pub const PLUGIN: &str = "com.zicode.devtools.duplicate-files";
pub const CAPABILITY: &str = "devtools.duplicate-files.scan";
pub const PROFILE: &str = "resource.local-directory.duplicates.draft1";
pub const MAX_SNAPSHOT_BYTES: usize = 60 * 1024;
pub const MAX_REPORT_BYTES: usize = 48 * 1024;

const MAX_VISITED: usize = 4096;
const MAX_HASHED_FILES: usize = 256;
const MAX_HASHED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 2048;
const MAX_GROUPS: usize = 20;
const MAX_PATHS_PER_GROUP: usize = 12;
const MAX_DISPLAY_PATH_BYTES: usize = 32 * 1024;
const INVALID: &str = "INVALID_DIRECTORY_SNAPSHOT";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Limit {
    Entries,
    Time,
    CandidateFiles,
    HashBytes,
    SnapshotBytes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    schema_version: u32,
    profile: String,
    files: Vec<Row>,
    visited_entries: usize,
    candidate_files: usize,
    hashed_files: usize,
    hashed_bytes: String,
    skipped_links: usize,
    skipped_depth: usize,
    skipped_names: usize,
    unreadable: usize,
    changed_files: usize,
    skipped_budget: usize,
    limit_reason: Option<Limit>,
    partial: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Row {
    path: String,
    logical_bytes: String,
    sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    visited_entries: usize,
    candidate_files: usize,
    hashed_files: usize,
    hashed_bytes: String,
    skipped_links: usize,
    skipped_depth: usize,
    skipped_names: usize,
    unreadable: usize,
    changed_files: usize,
    skipped_budget: usize,
    limit_reason: Option<Limit>,
    partial: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    ok: bool,
    schema_version: u32,
    material: &'static str,
    partial: bool,
    group_count: usize,
    duplicate_file_count: usize,
    redundant_bytes: String,
    groups: Vec<ReportGroup>,
    projection_truncated: bool,
    source: Source,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportGroup {
    logical_bytes: String,
    sha256: String,
    redundant_bytes: String,
    file_count: usize,
    paths: Vec<String>,
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

fn file_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && !path.chars().any(|ch| {
            ch.is_control() || matches!(ch, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn digest(text: &str) -> Result<[u8; 32], &'static str> {
    if text.len() != 64 {
        return Err(INVALID);
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(INVALID),
    };
    let mut result = [0; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (slot, pair) in result.iter_mut().zip(pairs) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(result)
}

impl Snapshot {
    fn validated_source(&self) -> Result<(), &'static str> {
        let skipped = [
            self.skipped_links,
            self.skipped_depth,
            self.skipped_names,
            self.unreadable,
            self.changed_files,
            self.skipped_budget,
        ];
        let has_limit = self.limit_reason.is_some() || skipped.iter().any(|value| *value != 0);
        if self.schema_version != 1
            || self.profile != PROFILE
            || self.visited_entries > MAX_VISITED
            || self.candidate_files > self.visited_entries
            || self.hashed_files > self.candidate_files
            || self.hashed_files > MAX_HASHED_FILES
            || self.hashed_files != self.files.len()
            || skipped.iter().any(|count| *count > self.visited_entries)
            || self.partial != has_limit
            || (!self.partial && self.hashed_files != self.candidate_files)
        {
            return Err(INVALID);
        }
        Ok(())
    }
}

pub fn analyze(bytes: &[u8]) -> Result<Report, &'static str> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err("INPUT_TOO_LARGE");
    }
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    snapshot.validated_source()?;
    let expected_bytes = decimal(&snapshot.hashed_bytes)?;
    if expected_bytes > MAX_HASHED_BYTES {
        return Err(INVALID);
    }

    let mut observed_bytes = 0u64;
    let mut unique_paths = BTreeSet::new();
    let mut buckets = groups::Buckets::new();
    for row in &snapshot.files {
        if !file_path(&row.path) || !unique_paths.insert(row.path.as_str()) {
            return Err(INVALID);
        }
        let size = decimal(&row.logical_bytes)?;
        observed_bytes = observed_bytes.checked_add(size).ok_or(INVALID)?;
        buckets
            .entry((size, digest(&row.sha256)?))
            .or_default()
            .push(row.path.as_str());
    }
    if observed_bytes != expected_bytes {
        return Err(INVALID);
    }

    let groups = groups::rank(buckets);
    let duplicate_file_count = groups.iter().map(|group| group.paths.len()).sum();
    let redundant_bytes: u128 = groups.iter().map(groups::Group::redundant_bytes).sum();
    let mut projection_truncated = groups.len() > MAX_GROUPS;
    let mut remaining_path_bytes = MAX_DISPLAY_PATH_BYTES;
    let visible = groups
        .iter()
        .take(MAX_GROUPS)
        .map(|group| {
            let mut paths = Vec::new();
            for path in group.paths.iter().take(MAX_PATHS_PER_GROUP) {
                if path.len() > remaining_path_bytes {
                    break;
                }
                remaining_path_bytes -= path.len();
                paths.push((*path).to_owned());
            }
            projection_truncated |= paths.len() < group.paths.len();
            ReportGroup {
                logical_bytes: group.size.to_string(),
                sha256: group.sha256(),
                redundant_bytes: group.redundant_bytes().to_string(),
                file_count: group.paths.len(),
                paths,
            }
        })
        .collect();

    Ok(Report {
        ok: true,
        schema_version: 1,
        material: "duplicate-files-report",
        partial: snapshot.partial,
        group_count: groups.len(),
        duplicate_file_count,
        redundant_bytes: redundant_bytes.to_string(),
        groups: visible,
        projection_truncated,
        source: Source {
            visited_entries: snapshot.visited_entries,
            candidate_files: snapshot.candidate_files,
            hashed_files: snapshot.hashed_files,
            hashed_bytes: snapshot.hashed_bytes,
            skipped_links: snapshot.skipped_links,
            skipped_depth: snapshot.skipped_depth,
            skipped_names: snapshot.skipped_names,
            unreadable: snapshot.unreadable,
            changed_files: snapshot.changed_files,
            skipped_budget: snapshot.skipped_budget,
            limit_reason: snapshot.limit_reason,
            partial: snapshot.partial,
        },
    })
}
