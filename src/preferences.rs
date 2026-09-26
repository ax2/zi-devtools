use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub light: bool,
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
    pub usage: std::collections::BTreeMap<String, u32>,
}
pub fn path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/ui-preferences.json")
}
impl Preferences {
    pub fn visit(&mut self, id: &str) {
        self.recent.retain(|s| s != id);
        self.recent.insert(0, id.into());
        self.recent.truncate(20);
        if self.usage.len() < 4096 || self.usage.contains_key(id) {
            let count = self.usage.entry(id.into()).or_default();
            *count = count.saturating_add(1);
        }
    }
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?).context("无法保存界面偏好")
    }
    pub fn toggle(&mut self, id: &str) {
        if self.favorites.iter().any(|s| s == id) {
            self.favorites.retain(|s| s != id);
        } else {
            self.favorites.push(id.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_preferences_migrate_and_usage_stays_bounded() {
        let mut p: Preferences =
            serde_json::from_str(r#"{"light":true,"favorites":["json"]}"#).unwrap();
        assert!(p.recent.is_empty());
        for i in 0..30 {
            p.visit(&format!("tool-{i}"));
        }
        p.visit("tool-1");
        assert_eq!(p.recent.len(), 20);
        assert_eq!(p.recent[0], "tool-1");
        assert_eq!(p.usage["tool-1"], 2);
        assert_eq!(p.favorites, vec!["json"]);
    }
}
