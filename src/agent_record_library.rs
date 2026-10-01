//! Bounded, explicit and transient search over exported Agent records.
use crate::agent_record::{self, ImportedRecord, MAX_IMPORT_BYTES};
use anyhow::{Result, bail, ensure};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_DIRECTORY_ENTRIES: usize = 4096;
pub const MAX_RECORD_FILES: usize = 64;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct Entry {
    pub file_name: String,
    pub record: ImportedRecord,
}

pub struct ScanResult {
    pub entries: Vec<Entry>,
    pub rejected: usize,
}

pub fn scan_folder(folder: &Path, cancelled: &AtomicBool) -> Result<ScanResult> {
    let folder_type = std::fs::symlink_metadata(folder)?.file_type();
    ensure!(
        folder_type.is_dir() && !folder_type.is_symlink(),
        "请选择普通目录"
    );
    let mut paths = Vec::new();
    for (index, item) in std::fs::read_dir(folder)?.enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            bail!("目录读取已取消");
        }
        ensure!(
            index < MAX_DIRECTORY_ENTRIES,
            "目录条目超过 4096 个，请选择更小的目录"
        );
        let item = item?;
        let path = item.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            paths.push(path);
            ensure!(
                paths.len() <= MAX_RECORD_FILES,
                "JSON 文件超过 64 个，请选择更小的目录"
            );
        }
    }
    paths.sort();
    let mut result = ScanResult {
        entries: Vec::new(),
        rejected: 0,
    };
    let mut total_bytes = 0_u64;
    for path in paths {
        if cancelled.load(Ordering::Relaxed) {
            bail!("目录读取已取消");
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_IMPORT_BYTES as u64
        {
            result.rejected += 1;
            continue;
        }
        total_bytes += metadata.len();
        ensure!(
            total_bytes <= MAX_TOTAL_BYTES,
            "JSON 文件累计超过 16 MiB，请选择更小的目录"
        );
        match agent_record::load_file(&path) {
            Ok(record) => result.entries.push(Entry {
                file_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "记录.json".into()),
                record,
            }),
            Err(_) => result.rejected += 1,
        }
    }
    result.entries.sort_by(|a, b| {
        let a_time = chrono::DateTime::parse_from_rfc3339(&a.record.finished_at_utc)
            .expect("validated timestamp");
        let b_time = chrono::DateTime::parse_from_rfc3339(&b.record.finished_at_utc)
            .expect("validated timestamp");
        b_time
            .cmp(&a_time)
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    Ok(result)
}

pub fn matches(entry: &Entry, query: &str, include_content: bool) -> bool {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return true;
    }
    let metadata = format!(
        "{} {} {} {} {} {}",
        entry.file_name,
        entry.record.finished_at_utc,
        entry.record.status,
        status_label(&entry.record.status),
        entry.record.model,
        entry.record.allowed_tools.join(" ")
    )
    .to_lowercase();
    if terms.iter().all(|term| metadata.contains(term)) {
        return true;
    }
    let Some(content) = entry.record.content.as_ref().filter(|_| include_content) else {
        return false;
    };
    let content = format!(
        "{} {} {} {}",
        content.goal,
        content.plan,
        content.answer.as_deref().unwrap_or_default(),
        content.error.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    terms
        .iter()
        .all(|term| metadata.contains(term) || content.contains(term))
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "completed" => "已完成",
        "failed" => "失败",
        "cancelled" => "已取消",
        _ => "未知",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, sync::atomic::AtomicBool};

    fn record(goal: &str, time: &str) -> String {
        json!({
            "schema":"zi-devtools-agent-run","schema_version":1,
            "finished_at_utc":time,"status":"completed","model":"local-model",
            "approved":true,"allowed_tools":["search_knowledge"],"max_calls":2,
            "calls_made":0,"model_tokens_reported":10,"steps":[],
            "content":{"goal":goal,"plan":"read","answer":"done","error":null}
        })
        .to_string()
    }

    #[test]
    fn scan_is_bounded_and_search_hides_content_by_default() {
        let dir = std::env::temp_dir().join(format!("zi-agent-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("old.json"),
            record("private needle", "2026-10-01T01:00:00Z"),
        )
        .unwrap();
        fs::write(
            dir.join("new.json"),
            record("other", "2026-10-01T02:00:00Z"),
        )
        .unwrap();
        fs::write(dir.join("bad.json"), "{bad").unwrap();
        fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let result = scan_folder(&dir, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.rejected, 1);
        assert_eq!(result.entries[0].file_name, "new.json");
        assert!(matches(
            &result.entries[0],
            "local-model search_knowledge",
            false
        ));
        assert!(matches(&result.entries[0], "已完成", false));
        assert!(!matches(&result.entries[1], "needle", false));
        assert!(matches(&result.entries[1], "needle", true));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn too_many_files_and_cancel_do_not_return_partial_results() {
        let dir = std::env::temp_dir().join(format!("zi-agent-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        for index in 0..=MAX_RECORD_FILES {
            fs::write(dir.join(format!("{index}.json")), "{}").unwrap();
        }
        assert!(scan_folder(&dir, &AtomicBool::new(false)).is_err());
        assert!(scan_folder(&dir, &AtomicBool::new(true)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn aggregate_budget_is_enforced_and_oversized_file_is_skipped() {
        let dir = std::env::temp_dir().join(format!("zi-agent-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let oversized = fs::File::create(dir.join("oversized.json")).unwrap();
        oversized.set_len((MAX_IMPORT_BYTES + 1) as u64).unwrap();
        let initial = scan_folder(&dir, &AtomicBool::new(false)).unwrap();
        assert_eq!(initial.rejected, 1);
        for index in 0..17 {
            let file = fs::File::create(dir.join(format!("{index:02}.json"))).unwrap();
            file.set_len(MAX_IMPORT_BYTES as u64).unwrap();
        }
        assert!(scan_folder(&dir, &AtomicBool::new(false)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fractional_utc_timestamps_sort_chronologically() {
        let dir = std::env::temp_dir().join(format!("zi-agent-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("first.json"),
            record("first", "2026-10-01T01:00:00Z"),
        )
        .unwrap();
        fs::write(
            dir.join("later.json"),
            record("later", "2026-10-01T01:00:00.500Z"),
        )
        .unwrap();
        let result = scan_folder(&dir, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries[0].file_name, "later.json");
        fs::remove_dir_all(dir).unwrap();
    }
}
