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
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    connections: BTreeMap<String, Connection>,
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
    pub fn set(&mut self, key: &str, value: Option<Connection>) -> Result<()> {
        ensure!(self.error.is_none(), "设置文件异常，未覆盖原文件");
        ensure!(
            key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit()),
            "设置标识无效"
        );
        if let Some(v) = &value {
            v.validate()?;
        }
        let mut next = Document {
            connections: self.data.connections.clone(),
        };
        if let Some(value) = value {
            next.connections.insert(key.into(), value);
        } else {
            next.connections.remove(key);
        }
        ensure!(next.connections.len() <= 512, "最多保存 512 个工具连接设置");
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
