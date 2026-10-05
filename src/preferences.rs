use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub updates: crate::updates::Policy,
    pub command_bindings: crate::commands::Bindings,
    pub light: bool,
    pub hotkey: crate::hotkey::Setting,
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
    pub usage: std::collections::BTreeMap<String, u32>,
    pub recorder_auto_minimize: bool,
    pub recorder_auto_stop_minutes: u16,
    pub recorder_quality: crate::recorder::RecordingQuality,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            updates: Default::default(),
            command_bindings: Default::default(),
            light: false,
            hotkey: Default::default(),
            favorites: Vec::new(),
            recent: Vec::new(),
            usage: Default::default(),
            recorder_auto_minimize: true,
            recorder_auto_stop_minutes: 0,
            recorder_quality: crate::recorder::RecordingQuality::default(),
        }
    }
}
pub fn path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zi-devtools/ui-preferences.json")
}
impl Preferences {
    /// Save before activating, preserving the live state on failure.
    pub fn save_command_bindings(
        &mut self,
        path: &Path,
        bindings: crate::commands::Bindings,
    ) -> Result<()> {
        let mut next = self.clone();
        next.command_bindings = bindings;
        next.save(path)?;
        *self = next;
        Ok(())
    }
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
        let mut bytes = Vec::new();
        let read =
            fs::File::open(path).and_then(|file| file.take(8_388_609).read_to_end(&mut bytes));
        let mut value: Self = if read.is_ok() && bytes.len() <= 8_388_608 {
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            Self::default()
        };
        let normalize = |ids: &mut Vec<String>, limit: usize| {
            let mut seen = std::collections::HashSet::new();
            ids.retain(|id| !id.is_empty() && id.len() <= 512 && seen.insert(id.clone()));
            ids.truncate(limit);
        };
        normalize(&mut value.favorites, 4096);
        normalize(&mut value.recent, 20);
        value.usage = value
            .usage
            .into_iter()
            .filter(|(id, count)| !id.is_empty() && id.len() <= 512 && *count > 0)
            .take(4096)
            .collect();
        if ![0, 1, 5, 15, 30, 60].contains(&value.recorder_auto_stop_minutes) {
            value.recorder_auto_stop_minutes = 0;
        }
        value
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            // Same-directory rename replaces the previous file without first deleting it.
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.context("无法保存界面偏好")
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
    fn binding_save_is_transactional_and_preserves_other_preferences() {
        let dir = std::env::temp_dir().join(format!("zi-bindings-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let mut prefs = Preferences::default();
        prefs.visit("json");
        let mut bindings = crate::commands::Bindings::new();
        bindings.insert("open:json".into(), "Q J".into());
        bindings.insert("open:disabled-plugin".into(), String::new());
        assert!(prefs.save_command_bindings(&dir, bindings.clone()).is_err());
        assert!(prefs.command_bindings.is_empty());
        assert_eq!(prefs.recent, ["json"]);
        let path = dir.join("preferences.json");
        prefs
            .save_command_bindings(&path, bindings.clone())
            .unwrap();
        assert_eq!(Preferences::load(&path).command_bindings, bindings);
        assert_eq!(Preferences::load(&path).recent, ["json"]);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn saves_replace_existing_preferences_and_load_deduplicates() {
        let dir = std::env::temp_dir().join(format!("zi-preferences-{}", uuid::Uuid::new_v4()));
        let path = dir.join("preferences.json");
        let mut prefs = Preferences::default();
        prefs.toggle("plugin:disabled/tool");
        prefs.save(&path).unwrap();
        prefs.visit("json");
        prefs.recorder_auto_minimize = false;
        prefs.recorder_auto_stop_minutes = 15;
        prefs.recorder_quality = crate::recorder::RecordingQuality::Detailed;
        prefs.favorites.push("plugin:disabled/tool".into());
        prefs.save(&path).unwrap();
        let restored = Preferences::load(&path);
        assert_eq!(restored.favorites, ["plugin:disabled/tool"]);
        assert_eq!(restored.recent, ["json"]);
        assert_eq!(restored.usage["json"], 1);
        assert!(!restored.recorder_auto_minimize);
        assert_eq!(restored.recorder_auto_stop_minutes, 15);
        assert_eq!(
            restored.recorder_quality,
            crate::recorder::RecordingQuality::Detailed
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::write(&path, "invalid").unwrap();
        assert!(Preferences::load(&path).recent.is_empty());
        fs::write(&path, r#"{"recorder_auto_stop_minutes":999}"#).unwrap();
        assert_eq!(Preferences::load(&path).recorder_auto_stop_minutes, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_preferences_migrate_and_usage_stays_bounded() {
        let mut p: Preferences =
            serde_json::from_str(r#"{"light":true,"favorites":["json"]}"#).unwrap();
        assert!(p.command_bindings.is_empty());
        assert!(p.recent.is_empty());
        assert!(p.recorder_auto_minimize);
        assert_eq!(p.recorder_auto_stop_minutes, 0);
        assert_eq!(
            p.recorder_quality,
            crate::recorder::RecordingQuality::Balanced
        );
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
