//! Opt-in encrypted local snapshots. Clipboard text never reaches SQLite in plaintext.
use super::{BYTE_LIMIT, History, ITEM_LIMIT, TEXT_LIMIT};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const APP_ID: i64 = 0x5A434C50;
const ENCRYPTED_LIMIT: usize = 40 * 1024 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    history: History,
}
fn validate(history: &History) -> Result<()> {
    ensure!(
        history.entries.len() <= ITEM_LIMIT && history.bytes() <= BYTE_LIMIT,
        "剪贴板历史超出容量"
    );
    ensure!(
        history
            .retention_days
            .is_none_or(|days| (1..=3650).contains(&days)),
        "历史保留期无效"
    );
    let mut ids = std::collections::HashSet::new();
    let mut texts = std::collections::HashSet::new();
    let mut images = std::collections::HashSet::new();
    for e in &history.entries {
        ensure!(
            e.first_captured_utc
                .into_iter()
                .chain(e.last_captured_utc)
                .all(|time| { chrono::DateTime::from_timestamp(time, 0).is_some() }),
            "历史UTC时间无效"
        );
        ensure!(
            e.first_captured_utc
                .is_none_or(|first| e.last_captured_utc.is_some_and(|last| first <= last)),
            "首次与最近时间无效"
        );
        ensure!(
            e.id > 0 && e.id <= history.next && ids.insert(e.id),
            "剪贴板历史ID无效"
        );
        if let Some(image) = &e.image {
            ensure!(
                e.text.is_empty() && images.insert(&image.sha256),
                "图片混入文本或重复图片"
            );
            image.validate()?;
        } else {
            ensure!(
                !e.text.is_empty()
                    && e.text.len() <= TEXT_LIMIT
                    && !e.text.contains('\0')
                    && texts.insert(&e.text),
                "剪贴板历史文本无效"
            );
        }
        ensure!(
            e.source.len() <= 2048 && e.time.len() <= 64,
            "剪贴板历史元信息过大"
        );
    }
    Ok(())
}
fn protect(input: &[u8], decrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    ensure!(input.len() <= ENCRYPTED_LIMIT, "保护数据超过上限");
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let domain = b"ZiDevTools/Clipboard/Snapshot/1";
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: domain.len() as u32,
        pbData: domain.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &source,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &source,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    ensure!(ok != 0, "Windows用户范围保护失败，原内容保持不变");
    let result = if output.cbData as usize <= ENCRYPTED_LIMIT {
        Ok(Zeroizing::new(
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec(),
        ))
    } else {
        Err(anyhow::anyhow!("保护后的数据超过上限"))
    };
    unsafe {
        if !output.pbData.is_null() {
            std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
            LocalFree(output.pbData as _);
        }
    }
    result
}
fn connect(path: &Path, write: bool) -> Result<Connection> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.is_file() && meta.file_attributes() & 0x400 == 0 && meta.len() <= 96 * 1024 * 1024,
            "剪贴板历史库不是安全的普通文件或超过上限"
        );
    }
    if write {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(path, flags)?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA trusted_schema=OFF;")?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    if version == 0 && app == 0 && write {
        let count: i64 = db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        ensure!(count == 0, "不是剪贴板历史库，未修改原文件");
        db.execute_batch("PRAGMA auto_vacuum=FULL; BEGIN IMMEDIATE; CREATE TABLE clipboard(slot INTEGER PRIMARY KEY CHECK(slot=1), enabled INTEGER NOT NULL CHECK(enabled IN(0,1)), payload BLOB); PRAGMA user_version=1; PRAGMA application_id=1514359888; COMMIT;")?;
    } else {
        ensure!(
            version == 1 && app == APP_ID,
            "剪贴板历史库格式不支持，未修改原文件"
        );
    }
    Ok(db)
}
fn save(path: &Path, history: History) -> Result<()> {
    validate(&history)?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    serde_json::to_writer(&mut encoder, &Snapshot { schema: 3, history })?;
    let compressed = Zeroizing::new(encoder.finish()?);
    let protected = protect(&compressed, false)?;
    let mut db = connect(path, true)?;
    let tx = db.transaction()?;
    tx.execute("INSERT INTO clipboard VALUES(1,1,?1) ON CONFLICT(slot) DO UPDATE SET enabled=1,payload=excluded.payload",params![protected.as_slice()])?;
    tx.commit()?;
    Ok(())
}
fn load(path: &Path) -> Result<Option<History>> {
    if !path.exists() {
        return Ok(None);
    }
    let db = connect(path, false)?;
    let row: Option<(bool, Option<Vec<u8>>)> = db
        .query_row(
            "SELECT enabled,payload FROM clipboard WHERE slot=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let cipher = match row {
        None | Some((false, None)) => return Ok(None),
        Some((true, Some(cipher))) => cipher,
        _ => anyhow::bail!("历史启用标记和快照不一致"),
    };
    let compressed = protect(&cipher, true)?;
    let decoder = flate2::read::GzDecoder::new(compressed.as_slice()).take(200 * 1024 * 1024);
    let snapshot: Snapshot = serde_json::from_reader(decoder)?;
    ensure!(matches!(snapshot.schema, 1..=3), "剪贴板快照版本不支持");
    if snapshot.schema < 3 {
        ensure!(
            snapshot
                .history
                .entries
                .iter()
                .all(|entry| entry.image.is_none()),
            "旧快照不支持图片"
        );
    }
    if snapshot.schema == 1 {
        ensure!(
            snapshot.history.retention_days.is_none()
                && snapshot.history.entries.iter().all(|entry| {
                    entry.first_captured_utc.is_none() && entry.last_captured_utc.is_none()
                }),
            "旧快照包含不支持的时间或策略字段"
        );
    }
    validate(&snapshot.history)?;
    Ok(Some(snapshot.history))
}
fn forget(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut db = connect(path, true)?;
    let tx = db.transaction()?;
    tx.execute("INSERT INTO clipboard VALUES(1,0,NULL) ON CONFLICT(slot) DO UPDATE SET enabled=0,payload=NULL",[])?;
    tx.commit()?;
    Ok(())
}

enum Receipt {
    Load(Option<History>),
    Save(u64),
    Forget,
}
pub(super) struct Persistence {
    pub path: Option<PathBuf>,
    pub enabled: bool,
    wanted_save: bool,
    pub error: String,
    loading: bool,
    pub load_failed: bool,
    pub restored_revision: u64,
    last_save: Instant,
    job: Option<crossbeam_channel::Receiver<Result<Receipt>>>,
    startup: bool,
    dirty: u64,
    saved: u64,
    last_change: Instant,
}
impl Default for Persistence {
    fn default() -> Self {
        Self {
            path: if cfg!(any(feature = "ui-preview", test)) {
                None
            } else {
                dirs::data_local_dir().map(|p| p.join("ZiDevTools/clipboard/history.sqlite3"))
            },
            enabled: false,
            wanted_save: false,
            error: String::new(),
            loading: false,
            load_failed: false,
            restored_revision: 0,
            last_save: Instant::now(),
            job: None,
            startup: false,
            dirty: 0,
            saved: 0,
            last_change: Instant::now(),
        }
    }
}
impl Persistence {
    #[cfg(feature = "ui-preview")]
    pub fn preview_enabled(&mut self) {
        self.enabled = true;
        self.wanted_save = true;
        self.saved = self.dirty;
        self.startup = true;
    }
    pub fn restoring(&self) -> bool {
        self.loading
    }
    pub fn reload(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        if let Some(path) = self.path.clone() {
            self.launch(ctx, move || load(&path).map(Receipt::Load));
            self.loading = true;
        }
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn needs_clock(&self) -> bool {
        self.busy() || (self.enabled && self.dirty != self.saved && self.error.is_empty())
    }
    pub fn pending(&self) -> bool {
        self.busy() || ((self.enabled || self.wanted_save) && self.dirty != self.saved)
    }
    pub fn changed(&mut self) {
        self.dirty = self.dirty.saturating_add(1);
        self.last_change = Instant::now();
    }
    fn launch(
        &mut self,
        ctx: &eframe::egui::Context,
        job: impl FnOnce() -> Result<Receipt> + Send + 'static,
    ) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        self.job = Some(rx);
        self.error.clear();
        std::thread::spawn(move || {
            let result=job().map_err(|_|anyhow::anyhow!("本机历史读写或Windows用户保护失败；原历史未替换。请检查权限、文件格式或磁盘空间后重试。"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, ctx: &eframe::egui::Context, history: &mut History) {
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(result) => {
                    self.job = None;
                    match result {
                        Ok(Receipt::Load(Some(restored))) => {
                            *history = restored;
                            self.restored_revision = self.restored_revision.saturating_add(1);
                            self.enabled = true;
                            self.wanted_save = true;
                            self.saved = self.dirty;
                            self.load_failed = false;
                        }
                        Ok(Receipt::Load(None)) => {
                            self.load_failed = false;
                        }
                        Ok(Receipt::Save(revision)) => {
                            self.enabled = true;
                            self.wanted_save = true;
                            self.saved = revision;
                            self.last_save = Instant::now();
                        }
                        Ok(Receipt::Forget) => {
                            self.enabled = false;
                            self.wanted_save = false;
                            self.load_failed = false;
                            self.saved = self.dirty;
                        }
                        Err(e) => {
                            self.error = e.to_string();
                            if self.loading {
                                self.load_failed = true;
                            }
                        }
                    }
                    self.loading = false;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.job = None;
                    if self.loading {
                        self.load_failed = true;
                    }
                    self.loading = false;
                    self.error = "历史后台任务提前结束，尚未保存".into();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if !self.startup && !self.busy() {
            self.startup = true;
            if let Some(path) = self.path.clone().filter(|p| p.exists()) {
                self.launch(ctx, move || load(&path).map(Receipt::Load));
                self.loading = true;
            }
        } else if self.enabled
            && self.dirty != self.saved
            && !self.busy()
            && self.error.is_empty()
            && (self.last_change.elapsed() >= Duration::from_secs(1)
                || self.last_save.elapsed() >= Duration::from_secs(5))
        {
            self.save_now(ctx, history);
        }
    }
    pub fn save_now(&mut self, ctx: &eframe::egui::Context, history: &History) {
        if self.load_failed {
            self.error =
                "此前历史未能读取；请重试读取，或明确删除旧快照后再保存，不能直接覆盖未读内容。"
                    .into();
            return;
        }
        if self.busy() {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.error = "未配置本机历史路径".into();
            return;
        };
        if !self.startup {
            self.startup = true;
            if path.exists() {
                self.reload(ctx);
                return;
            }
        }
        self.wanted_save = true;
        let history = history.clone();
        let revision = self.dirty;
        self.launch(ctx, move || {
            save(&path, history)?;
            Ok(Receipt::Save(revision))
        });
    }
    pub fn forget(&mut self, ctx: &eframe::egui::Context) {
        if self.busy() {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        self.startup = true;
        self.launch(ctx, move || {
            forget(&path)?;
            Ok(Receipt::Forget)
        });
    }
}

#[cfg(feature = "ui-preview")]
pub fn media_benchmark() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "zi-clipboard-media-benchmark-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&root)?;
    let path = root.join("history.sqlite3");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0.join("history.sqlite3"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }
    let _cleanup = Cleanup(root);
    let started = Instant::now();
    let mut history = History::default();
    for i in 0..500 {
        let mut seed = 0x12345678u32 ^ (i + 1);
        let mut image = image::RgbaImage::new(75, 75);
        for pixel in image.pixels_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *pixel = image::Rgba([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]);
        }
        history
            .insert_image(
                std::sync::Arc::new(super::Picture::from_image(
                    image::DynamicImage::ImageRgba8(image),
                )?),
                "fixture.exe".into(),
            )
            .map_err(anyhow::Error::msg)?;
    }
    ensure!(
        history.entries.len() == 500,
        "benchmark must retain 500 distinct images"
    );
    let build_ms = started.elapsed().as_millis();
    let budget = history.bytes();
    let sampled_rss = || -> u64 {
        let pid = sysinfo::get_current_pid().unwrap();
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        system.process(pid).unwrap().memory()
    };
    let rss_built = sampled_rss();
    let expected = history
        .entries
        .iter()
        .map(|e| e.image.as_ref().unwrap().sha256.clone())
        .collect::<Vec<_>>();
    let start = Instant::now();
    save(&path, history)?;
    let save_ms = start.elapsed().as_millis();
    let disk = std::fs::metadata(&path)?.len();
    let rss_saved = sampled_rss();
    let start = Instant::now();
    let restored = load(&path)?.unwrap();
    let restore_ms = start.elapsed().as_millis();
    let rss_restored = sampled_rss();
    ensure!(
        restored
            .entries
            .iter()
            .map(|e| e.image.as_ref().unwrap().sha256.clone())
            .collect::<Vec<_>>()
            == expected,
        "benchmark image hashes changed"
    );
    println!(
        "PASS synthetic 500-image protected snapshot: {}",
        serde_json::json!({"items":500,"pixel_size":[75,75],"charged_bytes":budget,"sqlite_bytes":disk,"build_ms":build_ms,"save_ms":save_ms,"restore_ms":restore_ms,"sampled_rss_bytes":[rss_built,rss_saved,rss_restored],"scope":"synthetic temporary data, sampled boundaries not peak RSS or full application cold startup; no clipboard access"})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_mixed_snapshot_roundtrip_and_legacy_media_rejection() {
        let path = path();
        let mut h = History::default();
        h.insert("synthetic mixed text".into(), "fixture".into())
            .unwrap();
        let mut old = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        serde_json::to_writer(
            &mut old,
            &Snapshot {
                schema: 2,
                history: h.clone(),
            },
        )
        .unwrap();
        save(&path, h.clone()).unwrap();
        let cipher = protect(&old.finish().unwrap(), false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE clipboard SET payload=?1",
            params![cipher.as_slice()],
        )
        .unwrap();
        drop(db);
        let old_restored = load(&path).unwrap().unwrap();
        assert_eq!(
            old_restored.entries[0].first_captured_utc,
            h.entries[0].first_captured_utc
        );
        assert_eq!(old_restored.entries[0].id, h.entries[0].id);
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::ImageRgba8(
                image::RgbaImage::from_fn(4, 3, |x, y| {
                    image::Rgba([x as u8, y as u8, 80, (x * 60) as u8])
                }),
            ))
            .unwrap(),
        );
        h.insert_image(picture.clone(), "fixture.exe".into())
            .unwrap();
        h.entries[0].pinned = true;
        save(&path, h.clone()).unwrap();
        let restored = load(&path).unwrap().unwrap();
        assert_eq!(restored.entries.len(), 2);
        assert!(restored.entries[0].pinned);
        assert_eq!(restored.entries[0].image.as_ref().unwrap().png, picture.png);
        let disk = std::fs::read(&path).unwrap();
        assert!(!disk.windows(8).any(|s| s == b"\x89PNG\r\n\x1a\n"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        serde_json::to_writer(
            &mut encoder,
            &Snapshot {
                schema: 2,
                history: h,
            },
        )
        .unwrap();
        let cipher = protect(&encoder.finish().unwrap(), false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE clipboard SET payload=?1",
            params![cipher.as_slice()],
        )
        .unwrap();
        drop(db);
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn legacy_snapshot_migrates_without_inventing_utc_and_new_policy_roundtrips() {
        let path = path();
        save(&path, History::default()).unwrap();
        let legacy = serde_json::json!({"schema":1,"history":{"next":1,"entries":[{"id":1,"text":"legacy fixture","source":"fixture.exe","time":"01-02 03:04:05","pinned":false}]}});
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        serde_json::to_writer(&mut encoder, &legacy).unwrap();
        let protected = protect(&encoder.finish().unwrap(), false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE clipboard SET payload=?1",
            params![protected.as_slice()],
        )
        .unwrap();
        drop(db);
        let original = std::fs::read(&path).unwrap();
        let mut history = load(&path).unwrap().unwrap();
        assert_eq!(original, std::fs::read(&path).unwrap());
        assert!(history.retention_days.is_none());
        assert!(history.entries[0].first_captured_utc.is_none());
        assert!(history.entries[0].last_captured_utc.is_none());
        history.retention_days = Some(1);
        assert_eq!(history.expire(chrono::Utc::now().timestamp()), 0);
        history
            .insert("new fixture".into(), "fixture.exe".into())
            .unwrap();
        save(&path, history).unwrap();
        let restored = load(&path).unwrap().unwrap();
        assert_eq!(restored.retention_days, Some(1));
        assert_eq!(restored.entries.len(), 2);
        assert!(restored.entries[0].first_captured_utc.is_some());
        assert!(restored.entries[1].last_captured_utc.is_none());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_retention_or_timestamps_do_not_replace_saved_history() {
        let path = path();
        let mut h = History::default();
        h.insert("synthetic".into(), "fixture".into()).unwrap();
        save(&path, h.clone()).unwrap();
        let original = std::fs::read(&path).unwrap();
        h.retention_days = Some(0);
        assert!(save(&path, h.clone()).is_err());
        h.retention_days = None;
        h.entries[0].last_captured_utc = Some(i64::MAX);
        assert!(save(&path, h.clone()).is_err());
        h.entries[0].last_captured_utc = Some(h.entries[0].first_captured_utc.unwrap() - 1);
        assert!(save(&path, h).is_err());
        assert_eq!(original, std::fs::read(&path).unwrap());
        assert_eq!(load(&path).unwrap().unwrap().entries[0].text, "synthetic");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn unknown_schema_and_invalid_ids_are_rejected_without_mutation() {
        let path = path();
        let h = History::default();
        save(&path, h).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        serde_json::to_writer(
            &mut encoder,
            &Snapshot {
                schema: 99,
                history: History::default(),
            },
        )
        .unwrap();
        let bytes = encoder.finish().unwrap();
        let protected = protect(&bytes, false).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE clipboard SET payload=?1",
            params![protected.as_slice()],
        )
        .unwrap();
        drop(db);
        let original = std::fs::read(&path).unwrap();
        assert!(load(&path).is_err());
        let mut bad = History::default();
        bad.insert("synthetic".into(), "fixture".into()).unwrap();
        bad.entries[0].id = 0;
        assert!(save(&path, bad).is_err());
        assert_eq!(original, std::fs::read(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn first_save_failure_remains_pending_and_keeps_memory() {
        let parent = path();
        std::fs::write(&parent, b"fixture parent is a file").unwrap();
        let target = parent.join("history.sqlite3");
        let ctx = eframe::egui::Context::default();
        let mut p = Persistence {
            path: Some(target),
            ..Default::default()
        };
        let mut h = History::default();
        h.insert("not-yet-saved-fixture".into(), "fixture".into())
            .unwrap();
        p.changed();
        p.save_now(&ctx, &h);
        settle(&mut p, &mut h);
        assert!(!p.enabled && p.pending() && !p.error.is_empty());
        assert!(!p.needs_clock());
        assert_eq!(h.entries[0].text, "not-yet-saved-fixture");
        assert_eq!(std::fs::read(&parent).unwrap(), b"fixture parent is a file");
        std::fs::remove_file(parent).unwrap();
    }
    fn settle(p: &mut Persistence, h: &mut History) {
        let ctx = eframe::egui::Context::default();
        let until = Instant::now() + Duration::from_secs(5);
        while p.busy() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
            p.poll(&ctx, h);
        }
        assert!(!p.busy(), "fixture job must finish");
    }
    #[test]
    fn changes_during_save_remain_pending_and_restore_never_enables_capture() {
        let path = path();
        let ctx = eframe::egui::Context::default();
        let mut p = Persistence {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut h = History::default();
        h.insert("first-fixture".into(), "fixture".into()).unwrap();
        p.changed();
        p.save_now(&ctx, &h);
        h.insert("later-fixture".into(), "fixture".into()).unwrap();
        p.changed();
        settle(&mut p, &mut h);
        assert!(p.enabled && p.pending());
        assert_eq!(h.entries.len(), 2);
        assert_eq!(load(&path).unwrap().unwrap().entries.len(), 1);
        p.save_now(&ctx, &h);
        settle(&mut p, &mut h);
        assert!(!p.pending());
        let mut restored = History::default();
        let mut reboot = Persistence {
            path: Some(path.clone()),
            ..Default::default()
        };
        reboot.poll(&ctx, &mut restored);
        settle(&mut reboot, &mut restored);
        assert!(reboot.enabled);
        assert_eq!(restored.entries.len(), 2);
        assert_eq!(reboot.restored_revision, 1);
        reboot.forget(&ctx);
        settle(&mut reboot, &mut restored);
        assert!(!reboot.enabled);
        assert_eq!(restored.entries.len(), 2);
        assert!(load(&path).unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn failed_restore_blocks_overwrite_until_explicit_forget() {
        let path = path();
        save(&path, History::default()).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute("UPDATE clipboard SET payload=x'010203'", [])
            .unwrap();
        drop(db);
        let original = std::fs::read(&path).unwrap();
        let ctx = eframe::egui::Context::default();
        let mut p = Persistence {
            path: Some(path.clone()),
            ..Default::default()
        };
        let mut h = History::default();
        p.poll(&ctx, &mut h);
        settle(&mut p, &mut h);
        assert!(p.load_failed && !p.enabled);
        h.insert("new-fixture".into(), "fixture".into()).unwrap();
        p.changed();
        p.save_now(&ctx, &h);
        assert!(!p.busy());
        assert_eq!(original, std::fs::read(&path).unwrap());
        p.forget(&ctx);
        settle(&mut p, &mut h);
        assert!(!p.load_failed);
        p.save_now(&ctx, &h);
        settle(&mut p, &mut h);
        assert!(p.enabled);
        assert_eq!(load(&path).unwrap().unwrap().entries[0].text, "new-fixture");
        std::fs::remove_file(path).unwrap();
    }
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "zi-clipboard-store-{}.sqlite",
            uuid::Uuid::new_v4()
        ))
    }
    #[test]
    fn windows_protection_roundtrip_mutations_clear_and_disable() {
        let p = path();
        let mut h = History::default();
        h.insert("合成多行\nfixture-only".into(), "fixture".into())
            .unwrap();
        h.entries[0].pinned = true;
        save(&p, h.clone()).unwrap();
        let raw = std::fs::read(&p).unwrap();
        assert!(
            !raw.windows(b"fixture-only".len())
                .any(|w| w == b"fixture-only")
        );
        let restored = load(&p).unwrap().unwrap();
        assert_eq!(restored.entries[0].text, h.entries[0].text);
        assert!(restored.entries[0].pinned);
        h.entries.clear();
        save(&p, h).unwrap();
        assert!(load(&p).unwrap().unwrap().entries.is_empty());
        forget(&p).unwrap();
        assert!(load(&p).unwrap().is_none());
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn corruption_and_foreign_database_never_overwrite_saved_content() {
        let p = path();
        let db = Connection::open(&p).unwrap();
        db.execute_batch(
            "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('synthetic');",
        )
        .unwrap();
        drop(db);
        let before = std::fs::read(&p).unwrap();
        assert!(save(&p, History::default()).is_err());
        assert_eq!(before, std::fs::read(&p).unwrap());
        std::fs::remove_file(&p).unwrap();
        let mut h = History::default();
        h.insert("fixture".into(), "fixture".into()).unwrap();
        save(&p, h).unwrap();
        let db = Connection::open(&p).unwrap();
        db.execute("UPDATE clipboard SET payload=x'010203'", [])
            .unwrap();
        drop(db);
        let before = std::fs::read(&p).unwrap();
        assert!(load(&p).is_err());
        assert_eq!(before, std::fs::read(&p).unwrap());
        std::fs::remove_file(p).unwrap();
    }
}
