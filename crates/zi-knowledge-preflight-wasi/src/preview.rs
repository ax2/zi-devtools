//! Classify names in one directory level without opening any file.

use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};
use zi_knowledge_source_core::{Exclusions, Format, format, parse_excludes};

pub const MAX_INPUT: usize = 64 * 1024;
pub const MAX_RESULT: usize = 24 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_NAME_BYTES: usize = 12 * 1024;
const MAX_RULE_BYTES: usize = 960;
const MAX_RULES: usize = 8;
const MAX_SHOWN: usize = 24;
const PROFILE: &str = "resource.local-directory.draft1";

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    excludes: String,
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

#[derive(Default, Serialize)]
struct Counts {
    markdown: usize,
    text: usize,
    html: usize,
    pdf: usize,
    docx: usize,
    image: usize,
}

impl Counts {
    fn add(&mut self, format: Format) {
        let group = match format {
            Format::Markdown => &mut self.markdown,
            Format::Text => &mut self.text,
            Format::Html => &mut self.html,
            Format::Pdf => &mut self.pdf,
            Format::Docx => &mut self.docx,
            Format::Image => &mut self.image,
        };
        *group += 1;
    }
}

#[derive(Serialize)]
struct Candidate {
    name: String,
    format: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    ok: bool,
    file_contents_checked: bool,
    entry_count: usize,
    eligible_count: usize,
    excluded_count: usize,
    unsupported_count: usize,
    directories_not_scanned: usize,
    candidates_truncated: bool,
    format_counts: Counts,
    candidates: Vec<Candidate>,
}

pub fn analyze(bytes: &[u8], options: &Options) -> Result<Report, &'static str> {
    if bytes.len() > MAX_INPUT {
        return Err("INPUT_TOO_LARGE");
    }
    let mut snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| "INVALID_SNAPSHOT")?;
    if snapshot.schema_version != 1 || snapshot.profile != PROFILE {
        return Err("INVALID_SNAPSHOT");
    }
    let entry_count = snapshot.entries.len();
    if entry_count > MAX_ENTRIES {
        return Err("TOO_MANY_ENTRIES");
    }
    let name_bytes: usize = snapshot.entries.iter().map(|entry| entry.name.len()).sum();
    if name_bytes > MAX_NAME_BYTES {
        return Err("NAME_BUDGET_EXCEEDED");
    }
    if options.excludes.len() > MAX_RULE_BYTES {
        return Err("RULE_BUDGET_EXCEEDED");
    }
    let rules = parse_excludes(&options.excludes).map_err(|_| "INVALID_RULES")?;
    if rules.len() > MAX_RULES {
        return Err("RULE_BUDGET_EXCEEDED");
    }
    snapshot
        .entries
        .sort_unstable_by(|a, b| a.name.cmp(&b.name));
    let exclusions = Exclusions::new(&rules);
    let mut seen = HashSet::with_capacity(entry_count);
    let mut report = Report {
        ok: true,
        file_contents_checked: false,
        entry_count,
        eligible_count: 0,
        excluded_count: 0,
        unsupported_count: 0,
        directories_not_scanned: 0,
        candidates_truncated: false,
        format_counts: Counts::default(),
        candidates: Vec::new(),
    };
    for Entry { name, kind } in snapshot.entries {
        let lower = name.to_lowercase();
        if !is_leaf(&name) || !seen.insert(lower.clone()) {
            return Err("INVALID_SNAPSHOT");
        }
        let directory = matches!(kind, Kind::Directory);
        if exclusions.excludes_lowercase(&lower, directory) {
            report.excluded_count += 1;
        } else if directory {
            report.directories_not_scanned += 1;
        } else if let Some(format) = format(Path::new(&name)) {
            report.eligible_count += 1;
            report.format_counts.add(format);
            if report.candidates.len() < MAX_SHOWN {
                report.candidates.push(Candidate {
                    name,
                    format: format.key(),
                });
            }
        } else {
            report.unsupported_count += 1;
        }
    }
    report.candidates_truncated = report.eligible_count > report.candidates.len();
    Ok(report)
}

fn is_leaf(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && name.encode_utf16().count() <= 255
        && !name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
}
