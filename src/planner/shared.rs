//! Read-only discovery and a lifetime Windows lease for automatic reminders.
use super::*;
use anyhow::Context;
use std::{fs::File, path::Path, time::SystemTime};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp(Vec<Option<(u64, SystemTime)>>);
fn stamp(path: &Path) -> Result<Stamp> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    let mut parts = Vec::new();
    for part in [path.to_path_buf(), PathBuf::from(wal)] {
        match std::fs::symlink_metadata(part) {
            Ok(meta) => {
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "日程同步目标不是普通文件"
                );
                parts.push(Some((meta.len(), meta.modified()?)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => parts.push(None),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Stamp(parts))
}

pub(super) fn lease(path: &Path) -> Result<Option<File>> {
    ensure!(cfg!(windows), "此平台尚不支持多窗口自动提醒协调");
    let absolute = std::path::absolute(path)?;
    let parent = absolute.parent().context("日程路径没有父目录")?;
    std::fs::create_dir_all(parent)?;
    let mut name = std::fs::canonicalize(parent)?
        .join(absolute.file_name().context("日程文件名为空")?)
        .into_os_string();
    name.push(".notify.lock");
    let name = PathBuf::from(name);
    if name.exists() {
        let meta = std::fs::symlink_metadata(&name)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "提醒协调文件不是普通文件"
        );
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Allow fixture/directory cleanup, but deny competing readers/writers.
        options.share_mode(4);
    }
    match options.open(name) {
        Ok(file) => Ok(Some(file)),
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::tests::{event, fixture, wait_state};
    use super::*;

    fn refresh(state: &mut State) {
        state.shared.checked -= std::time::Duration::from_secs(3);
        state.last_tick -= std::time::Duration::from_secs(2);
        wait_state(state);
    }

    #[test]
    fn lease_process_fixture() {
        let Some(path) = std::env::var_os("ZI_PLANNER_LEASE_FIXTURE") else {
            return;
        };
        let path = PathBuf::from(path);
        let _owner = lease(&path).unwrap().unwrap();
        std::fs::write(path.with_extension("ready"), b"ready").unwrap();
        // The parent deliberately terminates this disposable child to check crash release.
        std::thread::sleep(std::time::Duration::from_secs(15));
    }

    #[test]
    fn kernel_lease_excludes_real_process_and_releases_after_termination() {
        use std::os::windows::process::CommandExt;
        let path = fixture();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "planner::shared::tests::lease_process_fixture"])
            .env("ZI_PLANNER_LEASE_FIXTURE", &path)
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut child = Child(child);
        let started = Instant::now();
        while !path.with_extension("ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "lease fixture exited early"
            );
            assert!(
                started.elapsed().as_secs() < 5,
                "lease fixture did not become ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(lease(&path).unwrap().is_none());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(lease(&path).unwrap().is_some());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn one_owner_follower_discovers_external_event_and_takes_over() {
        let path = fixture();
        let mut first = State::new(path.clone());
        wait_state(&mut first);
        let mut second = State::new(path.clone());
        wait_state(&mut second);
        assert!(first.shared.owner.is_some());
        assert!(second.shared.owner.is_none());
        let mut item = event();
        let schedule = item.schedule.as_mut().unwrap();
        schedule.start = Local::now().naive_local() - Duration::minutes(1);
        schedule.minutes = 0;
        store::save(&path, item).unwrap();
        refresh(&mut first);
        refresh(&mut second);
        assert!(first.alarm_open);
        assert!(!second.alarm_open);
        assert_eq!(second.alarms.len(), 1);
        assert_eq!(first.items, second.items);
        drop(first);
        refresh(&mut second);
        assert!(second.shared.owner.is_some());
        assert!(second.alarm_open);
        let at = second.alarms[0].1;
        let id = second.alarms[0].0.clone();
        second
            .respond_reminder(
                &id,
                at,
                reminder_actions::ReminderAction::Acknowledge,
                Local::now(),
            )
            .unwrap();
        wait_state(&mut second);
        drop(second);
        let mut third = State::new(path.clone());
        wait_state(&mut third);
        assert!(!third.alarm_open);
        assert!(third.alarms.is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn external_updates_preserve_raw_draft_and_revision_conflict() {
        let path = fixture();
        store::save(&path, event()).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        state.edit(state.items[0].clone());
        let revision = state.draft.as_ref().unwrap().revision;
        state.repeat_until_text = "2028-12-31".into();
        assert!(state.has_unsaved());
        let mut remote = state.items[0].clone();
        remote.title = "另一窗口的修改".into();
        store::save(&path, remote).unwrap();
        refresh(&mut state);
        assert_eq!(state.items[0].title, "另一窗口的修改");
        assert_eq!(state.draft.as_ref().unwrap().revision, revision);
        assert_eq!(state.repeat_until_text, "2028-12-31");
        assert!(state.shared_conflict());
        assert!(store::save(&path, state.draft.clone().unwrap()).is_err());
        state.draft = None;
        state.original = None;
        state.edit(state.items[0].clone());
        let focus = state.focus_editor;
        let mut remote = state.items[0].clone();
        remote.body = "自动刷新正文".into();
        store::save(&path, remote).unwrap();
        refresh(&mut state);
        assert_eq!(state.draft.as_ref().unwrap().body, "自动刷新正文");
        assert_eq!(state.focus_editor, focus);
        assert!(!state.has_unsaved());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn stale_async_result_and_corrupt_database_never_replace_current_items() {
        let path = fixture();
        store::save(&path, event()).unwrap();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        let current = state.items.clone();
        let (tx, rx) = mpsc::channel();
        state.shared.pending = Some((state.shared.epoch, rx));
        tx.send(Ok(Some((stamp(&path).unwrap(), Vec::new()))))
            .unwrap();
        state.shared.invalidate();
        state.poll_shared();
        assert_eq!(state.items, current);
        std::fs::write(&path, b"broken sqlite fixture").unwrap();
        refresh(&mut state);
        assert_eq!(state.items, current);
        assert!(!state.shared.notice.is_empty());
        assert!(!state.owns_current_reminders());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn full_database_filename_and_canonical_parent_define_lease() {
        let path = fixture();
        let first = lease(&path).unwrap().unwrap();
        assert!(lease(&path).unwrap().is_none());
        let alias = path.parent().unwrap().join(".").join("planner.sqlite3");
        assert!(lease(&alias).unwrap().is_none());
        let distinct = path.with_extension("db");
        assert!(lease(&distinct).unwrap().is_some());
        drop(first);
        assert!(lease(&path).unwrap().is_some());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

type Snapshot = std::result::Result<Option<(Stamp, Vec<Item>)>, String>;
pub(super) struct Shared {
    owner: Option<File>,
    observed: Option<Stamp>,
    pending: Option<(u64, mpsc::Receiver<Snapshot>)>,
    epoch: u64,
    checked: Instant,
    pub(super) notice: String,
    conflict: bool,
    #[cfg(feature = "ui-preview")]
    pub(super) synthetic: bool,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            owner: None,
            observed: None,
            pending: None,
            epoch: 0,
            checked: Instant::now() - std::time::Duration::from_secs(3),
            notice: String::new(),
            conflict: false,
            #[cfg(feature = "ui-preview")]
            synthetic: false,
        }
    }
}
impl Shared {
    #[cfg(test)]
    pub(super) fn expedite(&mut self) {
        self.checked -= std::time::Duration::from_secs(3);
    }
    #[cfg(test)]
    pub(super) fn refreshing(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.observed = None;
    }
    fn synthetic(&self) -> bool {
        #[cfg(feature = "ui-preview")]
        {
            self.synthetic
        }
        #[cfg(not(feature = "ui-preview"))]
        {
            false
        }
    }
    pub(super) fn status(&self) -> &str {
        if self.synthetic() {
            "预览数据 · 自动提醒示例"
        } else if self.owner.is_some() {
            "自动同步 · 此窗口负责到期提醒"
        } else {
            "自动同步 · 到期提醒由其他窗口负责，关闭后自动接管"
        }
    }
}
impl State {
    fn apply_shared(&mut self, items: Vec<Item>) {
        let dirty = self.has_unsaved();
        let latest = self
            .draft
            .as_ref()
            .and_then(|draft| items.iter().find(|i| i.id == draft.id))
            .cloned();
        self.shared.conflict = dirty
            && self
                .original
                .as_ref()
                .is_some_and(|original| original.revision > 0 && latest.as_ref() != Some(original));
        if !dirty && self.draft.as_ref().is_some_and(|i| i.revision > 0) {
            if let Some(item) = latest {
                let focus = self.focus_editor;
                self.edit(item);
                self.focus_editor = focus;
            } else {
                self.draft = None;
                self.original = None;
            }
        }
        self.replace_items(items);
    }
    pub(super) fn poll_shared(&mut self) {
        if self.shared.synthetic() {
            return;
        }
        let received = self
            .shared
            .pending
            .as_ref()
            .and_then(|(epoch, rx)| match rx.try_recv() {
                Ok(result) => Some((*epoch, result)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some((*epoch, Err("日程同步线程中断；将自动重试".into())))
                }
            });
        if let Some((epoch, result)) = received {
            self.shared.pending = None;
            if self.pending.is_none() && epoch == self.shared.epoch {
                match result {
                    Ok(Some((observed, items))) => {
                        self.apply_shared(items);
                        self.shared.observed = Some(observed);
                        self.shared.notice.clear();
                    }
                    Ok(None) => self.shared.observed = None,
                    Err(error) => {
                        self.shared.notice = format!("自动同步失败，现有内容保留：{error}");
                        self.shared.observed = None;
                    }
                }
            }
        }
        if self.shared.checked.elapsed().as_secs() < 2 {
            return;
        }
        self.shared.checked = Instant::now();
        if self.shared.owner.is_none() {
            match lease(&self.path) {
                Ok(Some(owner)) => {
                    self.shared.owner = Some(owner);
                    self.shared.observed = None;
                }
                Ok(None) => (),
                Err(error) => {
                    self.shared.notice = format!("自动提醒暂不可用：{error}");
                    return;
                }
            }
        }
        if self.pending.is_some() || self.shared.pending.is_some() || !self.loaded {
            return;
        }
        match stamp(&self.path) {
            Ok(current) if self.shared.observed.as_ref() == Some(&current) => (),
            Ok(_) => {
                let path = self.path.clone();
                let (tx, rx) = mpsc::channel();
                self.shared.pending = Some((self.shared.epoch, rx));
                std::thread::spawn(move || {
                    let result = (|| -> Result<_> {
                        let before = stamp(&path)?;
                        let items = store::load(&path)?;
                        let after = stamp(&path)?;
                        Ok((before == after).then_some((after, items)))
                    })()
                    .map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });
            }
            Err(error) => {
                self.shared.notice = format!("自动同步失败，现有内容保留：{error}");
                self.shared.observed = None;
            }
        }
    }
    pub(super) fn owns_current_reminders(&self) -> bool {
        self.shared.synthetic()
            || (self.shared.owner.is_some()
                && self.pending.is_none()
                && self.shared.pending.is_none()
                && self.shared.observed.is_some()
                && stamp(&self.path).ok().as_ref() == self.shared.observed.as_ref())
    }
    pub(super) fn shared_conflict(&self) -> bool {
        self.shared.conflict && self.has_unsaved()
    }
}
