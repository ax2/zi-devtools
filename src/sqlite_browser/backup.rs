//! Explicit, no-overwrite snapshots of a selected SQLite database.
use super::*;
use rusqlite::{
    DatabaseName,
    backup::{Backup, StepResult},
};
use std::{
    ffi::OsString,
    sync::atomic::{AtomicU8, AtomicUsize},
};

#[derive(Default)]
pub(super) struct Progress {
    pub(super) percent: AtomicUsize,
    phase: AtomicU8,
}

impl Progress {
    pub(super) fn phase_label(&self) -> &'static str {
        match self.phase.load(Ordering::Relaxed) {
            1 => "核验完整性",
            2 => "发布备份",
            3 => "已完成",
            _ => "复制数据库",
        }
    }
}

struct TemporaryBackup(PathBuf);

impl Drop for TemporaryBackup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = OsString::from(self.0.as_os_str());
            sidecar.push(suffix);
            let _ = fs::remove_file(PathBuf::from(sidecar));
        }
    }
}

pub(super) fn create(
    source_path: &Path,
    destination_path: &Path,
    cancelled: &Arc<AtomicBool>,
    progress: &Progress,
) -> Result<PathBuf> {
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "备份已取消，未创建目标文件"
    );
    let source = crate::material_files::FileMaterial::selected(source_path, usize::MAX)?;
    let source_connection = Connection::open_with_flags(
        source.path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法以只读模式打开备份源")?;
    source_connection.busy_timeout(Duration::from_secs(1))?;

    ensure!(destination_path.is_absolute(), "请选择绝对路径作为备份目标");
    let filename = destination_path
        .file_name()
        .context("备份目标必须是文件路径")?;
    let parent = destination_path.parent().context("备份目标缺少父目录")?;
    let parent_metadata = fs::symlink_metadata(parent).context("无法读取备份目录")?;
    ensure!(
        parent_metadata.is_dir() && !crate::disk_inspector::is_link(&parent_metadata),
        "备份目标目录不能是符号链接或重解析点"
    );
    let parent = fs::canonicalize(parent).context("无法规范化备份目录")?;
    let destination = parent.join(filename);
    ensure!(destination != source.path(), "备份目标不能与源数据库相同");
    match fs::symlink_metadata(&destination) {
        Ok(_) => anyhow::bail!("目标文件已存在；为避免覆盖，未创建备份"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("无法检查备份目标"),
    }

    let temporary_path = parent.join(format!(
        ".{}.{}.tmp",
        filename.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .context("无法在目标目录创建唯一临时备份")?;
    let temporary = TemporaryBackup(temporary_path.clone());
    let mut destination_connection = Connection::open_with_flags(
        &temporary_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("无法打开临时备份文件")?;
    destination_connection.busy_timeout(Duration::from_secs(1))?;

    let mut busy_steps = 0;
    {
        let backup = Backup::new_with_names(
            &source_connection,
            DatabaseName::Main,
            &mut destination_connection,
            DatabaseName::Main,
        )
        .context("无法开始 SQLite 在线备份")?;
        loop {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "备份已取消，临时文件将被清理"
            );
            match backup.step(128).context("SQLite 在线备份失败")? {
                StepResult::Done => {
                    progress.percent.store(95, Ordering::Relaxed);
                    progress.phase.store(1, Ordering::Relaxed);
                    break;
                }
                StepResult::More => {
                    let snapshot_progress = backup.progress();
                    let percent = if snapshot_progress.pagecount <= 0 {
                        0
                    } else {
                        ((snapshot_progress.pagecount - snapshot_progress.remaining).max(0) * 100
                            / snapshot_progress.pagecount)
                            .clamp(0, 99) as usize
                    };
                    progress.percent.store(percent, Ordering::Relaxed);
                    busy_steps = 0;
                }
                StepResult::Busy | StepResult::Locked => {
                    busy_steps += 1;
                    ensure!(busy_steps <= 120, "源数据库持续繁忙，备份未完成");
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => anyhow::bail!("SQLite 在线备份返回不支持的状态"),
            }
        }
    }

    let integrity_cancel = Arc::clone(cancelled);
    destination_connection
        .progress_handler(1000, Some(move || integrity_cancel.load(Ordering::Relaxed)));
    let integrity: String = destination_connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("无法核验备份数据库完整性")?;
    ensure!(integrity == "ok", "备份完整性检查失败：{integrity}");
    ensure!(
        !cancelled.load(Ordering::Relaxed),
        "备份已取消，临时文件将被清理"
    );
    progress.percent.store(99, Ordering::Relaxed);
    progress.phase.store(2, Ordering::Relaxed);
    destination_connection
        .close()
        .map_err(|(_, error)| error)
        .context("无法关闭备份文件")?;
    drop(source_connection);
    source.validate(usize::MAX)?;

    fs::hard_link(&temporary_path, &destination)
        .context("无法以不覆盖方式发布备份；请确认目标目录支持同卷硬链接")?;
    drop(temporary);
    progress.percent.store(100, Ordering::Relaxed);
    progress.phase.store(3, Ordering::Relaxed);
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        source: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("zi-sqlite-backup-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let source = root.join("source.sqlite");
            let connection = Connection::open(&source).unwrap();
            connection
                .execute_batch(
                    "PRAGMA journal_mode=WAL; CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT); INSERT INTO notes VALUES(1,'first'),(2,'second');",
                )
                .unwrap();
            drop(connection);
            Self { root, source }
        }

        fn destination(&self) -> PathBuf {
            self.root.join("backup.sqlite")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn creates_integrity_checked_wal_aware_snapshot_without_changing_source() {
        let fixture = Fixture::new();
        let writer = Connection::open(&fixture.source).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; INSERT INTO notes VALUES(3,'wal-only');")
            .unwrap();
        let wal_path = PathBuf::from(format!("{}-wal", fixture.source.display()));
        let wal_before = fs::read(&wal_path).unwrap();
        let progress = Progress::default();
        let output = create(
            &fixture.source,
            &fixture.destination(),
            &Arc::new(AtomicBool::new(false)),
            &progress,
        )
        .unwrap();
        assert_eq!(output, fs::canonicalize(fixture.destination()).unwrap());
        assert_eq!(progress.percent.load(Ordering::Relaxed), 100);
        assert_eq!(progress.phase_label(), "已完成");
        let backup =
            Connection::open_with_flags(&output, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(
            backup
                .query_row(
                    "SELECT group_concat(title, ',') FROM (SELECT title FROM notes ORDER BY id)",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "first,second,wal-only"
        );
        assert_eq!(fs::read(&wal_path).unwrap(), wal_before);
        assert_eq!(
            writer
                .query_row("SELECT count(*) FROM notes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3,
            "backup must leave the live source database intact"
        );
    }

    #[test]
    fn refuses_existing_target_without_changing_it() {
        let fixture = Fixture::new();
        let target = fixture.destination();
        fs::write(&target, "preserve existing data").unwrap();
        let progress = Progress::default();
        assert!(
            create(
                &fixture.source,
                &target,
                &Arc::new(AtomicBool::new(false)),
                &progress
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "preserve existing data"
        );
    }

    #[test]
    fn cancellation_before_work_creates_no_output() {
        let fixture = Fixture::new();
        let target = fixture.destination();
        let progress = Progress::default();
        assert!(
            create(
                &fixture.source,
                &target,
                &Arc::new(AtomicBool::new(true)),
                &progress
            )
            .is_err()
        );
        assert!(!target.exists());
        assert_eq!(
            fs::read_dir(&fixture.root).unwrap().count(),
            1,
            "cancellation must leave only the original source database"
        );
    }
}
