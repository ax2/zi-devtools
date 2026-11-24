//! Exact, read-only aggregation over a confirmed directory metadata snapshot.
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap};

pub const PLUGIN: &str = "com.zicode.devtools.disk-inspector";
pub const CAPABILITY: &str = "devtools.disk-inspector.scan";
pub const PROFILE: &str = "resource.local-directory.usage.draft1";
pub const MAX_HOST_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ANALYSIS_BYTES: usize = 64 * 1024;
pub const MAX_REPORT_BYTES: usize = 48 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 16;
const MAX_PATH_BYTES: usize = 2048;
const DISPLAY_PATH_BYTES: usize = 512;
const TOP: usize = 20;
const INVALID: &str = "INVALID_DIRECTORY_SNAPSHOT";
const CANONICAL_PREFIX: &[u8] =
    br#"{"schemaVersion":1,"profile":"resource.local-directory.usage.draft1","entries":["#;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Kind {
    File,
    Directory,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry<'a> {
    kind: Kind,
    #[serde(borrow)]
    path: Cow<'a, str>,
    logical_bytes: Option<&'a str>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    visited_entries: usize,
    skipped_links: usize,
    skipped_depth: usize,
    skipped_names: usize,
    unreadable: usize,
    limit_reason: Option<Limit>,
    partial: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Limit {
    Entries,
    Time,
    SnapshotBytes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot<'a> {
    schema_version: u32,
    profile: &'a str,
    #[serde(borrow)]
    entries: Vec<Entry<'a>>,
    visited_entries: usize,
    skipped_links: usize,
    skipped_depth: usize,
    skipped_names: usize,
    unreadable: usize,
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
    logical_bytes: String,
    root_files_logical_bytes: String,
    file_count: usize,
    directory_count: usize,
    group_count: usize,
    groups: Vec<Usage>,
    largest_files: Vec<Usage>,
    source: Option<Source>,
    analysis: Analysis,
    limits: Limits,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    path: String,
    logical_bytes: String,
    path_truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Analysis {
    examined_entries: usize,
    input_bytes: usize,
    read_bytes: usize,
    limit_reason: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    host_max_entries: usize,
    host_max_depth: usize,
    host_scan_seconds: usize,
    host_max_snapshot_bytes: usize,
    max_analysis_bytes: usize,
    displayed_path_bytes: usize,
    displayed_rows_per_list: usize,
}

fn prefix_entries(mut bytes: &[u8]) -> Result<Vec<Entry<'_>>, &'static str> {
    // The Host serializes version/profile/entries in this order. A prefix may
    // end inside a record; only rows whose complete JSON was read are used.
    bytes = bytes.strip_prefix(CANONICAL_PREFIX).ok_or(INVALID)?;
    let mut entries = Vec::new();
    while !matches!(bytes.first(), None | Some(b']')) {
        let mut parser = serde_json::Deserializer::from_slice(bytes).into_iter::<Entry>();
        let Some(entry) = parser.next() else {
            return Err(INVALID);
        };
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.is_eof() => break,
            Err(_) => return Err(INVALID),
        };
        entries.push(entry);
        if entries.len() > MAX_ENTRIES {
            return Err(INVALID);
        }
        bytes = &bytes[parser.byte_offset()..];
        match bytes.first() {
            Some(b',') if bytes.get(1) != Some(&b']') => bytes = &bytes[1..],
            None | Some(b']') => break,
            _ => return Err(INVALID),
        }
    }
    Ok(entries)
}

fn decode(bytes: &[u8], limited: bool) -> Result<(Vec<Entry<'_>>, Option<Source>), &'static str> {
    if limited {
        return Ok((prefix_entries(bytes)?, None));
    }
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    let Snapshot {
        schema_version,
        profile,
        entries,
        visited_entries,
        skipped_links,
        skipped_depth,
        skipped_names,
        unreadable,
        limit_reason,
        partial,
    } = snapshot;
    let counts = [skipped_links, skipped_depth, skipped_names, unreadable];
    if schema_version != 1
        || profile != PROFILE
        || entries.len() > visited_entries
        || visited_entries > MAX_ENTRIES
        || counts.iter().any(|&count| count > visited_entries)
        || partial != (limit_reason.is_some() || counts.iter().any(|&count| count != 0))
        || (!partial && entries.len() != visited_entries)
    {
        return Err(INVALID);
    }
    Ok((
        entries,
        Some(Source {
            visited_entries,
            skipped_links,
            skipped_depth,
            skipped_names,
            unreadable,
            limit_reason,
            partial,
        }),
    ))
}

fn first_segment(path: &str) -> Result<(&str, bool), &'static str> {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.bytes().any(|byte| {
            byte < 32
                || byte == 127
                || matches!(byte, b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*')
        })
    {
        return Err(INVALID);
    }
    let mut segments = path.split('/');
    let first = segments.next().ok_or(INVALID)?;
    let mut depth = 0;
    for part in std::iter::once(first).chain(segments.by_ref()) {
        if part.is_empty() || part.ends_with(['.', ' ']) {
            return Err(INVALID);
        }
        depth += 1;
    }
    if depth > MAX_DEPTH + 1 {
        return Err(INVALID);
    }
    Ok((first, depth > 1))
}

fn logical_bytes(value: Option<&str>) -> Result<u64, &'static str> {
    let bytes = value.ok_or(INVALID)?;
    if bytes.is_empty()
        || bytes.len() > 20
        || !bytes.bytes().all(|byte| byte.is_ascii_digit())
        || (bytes.len() > 1 && bytes.starts_with('0'))
    {
        return Err(INVALID);
    }
    bytes.parse().map_err(|_| INVALID)
}

fn top(mut rows: Vec<(&str, u128)>) -> Vec<Usage> {
    rows.sort_unstable_by(|(left_path, left), (right_path, right)| {
        right.cmp(left).then_with(|| left_path.cmp(right_path))
    });
    rows.into_iter()
        .take(TOP)
        .map(|(path, bytes)| {
            let path_truncated = path.len() > DISPLAY_PATH_BYTES;
            let mut end = path.len().min(DISPLAY_PATH_BYTES);
            while !path.is_char_boundary(end) {
                end -= 1;
            }
            Usage {
                path: path[..end].into(),
                logical_bytes: bytes.to_string(),
                path_truncated,
            }
        })
        .collect()
}

pub fn analyze(bytes: &[u8], input_bytes: usize) -> Result<Report, &'static str> {
    if input_bytes == 0
        || input_bytes > MAX_HOST_BYTES
        || bytes.len() != input_bytes.min(MAX_ANALYSIS_BYTES)
    {
        return Err(INVALID);
    }
    let limited = input_bytes > bytes.len();
    let (entries, source) = decode(bytes, limited)?;
    let mut groups = BTreeMap::<&str, u128>::new();
    let mut largest = Vec::new();
    let (mut total, mut root_files, mut directory_count) = (0u128, 0u128, 0usize);
    let mut last = "";
    for entry in &entries {
        let path = entry.path.as_ref();
        let (first, nested) = first_segment(path)?;
        if path <= last {
            return Err(INVALID);
        }
        last = path;
        let group = if nested {
            Some(groups.get_mut(first).ok_or(INVALID)?)
        } else {
            None
        };
        match entry.kind {
            Kind::Directory => {
                if entry.logical_bytes.is_some() {
                    return Err(INVALID);
                }
                directory_count += 1;
                if !nested {
                    groups.insert(first, 0);
                }
            }
            Kind::File => {
                let count = u128::from(logical_bytes(entry.logical_bytes)?);
                total += count;
                if let Some(group) = group {
                    *group += count;
                } else {
                    root_files += count;
                }
                largest.push((path, count));
            }
        }
    }
    let partial = limited || source.as_ref().is_some_and(|source| source.partial);
    Ok(Report {
        ok: true,
        schema_version: 1,
        material: "directory-usage-report",
        partial,
        logical_bytes: total.to_string(),
        root_files_logical_bytes: root_files.to_string(),
        file_count: largest.len(),
        directory_count,
        group_count: groups.len(),
        groups: top(groups.into_iter().collect()),
        largest_files: top(largest),
        source,
        analysis: Analysis {
            examined_entries: entries.len(),
            input_bytes,
            read_bytes: bytes.len(),
            limit_reason: limited.then_some("analysis_bytes"),
        },
        limits: Limits {
            host_max_entries: MAX_ENTRIES,
            host_max_depth: MAX_DEPTH,
            host_scan_seconds: 15,
            host_max_snapshot_bytes: MAX_HOST_BYTES,
            max_analysis_bytes: MAX_ANALYSIS_BYTES,
            displayed_path_bytes: DISPLAY_PATH_BYTES,
            displayed_rows_per_list: TOP,
        },
    })
}
