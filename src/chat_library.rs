//! Explicitly saved local conversations. Never writes drafts, connection settings or credentials.
use crate::conversation::Conversation;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

const MAX_FILES: usize = 64;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedFile {
    schema: String,
    version: u32,
    name: String,
    saved_at: String,
    conversation: Value,
}

#[derive(Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub saved_at: String,
    pub turns: usize,
    pub bytes: usize,
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

fn validate_name(name: &str) -> Result<&str> {
    let trimmed = name.trim();
    ensure!(!trimmed.is_empty(), "请输入会话名称");
    ensure!(
        trimmed.chars().count() <= 80 && !trimmed.chars().any(char::is_control),
        "会话名称最多 80 字且不能包含控制字符"
    );
    Ok(trimmed)
}

fn validate_id(id: &str) -> Result<()> {
    let parsed = uuid::Uuid::parse_str(id).context("会话 ID 无效")?;
    ensure!(parsed.to_string() == id, "会话 ID 无效");
    Ok(())
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }

    fn load_file(&self, id: &str, binding: String) -> Result<(Entry, Conversation)> {
        let path = self.path(id)?;
        let meta = fs::symlink_metadata(&path)?;
        ensure!(meta.file_type().is_file(), "会话文件不是普通文件");
        ensure!(meta.len() <= MAX_FILE_BYTES as u64, "会话文件超过 2 MiB");
        let mut bytes = Vec::new();
        File::open(&path)?
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_FILE_BYTES, "会话文件超过 2 MiB");
        let saved: SavedFile = serde_json::from_slice(&bytes).context("会话库文件格式无效")?;
        ensure!(
            saved.schema == "zi-devtools-saved-conversation" && saved.version == 1,
            "会话库文件版本无效"
        );
        validate_name(&saved.name)?;
        ensure!(
            chrono::DateTime::parse_from_rfc3339(&saved.saved_at).is_ok(),
            "保存时间无效"
        );
        let conversation = Conversation::import(&saved.conversation.to_string(), binding)?;
        let entry = Entry {
            id: id.into(),
            name: saved.name,
            saved_at: saved.saved_at,
            turns: conversation.turns().len(),
            bytes: conversation.bytes(),
        };
        Ok((entry, conversation))
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
            let item = item?;
            let path = item.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                listing.skipped += 1;
                continue;
            };
            match self.load_file(id, String::new()) {
                Ok((entry, _)) if listing.entries.len() < MAX_FILES => listing.entries.push(entry),
                _ => listing.skipped += 1,
            }
        }
        listing
            .entries
            .sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(a.id.cmp(&b.id)));
        Ok(listing)
    }

    pub fn load(&self, id: &str, binding: String) -> Result<(Entry, Conversation)> {
        self.load_file(id, binding)
    }

    pub fn save(&self, name: &str, conversation: &Conversation) -> Result<Entry> {
        let name = validate_name(name)?;
        let listing = self.list()?;
        ensure!(!listing.truncated, "会话库文件过多，请先整理目录");
        ensure!(
            listing.entries.len() < MAX_FILES,
            "本地会话库最多保存 64 份"
        );
        let saved = SavedFile {
            schema: "zi-devtools-saved-conversation".into(),
            version: 1,
            name: name.into(),
            saved_at: chrono::Utc::now().to_rfc3339(),
            conversation: serde_json::from_str(&conversation.export(true)?)?,
        };
        fs::create_dir_all(&self.root)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.path(&id)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let result = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec_pretty(&saved)?)?;
            file.sync_all()?;
            Ok(())
        })();
        drop(file);
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        result?;
        let (entry, _) = self.load_file(&id, String::new())?;
        Ok(entry)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.path(id)?;
        ensure!(
            fs::symlink_metadata(&path)?.file_type().is_file(),
            "会话文件不是普通文件"
        );
        fs::remove_file(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_save_list_restore_and_delete_without_binding_or_draft() {
        let root = std::env::temp_dir().join(format!("zi-chat-library-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        assert!(library.list().unwrap().entries.is_empty());
        let mut chat = Conversation::default();
        chat.bind("private-binding".into());
        chat.complete("你好 $input".into(), "回答🙂".into())
            .unwrap();
        assert!(library.save("  ", &chat).is_err());
        let saved = library.save(" 项目问答 ", &chat).unwrap();
        assert_eq!(saved.name, "项目问答");
        let bytes = fs::read(root.join(format!("{}.json", saved.id))).unwrap();
        let raw = String::from_utf8(bytes).unwrap();
        assert!(!raw.contains("private-binding"));
        let listed = library.list().unwrap();
        assert_eq!(listed.entries.len(), 1);
        let (_, restored) = library.load(&saved.id, "new-binding".into()).unwrap();
        assert_eq!(restored.turns(), chat.turns());
        assert_eq!(restored.binding(), "new-binding");
        assert!(library.load("../other", String::new()).is_err());
        library.delete(&saved.id).unwrap();
        assert!(library.list().unwrap().entries.is_empty());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn corrupt_entry_is_skipped_without_hiding_valid_conversation() {
        let root = std::env::temp_dir().join(format!("zi-chat-library-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        let mut chat = Conversation::default();
        chat.complete("q".into(), "a".into()).unwrap();
        let saved = library.save("valid", &chat).unwrap();
        let bad = root.join(format!("{}.json", uuid::Uuid::new_v4()));
        fs::write(&bad, "not json").unwrap();
        let listed = library.list().unwrap();
        assert_eq!(listed.entries.len(), 1);
        assert_eq!(listed.entries[0].id, saved.id);
        assert_eq!(listed.skipped, 1);
        fs::remove_file(bad).unwrap();
        library.delete(&saved.id).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn full_library_rejects_another_save_without_replacing_files() {
        let root = std::env::temp_dir().join(format!("zi-chat-library-{}", uuid::Uuid::new_v4()));
        let library = Library::new(root.clone());
        let mut chat = Conversation::default();
        chat.complete("q".into(), "a".into()).unwrap();
        let original = library.save("first", &chat).unwrap();
        let original_path = root.join(format!("{}.json", original.id));
        let original_bytes = fs::read(&original_path).unwrap();
        for _ in 1..MAX_FILES {
            fs::write(
                root.join(format!("{}.json", uuid::Uuid::new_v4())),
                &original_bytes,
            )
            .unwrap();
        }
        assert_eq!(library.list().unwrap().entries.len(), MAX_FILES);
        assert!(library.save("overflow", &chat).is_err());
        assert_eq!(fs::read(original_path).unwrap(), original_bytes);
        for entry in fs::read_dir(&root).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }
}
