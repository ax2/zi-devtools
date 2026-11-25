//! Derive advisory names from a complete, single-level snapshot.

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use zi_rename_core::{Rules, validate_component};

pub const MAX_INPUT: usize = 64 * 1024;
pub const MAX_ENTRIES: usize = 64;
pub const MAX_NAME_BYTES: usize = 12 * 1024;
pub const MAX_TARGET_NAME_BYTES: usize = 16 * 1024;
pub const MAX_RULE_BYTES: usize = 128;
pub const MAX_RESULT: usize = 48 * 1024;
const PROFILE: &str = "resource.local-directory.draft1";

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Options {
    find: String,
    replacement: String,
    prefix: String,
    suffix: String,
    case_sensitive: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Snapshot {
    schema_version: u32,
    profile: String,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    kind: Kind,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    File,
    Directory,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    Proposed,
    Unchanged,
    CaseOnly,
    Duplicate,
    Occupied,
    Invalid,
}

#[derive(Serialize)]
struct Row {
    source: String,
    target: Option<String>,
    status: Status,
    issue: Option<String>,
    #[serde(skip)]
    target_key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    ok: bool,
    preview_only: bool,
    entry_count: usize,
    file_count: usize,
    proposed_count: usize,
    unchanged_count: usize,
    blocked_count: usize,
    rows: Vec<Row>,
}

pub fn analyze(bytes: &[u8], options: &Options) -> Result<Report, &'static str> {
    if bytes.len() > MAX_INPUT {
        return Err("INPUT_TOO_LARGE");
    }
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| "INVALID_SNAPSHOT")?;
    if snapshot.schema_version != 1 || snapshot.profile != PROFILE {
        return Err("INVALID_SNAPSHOT");
    }
    if snapshot.entries.len() > MAX_ENTRIES {
        return Err("TOO_MANY_ENTRIES");
    }
    let name_bytes: usize = snapshot.entries.iter().map(|entry| entry.name.len()).sum();
    if name_bytes > MAX_NAME_BYTES {
        return Err("NAME_BUDGET_EXCEEDED");
    }
    let entry_count = snapshot.entries.len();
    let mut occupied = HashSet::with_capacity(entry_count);
    let mut files = Vec::with_capacity(entry_count);
    for entry in snapshot.entries {
        let folded = entry.name.to_lowercase();
        if validate_component(&entry.name).is_err() || !occupied.insert(folded.clone()) {
            return Err("INVALID_SNAPSHOT");
        }
        if matches!(entry.kind, Kind::File) {
            files.push((entry.name, folded));
        }
    }

    let rules = Rules {
        find: &options.find,
        replacement: &options.replacement,
        prefix: &options.prefix,
        suffix: &options.suffix,
        case_sensitive: options.case_sensitive,
    };
    if [rules.find, rules.replacement, rules.prefix, rules.suffix]
        .iter()
        .any(|value| value.len() > MAX_RULE_BYTES)
    {
        return Err("RULE_TOO_LARGE");
    }

    let no_rule = rules.find.is_empty() && rules.prefix.is_empty() && rules.suffix.is_empty();
    let mut rows = Vec::with_capacity(files.len());
    let mut destinations = HashMap::<String, usize>::new();
    let mut target_bytes = 0;
    for (source, source_key) in files {
        if no_rule {
            rows.push(Row {
                target: Some(source.clone()),
                source,
                status: Status::Unchanged,
                issue: None,
                target_key: None,
            });
            continue;
        }
        let path = Path::new(&source);
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("INVALID_SNAPSHOT")?;
        let extension = path.extension().and_then(|extension| extension.to_str());
        let (target, status, issue, target_key) = match rules.target_name(stem, extension) {
            Err(error) => (None, Status::Invalid, Some(error.to_string()), None),
            Ok(target) => {
                target_bytes += target.len();
                if target_bytes > MAX_TARGET_NAME_BYTES {
                    return Err("TARGET_BUDGET_EXCEEDED");
                }
                if target == source {
                    rows.push(Row {
                        source,
                        target: Some(target),
                        status: Status::Unchanged,
                        issue: None,
                        target_key: None,
                    });
                    continue;
                }
                let folded = target.to_lowercase();
                if folded == source_key {
                    (Some(target), Status::CaseOnly, None, None)
                } else {
                    *destinations.entry(folded.clone()).or_default() += 1;
                    (Some(target), Status::Proposed, None, Some(folded))
                }
            }
        };
        rows.push(Row {
            source,
            target,
            status,
            issue,
            target_key,
        });
    }

    let mut proposed_count = 0;
    let mut unchanged_count = 0;
    for row in &mut rows {
        match row.status {
            Status::Proposed => {
                let folded = row.target_key.as_deref().ok_or("RESULT_FAILED")?;
                if destinations[folded] > 1 {
                    row.status = Status::Duplicate;
                } else if occupied.contains(folded) {
                    row.status = Status::Occupied;
                } else {
                    proposed_count += 1;
                }
            }
            Status::Unchanged => unchanged_count += 1,
            _ => {}
        }
    }

    let file_count = rows.len();
    Ok(Report {
        ok: true,
        preview_only: true,
        entry_count,
        file_count,
        proposed_count,
        unchanged_count,
        blocked_count: file_count - proposed_count - unchanged_count,
        rows,
    })
}
