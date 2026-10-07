use super::{Definition, LIMIT};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_VISITED: usize = 1000;
const MAX_ENTRIES: usize = 256;
const MAX_READ: usize = 8 * 1024 * 1024;

pub(crate) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub steps: usize,
}
impl Entry {
    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {}",
            self.name,
            self.path.file_name().unwrap_or_default().to_string_lossy()
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}
#[derive(Default)]
pub(crate) struct Listing {
    pub entries: Vec<Entry>,
    pub skipped: usize,
    pub partial: bool,
}
fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
pub(super) fn scan(folder: &Path) -> Result<Listing> {
    ensure!(folder.is_absolute(), "请选择绝对路径的流程文件夹");
    let metadata = fs::symlink_metadata(folder).context("无法访问流程文件夹")?;
    ensure!(
        metadata.is_dir() && !linked(&metadata),
        "请选择普通文件夹，不能使用链接或重解析目录"
    );
    let mut listing = Listing::default();
    let mut total = 0;
    for (index, item) in fs::read_dir(folder)
        .context("无法列出流程文件夹")?
        .enumerate()
    {
        if index >= MAX_VISITED || listing.entries.len() >= MAX_ENTRIES {
            listing.partial = true;
            break;
        }
        let Ok(item) = item else {
            listing.skipped += 1;
            continue;
        };
        let path = item.path();
        if !path
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        let result = (|| -> Result<Entry> {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(metadata.is_file() && !linked(&metadata), "不是普通流程文件");
            ensure!(metadata.len() <= LIMIT as u64, "流程文件过大");
            let remaining = MAX_READ.saturating_sub(total);
            ensure!(remaining > 0, "累计读取达到上限");
            let mut bytes = Vec::new();
            fs::File::open(&path)?
                .take((LIMIT + 1).min(remaining) as u64)
                .read_to_end(&mut bytes)?;
            total += bytes.len();
            ensure!(bytes.len() <= LIMIT, "流程文件过大");
            let definition = Definition::parse(&bytes)?;
            Ok(Entry {
                path,
                name: definition.name,
                steps: definition.steps.len(),
            })
        })();
        match result {
            Ok(entry) => listing.entries.push(entry),
            Err(_) => listing.skipped += 1,
        }
        if total >= MAX_READ {
            listing.partial = true;
            break;
        }
    }
    listing
        .entries
        .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    Ok(listing)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_valid_definitions_only_searches_and_does_not_recurse() {
        let folder = std::env::temp_dir().join(format!("zi-flow-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&folder).unwrap();
        let definition = Definition {
            version: 2,
            name: "Daily clean".into(),
            steps: vec![super::super::super::workflow::Step::SelectColumns {
                columns: vec!["name".into()],
            }],
        };
        let bytes = serde_json::to_vec(&definition).unwrap();
        fs::write(folder.join("cleanup.JSON"), &bytes).unwrap();
        fs::write(folder.join("broken.json"), b"bad").unwrap();
        fs::write(folder.join("large.json"), vec![b' '; LIMIT + 1]).unwrap();
        fs::create_dir(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/hidden.json"), &bytes).unwrap();
        let listing = scan(&folder).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.skipped, 2);
        assert!(!listing.partial);
        assert!(listing.entries[0].matches("daily CLEANUP"));
        assert!(!listing.entries[0].matches("missing"));
        assert_eq!(fs::read(folder.join("cleanup.JSON")).unwrap(), bytes);
        let mut changed = definition.clone();
        changed.name = "Changed after scan".into();
        fs::write(
            &listing.entries[0].path,
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert_eq!(
            super::super::load(&listing.entries[0].path).unwrap().name,
            changed.name
        );
        fs::remove_file(&listing.entries[0].path).unwrap();
        assert!(super::super::load(&listing.entries[0].path).is_err());
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn entry_limit_is_reported_as_partial() {
        let folder = std::env::temp_dir().join(format!("zi-flow-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&folder).unwrap();
        let definition = Definition {
            version: 2,
            name: "Recipe".into(),
            steps: vec![super::super::super::workflow::Step::SelectColumns {
                columns: vec!["name".into()],
            }],
        };
        let bytes = serde_json::to_vec(&definition).unwrap();
        for index in 0..MAX_ENTRIES + 1 {
            fs::write(folder.join(format!("{index}.json")), &bytes).unwrap();
        }
        let listing = scan(&folder).unwrap();
        assert_eq!(listing.entries.len(), MAX_ENTRIES);
        assert!(listing.partial);
        assert!(scan(Path::new("relative")).is_err());
        fs::remove_dir_all(folder).unwrap();
    }
}
