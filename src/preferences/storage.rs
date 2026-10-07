//! Merge this window's changes while a Windows exclusive file handle is held.
use super::Preferences;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

fn read(path: &Path) -> Result<Option<Value>> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    file.take(8_388_609).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 8_388_608, "偏好文件超过 8 MiB，未覆盖原文件");
    let value: Value = serde_json::from_slice(&bytes).context("现有偏好文件损坏，未覆盖原文件")?;
    ensure!(value.is_object(), "现有偏好文件格式错误，未覆盖原文件");
    let _: Preferences =
        serde_json::from_value(value.clone()).context("现有偏好字段无效，未覆盖原文件")?;
    Ok(Some(value))
}

fn merge_fields(latest: &mut Value, base: &Value, current: &Value, root: bool) {
    if base == current {
        return;
    }
    if let (Some(old), Some(new), Some(target)) = (
        base.as_object(),
        current.as_object(),
        latest.as_object_mut(),
    ) {
        for key in old
            .keys()
            .chain(new.keys().filter(|key| !old.contains_key(*key)))
        {
            if root
                && matches!(
                    key.as_str(),
                    "favorites" | "recent" | "usage" | "workflow_favorites" | "workflow_recent"
                )
            {
                continue;
            }
            match (old.get(key), new.get(key)) {
                (Some(a), Some(b)) if a != b => {
                    if let Some(value) = target.get_mut(key) {
                        merge_fields(value, a, b, false);
                    } else {
                        target.insert(key.clone(), b.clone());
                    }
                }
                (None, Some(b)) => {
                    target.insert(key.clone(), b.clone());
                }
                (Some(_), None) => {
                    target.remove(key);
                }
                _ => {}
            }
        }
    } else {
        *latest = current.clone();
    }
}

fn lock(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED);
    }
    let file = options.open(path.with_extension("json.lock"))?;
    #[cfg(windows)]
    native_lock(&file)?;
    Ok(file)
}

#[cfg(windows)]
fn native_lock(file: &fs::File) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_IO_PENDING, HANDLE},
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LockFileEx},
        System::{
            IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
            Threading::CreateEventW,
        },
    };
    struct Event(HANDLE);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    unsafe {
        let event = Event(CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()));
        ensure!(!event.0.is_null(), "无法创建偏好锁事件");
        let mut overlapped: OVERLAPPED = std::mem::zeroed();
        overlapped.hEvent = event.0;
        let handle = file.as_raw_handle();
        if LockFileEx(handle, LOCKFILE_EXCLUSIVE_LOCK, 0, 1, 0, &mut overlapped) != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error.into());
        }
        let mut transferred = 0;
        if GetOverlappedResultEx(handle, &overlapped, &mut transferred, 500, 0) != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // Cancel only this pending request and drain completion before the event
        // or stack OVERLAPPED is released. Closing File releases any won race.
        CancelIoEx(handle, &overlapped);
        GetOverlappedResult(handle, &overlapped, &mut transferred, 1);
        Err(error).context("其他窗口正在保存界面偏好或文件锁不可用，请稍后重试")
    }
}

pub(super) fn refresh_discovery(prefs: &mut Preferences, path: &Path) -> Result<bool> {
    let mut base = prefs
        .baseline
        .clone()
        .unwrap_or(serde_json::to_value(Preferences::default())?);
    let old: Preferences = serde_json::from_value(base.clone())?;
    if !prefs.pending_recent.is_empty()
        || prefs.workflow_history_pending()
        || prefs.favorites != old.favorites
        || prefs.usage != old.usage
        || prefs.workflow_favorites != old.workflow_favorites
    {
        return Ok(false);
    }
    let Some(document) = read(path)? else {
        return Ok(false);
    };
    let mut latest: Preferences = serde_json::from_value(document)?;
    latest.normalize();
    let changed = prefs.favorites != latest.favorites
        || prefs.recent != latest.recent
        || prefs.usage != latest.usage
        || prefs.workflow_favorites != latest.workflow_favorites
        || prefs.workflow_recent != latest.workflow_recent;
    if changed {
        prefs.favorites = latest.favorites;
        prefs.recent = latest.recent;
        prefs.usage = latest.usage;
        prefs.workflow_favorites = latest.workflow_favorites;
        prefs.workflow_recent = latest.workflow_recent;
        let current = serde_json::to_value(&*prefs)?;
        for key in [
            "favorites",
            "recent",
            "usage",
            "workflow_favorites",
            "workflow_recent",
        ] {
            base[key] = current[key].clone();
        }
        prefs.baseline = Some(base);
    }
    Ok(changed)
}

pub(super) fn save(prefs: &mut Preferences, path: &Path) -> Result<()> {
    ensure!(!path.is_dir(), "偏好目标是目录");
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // The supported product is Windows. The fallback serializes in-process tests
    // on other hosts; it does not advertise cross-process locking there.
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _gate = GATE
        .lock()
        .map_err(|_| anyhow::anyhow!("偏好保存锁不可用"))?;
    let _lock = lock(path)?;
    let current = serde_json::to_value(&*prefs)?;
    let base = prefs
        .baseline
        .clone()
        .unwrap_or(serde_json::to_value(Preferences::default())?);
    let mut document = read(path)?.unwrap_or(serde_json::to_value(Preferences::default())?);
    merge_fields(&mut document, &base, &current, true);
    let mut next: Preferences = serde_json::from_value(document.clone())?;
    let old: Preferences = serde_json::from_value(base)?;
    for entry in &old.workflow_favorites {
        if !prefs
            .workflow_favorites
            .iter()
            .any(|item| item.path == entry.path)
        {
            next.workflow_favorites
                .retain(|item| item.path != entry.path);
        }
    }
    for entry in &prefs.workflow_favorites {
        if !old.workflow_favorites.iter().any(|item| item == entry) {
            if let Some(existing) = next
                .workflow_favorites
                .iter_mut()
                .find(|item| item.path == entry.path)
            {
                *existing = entry.clone();
            } else {
                ensure!(
                    next.workflow_favorites.len() < 100,
                    "其他窗口已收藏100条流程，请刷新后移除不再需要的收藏"
                );
                next.workflow_favorites.push(entry.clone());
            }
        }
    }
    for id in &old.favorites {
        if !prefs.favorites.contains(id) {
            next.favorites.retain(|item| item != id);
        }
    }
    for id in &prefs.favorites {
        if !old.favorites.contains(id) && !next.favorites.contains(id) {
            next.favorites.push(id.clone());
        }
    }
    for (id, count) in &prefs.usage {
        let delta = count.saturating_sub(old.usage.get(id).copied().unwrap_or_default());
        if delta > 0 && (next.usage.len() < 4096 || next.usage.contains_key(id)) {
            let value = next.usage.entry(id.clone()).or_default();
            *value = value.saturating_add(delta);
        }
    }
    if !prefs.pending_recent.is_empty() {
        let mut recent = prefs.pending_recent.clone();
        recent.extend(
            next.recent
                .into_iter()
                .filter(|id| !prefs.pending_recent.contains(id)),
        );
        next.recent = recent;
    }
    next.workflow_recent
        .retain(|entry| !prefs.removed_workflow_loads.contains(&entry.path));
    if !prefs.pending_workflow_loads.is_empty() {
        let mut recent = prefs.pending_workflow_loads.clone();
        recent.extend(next.workflow_recent.into_iter().filter(|entry| {
            !prefs
                .pending_workflow_loads
                .iter()
                .any(|pending| pending.path == entry.path)
        }));
        next.workflow_recent = recent;
    }
    next.normalize();
    let clean = serde_json::to_value(&next)?;
    for key in [
        "favorites",
        "recent",
        "usage",
        "workflow_favorites",
        "workflow_recent",
    ] {
        document[key] = clean[key].clone();
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&document)?;
        ensure!(
            bytes.len() <= 8_388_608,
            "保存后的偏好超过 8 MiB，未覆盖原文件"
        );
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    next.baseline = Some(clean);
    *prefs = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("zi-shared-prefs-{}", uuid::Uuid::new_v4())))
        }
        fn path(&self) -> std::path::PathBuf {
            self.0.join("preferences.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn stale_windows_merge_visits_favorites_and_independent_settings() {
        let fixture = Fixture::new();
        let path = fixture.path();
        let mut a = Preferences::load(&path);
        let mut b = Preferences::load(&path);
        a.visit("json");
        a.toggle("json");
        a.light = true;
        a.save(&path).unwrap();
        b.visit("base64");
        b.toggle("base64");
        b.recorder_auto_stop_minutes = 15;
        b.save(&path).unwrap();
        a.visit("json");
        a.save(&path).unwrap();
        let merged = Preferences::load(&path);
        assert_eq!(merged.usage["json"], 2);
        assert_eq!(merged.usage["base64"], 1);
        assert_eq!(merged.recent, ["json", "base64"]);
        assert_eq!(merged.favorites, ["json", "base64"]);
        assert!(merged.light);
        assert_eq!(merged.recorder_auto_stop_minutes, 15);
        a.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).usage, merged.usage);
        b.toggle("base64");
        b.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).favorites, ["json"]);
    }

    #[test]
    fn same_tool_visits_accumulate_and_binding_maps_merge_by_key() {
        let fixture = Fixture::new();
        let path = fixture.path();
        let mut a = Preferences::load(&path);
        let mut b = Preferences::load(&path);
        a.visit("json");
        b.visit("json");
        a.command_bindings.insert("open:json".into(), "Q J".into());
        b.command_bindings
            .insert("open:base64".into(), "Q B".into());
        a.save(&path).unwrap();
        b.save(&path).unwrap();
        let merged = Preferences::load(&path);
        assert_eq!(merged.usage["json"], 2);
        assert_eq!(merged.recent, ["json"]);
        assert_eq!(merged.command_bindings.len(), 2);
    }

    #[test]
    fn damaged_latest_file_is_preserved_and_pending_visit_can_retry() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::create_dir_all(&fixture.0).unwrap();
        fs::write(&path, br#"{"future_field":{"enabled":true},"light":true}"#).unwrap();
        let mut prefs = Preferences::load(&path);
        prefs.visit("json");
        fs::write(&path, "damaged").unwrap();
        assert!(prefs.save(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "damaged");
        assert_eq!(prefs.usage["json"], 1);
        fs::write(&path, br#"{"future_field":{"enabled":true},"light":true}"#).unwrap();
        prefs.save(&path).unwrap();
        prefs.save(&path).unwrap();
        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["future_field"]["enabled"], true);
        assert_eq!(document["usage"]["json"], 1);
        assert_eq!(document["recent"][0], "json");
    }

    #[test]
    fn discovery_refresh_keeps_local_settings_and_unsaved_actions() {
        let fixture = Fixture::new();
        let path = fixture.path();
        let mut local = Preferences::load(&path);
        let mut remote = Preferences::load(&path);
        local.recorder_auto_stop_minutes = 15;
        remote.light = true;
        remote.visit("base64");
        remote.toggle("base64");
        remote.save(&path).unwrap();
        assert!(local.refresh_discovery(&path).unwrap());
        assert_eq!(local.recent, ["base64"]);
        assert_eq!(local.favorites, ["base64"]);
        assert!(!local.light);
        assert_eq!(local.recorder_auto_stop_minutes, 15);
        local.visit("json");
        remote.visit("uuid");
        remote.save(&path).unwrap();
        assert!(!local.refresh_discovery(&path).unwrap());
        assert_eq!(local.recent[0], "json");
        local.save(&path).unwrap();
        let merged = Preferences::load(&path);
        assert_eq!(merged.recent, ["json", "uuid", "base64"]);
        assert!(merged.light);
        assert_eq!(merged.recorder_auto_stop_minutes, 15);
        assert_eq!(merged.usage["base64"], 1);
    }

    #[cfg(windows)]
    #[test]
    fn queued_lock_timeout_preserves_pending_changes_and_releases_request() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::create_dir_all(&fixture.0).unwrap();
        let held = lock(&path).unwrap();
        let mut prefs = Preferences::load(&path);
        prefs.visit("json");
        assert!(prefs.save(&path).is_err());
        assert!(!path.exists());
        assert_eq!(prefs.usage["json"], 1);
        drop(held);
        prefs.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).usage["json"], 1);
        assert_eq!(
            fs::metadata(path.with_extension("json.lock"))
                .unwrap()
                .len(),
            0
        );
    }
}
