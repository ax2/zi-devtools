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
}
pub fn path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/ui-preferences.json")
}
impl Preferences {
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
