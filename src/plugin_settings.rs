//! Ordinary connection settings only. Credentials live in the Windows vault.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub endpoint: String,
    pub model: String,
}
impl Connection {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.endpoint.len() <= 4096, "接口地址过长");
        crate::plugins::endpoint(&self.endpoint)?;
        ensure!(
            self.model.len() <= 256 && !self.model.chars().any(char::is_control),
            "模型名称最多 256 字节且不能包含控制字符"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub shape: String,
    pub connection: Connection,
}
impl Profile {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 160
                && !self.name.chars().any(char::is_control),
            "档案名称须为 1–160 字节，不能包含控制字符"
        );
        ensure!(valid_key(&self.shape), "档案协议标识无效");
        self.connection.validate()
    }
    pub fn for_tool(&self, tool: &crate::plugins::PluginTool) -> Result<Connection> {
        ensure!(
            request_shape(tool).as_deref() == Some(self.shape.as_str()),
            "档案与当前工具的请求格式不兼容"
        );
        self.validate()?;
        Ok(self.connection.clone())
    }
}
/// Exact request/response template compatibility, independent of destination and model.
pub fn request_shape(tool: &crate::plugins::PluginTool) -> Option<String> {
    if let crate::plugins::Adapter::Http {
        method,
        body,
        response_pointer,
        ..
    } = &tool.adapter
    {
        Some(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(method, body, response_pointer))
                    .expect("serializable adapter")
            )
        ))
    } else {
        None
    }
}
fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit())
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    connections: BTreeMap<String, Connection>,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
}
pub struct Settings {
    path: PathBuf,
    data: Document,
    pub error: Option<String>,
}
impl Settings {
    pub fn load(path: PathBuf) -> Self {
        let result = Self::read(&path);
        match result {
            Ok(data) => Self { path, data, error: None },
            Err(_) => Self { path, data: Document::default(), error: Some("连接设置读取失败；为保留原文件，已禁用保存。请修复 settings/connections.json 后重启。".into()) },
        }
    }
    fn read(path: &Path) -> Result<Document> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Document::default()),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "设置文件过大");
        let data: Document = serde_json::from_slice(&bytes)?;
        ensure!(data.connections.len() <= 512, "连接设置过多");
        for (key, connection) in &data.connections {
            ensure!(
                key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit()),
                "设置标识无效"
            );
            connection.validate()?;
        }
        ensure!(data.profiles.len() <= 128, "连接档案过多");
        let mut names = std::collections::BTreeSet::new();
        for (id, profile) in &data.profiles {
            ensure!(uuid::Uuid::parse_str(id).is_ok(), "档案 ID 无效");
            profile.validate()?;
            ensure!(
                names.insert(profile.name.trim().to_lowercase()),
                "档案名称重复"
            );
        }
        Ok(data)
    }
    /// A changed tool definition never silently inherits an old destination override.
    pub fn key(id: &str, tool: &crate::plugins::PluginTool) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(id, tool)).expect("serializable tool"))
        )
    }
    pub fn get(&self, key: &str) -> Option<&Connection> {
        self.data.connections.get(key)
    }
    pub fn profiles(&self) -> impl Iterator<Item = (&String, &Profile)> {
        self.data.profiles.iter()
    }
    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.data.profiles.get(id)
    }
    pub fn save_profile(&mut self, id: Option<&str>, mut profile: Profile) -> Result<String> {
        profile.name = profile.name.trim().into();
        profile.validate()?;
        ensure!(
            !self
                .data
                .profiles
                .iter()
                .any(|(key, p)| Some(key.as_str()) != id
                    && p.name.to_lowercase() == profile.name.to_lowercase()),
            "已有同名档案，请使用其他名称或更新选中档案"
        );
        let id = match id {
            Some(id) => {
                ensure!(self.data.profiles.contains_key(id), "选中档案已不存在");
                id.to_owned()
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        let mut next = self.data.clone();
        next.profiles.insert(id.clone(), profile);
        ensure!(next.profiles.len() <= 128, "最多保存 128 个连接档案");
        self.persist(next)?;
        Ok(id)
    }
    pub fn delete_profile(&mut self, id: &str) -> Result<()> {
        let mut next = self.data.clone();
        ensure!(next.profiles.remove(id).is_some(), "选中档案已不存在");
        self.persist(next)
    }
    pub fn set(&mut self, key: &str, value: Option<Connection>) -> Result<()> {
        ensure!(self.error.is_none(), "设置文件异常，未覆盖原文件");
        ensure!(
            key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit()),
            "设置标识无效"
        );
        if let Some(v) = &value {
            v.validate()?;
        }
        let mut next = self.data.clone();
        if let Some(value) = value {
            next.connections.insert(key.into(), value);
        } else {
            next.connections.remove(key);
        }
        ensure!(next.connections.len() <= 512, "最多保存 512 个工具连接设置");
        self.persist(next)
    }
    fn persist(&mut self, next: Document) -> Result<()> {
        ensure!(self.error.is_none(), "设置文件异常，未覆盖原文件");
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(bytes.len() <= 1024 * 1024, "设置文件过大");
        let parent = self.path.parent().context("设置路径无效")?;
        fs::create_dir_all(parent)?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.context("连接设置保存失败")?;
        self.data = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_migrate_reuse_update_and_delete_without_changing_tool_snapshots() {
        let root = std::env::temp_dir().join(format!("zi-profiles-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("connections.json");
        // Previous settings schema has no profiles field.
        fs::write(&path, br#"{"connections":{}}"#).unwrap();
        let mut store = Settings::load(path.clone());
        assert!(store.error.is_none());
        let manifest =
            crate::plugins::parse(include_bytes!("../plugins-examples/openai-compatible.json"))
                .unwrap();
        let tool = manifest.tools.iter().find(|t| matches!(&t.adapter, crate::plugins::Adapter::Http { method, .. } if method == "POST")).unwrap();
        let original = Profile {
            name: " Local Model ".into(),
            shape: request_shape(tool).unwrap(),
            connection: Connection {
                endpoint: "http://localhost:4321/v1/chat/completions".into(),
                model: "fixture-model".into(),
            },
        };
        let id = store.save_profile(None, original.clone()).unwrap();
        let mut duplicate = original.clone();
        duplicate.name = "local model".into();
        assert!(store.save_profile(None, duplicate).is_err());
        let mut other_tool = tool.clone();
        other_tool.id = "other-chat".into();
        if let crate::plugins::Adapter::Http { url, .. } = &mut other_tool.adapter {
            *url = "https://example.com/other".into();
        }
        let copied = store.profile(&id).unwrap().for_tool(&other_tool).unwrap();
        let key = Settings::key("plugin:other/chat", &other_tool);
        store.set(&key, Some(copied.clone())).unwrap();
        let mut changed = original;
        changed.connection.model = "new-model".into();
        store.save_profile(Some(&id), changed).unwrap();
        let mut reloaded = Settings::load(path.clone());
        assert!(reloaded.error.is_none());
        assert_eq!(reloaded.profile(&id).unwrap().connection.model, "new-model");
        assert_eq!(reloaded.get(&key).unwrap().model, "fixture-model");
        if let crate::plugins::Adapter::Http { body, .. } = &mut other_tool.adapter {
            *body = serde_json::json!({"prompt": "$input"});
        }
        assert!(
            reloaded
                .profile(&id)
                .unwrap()
                .for_tool(&other_tool)
                .is_err()
        );
        reloaded.delete_profile(&id).unwrap();
        let final_state = Settings::load(path.clone());
        assert!(final_state.profile(&id).is_none());
        assert!(final_state.get(&key) == Some(&copied));
        // Invalid replacement cannot destroy the existing snapshot document.
        let before = fs::read(&path).unwrap();
        let bad = Profile {
            name: "bad".into(),
            shape: "bad".into(),
            connection: copied,
        };
        assert!(reloaded.save_profile(None, bad).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn persisted_connections_replace_reset_and_preserve_invalid_file() {
        let root = std::env::temp_dir().join(format!("zi-settings-{}", uuid::Uuid::new_v4()));
        let path = root.join("connections.json");
        let key = "a".repeat(64);
        let mut store = Settings::load(path.clone());
        let mut value = Connection {
            endpoint: "http://127.0.0.1:1234/v1/chat/completions".into(),
            model: "model-one".into(),
        };
        store.set(&key, Some(value.clone())).unwrap();
        value.model = "model-two".into();
        store.set(&key, Some(value.clone())).unwrap();
        assert!(Settings::load(path.clone()).get(&key) == Some(&value));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        value.endpoint = "https://example.com/?token=fixture".into();
        assert!(store.set(&key, Some(value)).is_err());
        assert_eq!(store.get(&key).unwrap().model, "model-two");
        store.set(&key, None).unwrap();
        assert!(Settings::load(path.clone()).get(&key).is_none());
        fs::write(&path, b"invalid fixture").unwrap();
        let mut invalid = Settings::load(path.clone());
        assert!(invalid.error.is_some());
        assert!(invalid.set(&key, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid fixture");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_tool_definition_does_not_inherit_connection() {
        let manifest =
            crate::plugins::parse(include_bytes!("../plugins-examples/openai-compatible.json"))
                .unwrap();
        let mut tool = manifest.tools[0].clone();
        let key = Settings::key("plugin:sample/chat", &tool);
        assert_ne!(key, Settings::key("plugin:other/chat", &tool));
        tool.model = "different-default".into();
        assert_ne!(key, Settings::key("plugin:sample/chat", &tool));
    }
}
