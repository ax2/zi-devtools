//! Versioned local prompt files. User action is required to save, apply or delete.
use crate::prompt_template::Revision;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

const MAX_FILES: usize = 64;
const MAX_REVISIONS: usize = 20;
const MAX_FILE_BYTES: usize = 512 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    version: u32,
    revisions: Vec<Revision>,
}

#[derive(Clone)]
pub struct Entry {
    pub id: String,
    pub latest: Revision,
    pub revision_count: usize,
}

#[derive(Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub skipped: usize,
    pub truncated: bool,
}

pub struct Library {
    root: PathBuf,
}

fn validate_id(id: &str) -> Result<()> {
    let parsed = uuid::Uuid::parse_str(id).context("模板 ID 无效")?;
    ensure!(parsed.to_string() == id, "模板 ID 无效");
    Ok(())
}

impl Document {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == "zi-devtools-prompt" && self.version == 1,
            "模板文件版本无效"
        );
        ensure!(
            !self.revisions.is_empty() && self.revisions.len() <= MAX_REVISIONS,
            "模板修订数量无效"
        );
        for revision in &self.revisions {
            revision.validate()?;
        }
        Ok(())
    }
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }

    fn load_file(&self, id: &str) -> Result<Document> {
        let path = self.path(id)?;
        let meta = fs::symlink_metadata(&path)?;
        ensure!(meta.file_type().is_file(), "模板不是普通文件");
        ensure!(meta.len() <= MAX_FILE_BYTES as u64, "模板文件超过 512 KiB");
        let mut bytes = Vec::new();
        File::open(path)?
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_FILE_BYTES, "模板文件超过 512 KiB");
        let document: Document = serde_json::from_slice(&bytes).context("模板 JSON 格式无效")?;
        document.validate()?;
        Ok(document)
    }

    pub fn list(&self) -> Result<Listing> {
        let dir = match fs::read_dir(&self.root) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Listing::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut listing = Listing::default();
        for (index, item) in dir.enumerate() {
            if index == 256 {
                listing.truncated = true;
                break;
            }
            let path = item?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                listing.skipped += 1;
                continue;
            };
            match self.load_file(id) {
                Ok(document) if listing.entries.len() < MAX_FILES => {
                    listing.entries.push(Entry {
                        id: id.into(),
                        latest: document.revisions.last().unwrap().clone(),
                        revision_count: document.revisions.len(),
                    });
                }
                _ => listing.skipped += 1,
            }
        }
        listing.entries.sort_by(|a, b| {
            b.latest
                .saved_at
                .cmp(&a.latest.saved_at)
                .then(a.id.cmp(&b.id))
        });
        Ok(listing)
    }

    pub fn load(&self, id: &str) -> Result<Vec<Revision>> {
        Ok(self.load_file(id)?.revisions)
    }

    fn unique_name(&self, name: &str, except: Option<&str>) -> Result<()> {
        let listing = self.list()?;
        ensure!(!listing.truncated, "模板目录文件过多，请先整理目录");
        ensure!(
            !listing.entries.iter().any(|entry| {
                Some(entry.id.as_str()) != except
                    && entry.latest.name.to_lowercase() == name.to_lowercase()
            }),
            "已有同名模板，请更新选中模板或换一个名称"
        );
        Ok(())
    }

    pub fn save_new(&self, revision: Revision) -> Result<Entry> {
        revision.validate()?;
        let listing = self.list()?;
        ensure!(!listing.truncated, "模板目录文件过多，请先整理目录");
        ensure!(listing.entries.len() < MAX_FILES, "模板库最多保存 64 份");
        self.unique_name(&revision.name, None)?;
        let id = uuid::Uuid::new_v4().to_string();
        let document = Document {
            schema: "zi-devtools-prompt".into(),
            version: 1,
            revisions: vec![revision.clone()],
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
        ensure!(bytes.len() <= MAX_FILE_BYTES, "模板文件超过 512 KiB");
        fs::create_dir_all(&self.root)?;
        let path = self.path(&id)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let result = file.write_all(&bytes).and_then(|_| file.sync_all());
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        result?;
        Ok(Entry {
            id,
            latest: revision,
            revision_count: 1,
        })
    }

    pub fn save_revision(&self, id: &str, revision: Revision) -> Result<Entry> {
        revision.validate()?;
        let mut document = self.load_file(id)?;
        self.unique_name(&revision.name, Some(id))?;
        ensure!(
            document.revisions.len() < MAX_REVISIONS,
            "模板最多保留 20 个修订"
        );
        let previous = document.revisions.last().unwrap();
        ensure!(
            previous.name != revision.name
                || previous.tags != revision.tags
                || previous.body != revision.body
                || previous.options != revision.options,
            "模板内容与当前版本相同"
        );
        document.revisions.push(revision.clone());
        let bytes = serde_json::to_vec_pretty(&document)?;
        ensure!(bytes.len() <= MAX_FILE_BYTES, "模板文件超过 512 KiB");
        let path = self.path(id)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(Entry {
            id: id.into(),
            latest: revision,
            revision_count: document.revisions.len(),
        })
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path(id)?;
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "模板不是普通文件"
        );
        fs::remove_file(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_template::{RunOptions, parse_tags};

    fn revision(name: &str, body: &str) -> Revision {
        Revision::new(
            name,
            parse_tags("文档,Rust").unwrap(),
            body,
            RunOptions {
                tool_id: "plugin:openai-local/chat".into(),
                model: "local-model".into(),
                stream: true,
                multi_turn: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn save_revision_reload_search_and_delete_preserve_old_version() {
        let root = std::env::temp_dir().join(format!("zi-prompts-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        let first = library
            .save_new(revision("审查", "旧 {{topic}}\n"))
            .unwrap();
        assert!(library.save_new(revision("审查", "duplicate")).is_err());
        assert_eq!(library.list().unwrap().entries.len(), 1);
        let revised = library
            .save_revision(&first.id, revision("审查", "新 {{topic}}\n"))
            .unwrap();
        assert_eq!(revised.revision_count, 2);
        let versions = library.load(&first.id).unwrap();
        assert_eq!(versions[0].body, "旧 {{topic}}\n");
        assert_eq!(versions[1].body, "新 {{topic}}\n");
        assert!(
            library
                .save_revision(&first.id, versions[1].clone())
                .is_err()
        );
        assert!(library.load("../escape").is_err());
        library.delete(&first.id).unwrap();
        assert!(library.list().unwrap().entries.is_empty());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn malformed_file_is_skipped_and_does_not_overwrite_a_good_template() {
        let root = std::env::temp_dir().join(format!("zi-prompts-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        let saved = library.save_new(revision("good", "body")).unwrap();
        let invalid = root.join(format!("{}.json", uuid::Uuid::new_v4()));
        fs::write(&invalid, "malformed").unwrap();
        let listing = library.list().unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.skipped, 1);
        let path = root.join(format!("{}.json", saved.id));
        let prior = fs::read(&path).unwrap();
        let mut bad_revision = revision("bad", "valid");
        bad_revision.body = "{{illegal-name}}".into();
        assert!(library.save_revision(&saved.id, bad_revision).is_err());
        assert_eq!(fs::read(&path).unwrap(), prior);
        fs::remove_file(invalid).unwrap();
        library.delete(&saved.id).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn template_and_revision_limits_do_not_replace_existing_content() {
        let root = std::env::temp_dir().join(format!("zi-prompts-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        let saved = library.save_new(revision("first", "version 1")).unwrap();
        for version in 2..=MAX_REVISIONS {
            library
                .save_revision(&saved.id, revision("first", &format!("version {version}")))
                .unwrap();
        }
        let original_path = root.join(format!("{}.json", saved.id));
        let original_bytes = fs::read(&original_path).unwrap();
        assert!(
            library
                .save_revision(&saved.id, revision("first", "overflow"))
                .is_err()
        );
        assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
        for _ in 1..MAX_FILES {
            fs::write(
                root.join(format!("{}.json", uuid::Uuid::new_v4())),
                &original_bytes,
            )
            .unwrap();
        }
        assert_eq!(library.list().unwrap().entries.len(), MAX_FILES);
        assert!(library.save_new(revision("new", "body")).is_err());
        assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
        for file in fs::read_dir(&root).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
}
