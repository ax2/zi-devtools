//! Shared bounded ordinary-file reads and atomic no-replacement publication.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
fn check(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    Ok(())
}
pub(crate) fn leaf(path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("文件名无效")?;
    ensure!(
        !name.is_empty()
            && !name.ends_with(['.', ' '])
            && name.encode_utf16().count() <= 255
            && !name.chars().any(|c| c.is_control()
                || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*' | '/' | '\\')),
        "只支持普通Windows文件名，不支持数据流或保留字符"
    );
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(
                stem.get(3..),
                Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
            ));
    ensure!(!device, "不支持Windows设备文件名");
    Ok(())
}
fn ordinary(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let metadata = fs::symlink_metadata(ancestor).context("无法检查文件位置")?;
        ensure!(!metadata.file_type().is_symlink(), "不支持链接或重解析位置");
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "不支持链接或重解析位置"
            );
        }
    }
    Ok(())
}
pub(crate) fn open_regular(path: &Path, limit: usize) -> Result<fs::File> {
    leaf(path)?;
    // Resolve directory aliases (the workspace itself may be a junction), but
    // never accept a linked input file. Operate on the pinned resolved path.
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "请选择普通文件"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "不支持链接输入文件"
        );
    }
    let path = path.canonicalize()?;
    ordinary(&path)?;
    let file = fs::File::open(&path).context("无法读取所选文件")?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit as u64,
        "文件类型或大小超出限制"
    );
    Ok(file)
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub(crate) fn save_new(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<PathBuf> {
    publish(path, bytes, cancel, || {})
}
pub(crate) fn publish(
    path: &Path,
    bytes: &[u8],
    cancel: &AtomicBool,
    before_commit: impl FnOnce(),
) -> Result<PathBuf> {
    publish_using(
        path,
        cancel,
        before_commit,
        |file| write_bytes(file, bytes, cancel),
        |temporary, path| {
            fs::hard_link(temporary, path)
                .context("无法无覆盖发布文件：目标已出现或文件系统不支持硬链接")
        },
    )
}

fn write_bytes(file: &mut fs::File, bytes: &[u8], cancel: &AtomicBool) -> Result<()> {
    for chunk in bytes.chunks(65536) {
        check(cancel)?;
        file.write_all(chunk)?;
    }
    Ok(())
}

/// Same-directory move without replacement on Windows, including volumes without hard links.
pub(crate) fn save_new_moved(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<PathBuf> {
    publish_using(
        path,
        cancel,
        || {},
        |file| write_bytes(file, bytes, cancel),
        move_new,
    )
}

fn move_new(temporary: &Path, path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::{
            Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW},
            core::HSTRING,
        };
        // No REPLACE_EXISTING, COPY_ALLOWED or delayed reboot operation.
        unsafe {
            MoveFileExW(
                &HSTRING::from(temporary.as_os_str()),
                &HSTRING::from(path.as_os_str()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .context("无法发布流程文件：目标已出现或文件系统拒绝移动")?;
    }
    #[cfg(not(windows))]
    fs::hard_link(temporary, path).context("无法无覆盖发布文件")?;
    Ok(())
}

fn publish_using(
    path: &Path,
    cancel: &AtomicBool,
    before_commit: impl FnOnce(),
    write: impl FnOnce(&mut fs::File) -> Result<()>,
    commit: impl FnOnce(&Path, &Path) -> Result<()>,
) -> Result<PathBuf> {
    check(cancel)?;
    leaf(path)?;
    ensure!(
        path.is_absolute() && path.file_name().is_some(),
        "请选择完整输出路径"
    );
    let parent = path.parent().context("输出目录无效")?.canonicalize()?;
    ordinary(&parent)?;
    let path = parent.join(path.file_name().context("输出文件名无效")?);
    ensure!(parent.is_dir(), "输出目录不存在");
    ensure!(
        fs::symlink_metadata(&path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "目标已存在或无法核对，不会覆盖"
    );
    let temporary_path = parent.join(format!(".zi-local-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .context("无法创建暂存文件")?;
    // Only own cleanup after exclusive creation succeeds.
    let temporary = Temporary(temporary_path);
    let written = write(&mut file).and_then(|_| file.sync_all().map_err(Into::into));
    drop(file);
    written?;
    before_commit();
    check(cancel)?;
    ordinary(&parent)?;
    commit(&temporary.0, &path)?;
    Ok(path) // Commit point; cancellation afterwards cannot delete a published result.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("zi-file-publish-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        root
    }
    #[test]
    fn moved_publication_preserves_competitor_and_never_exposes_staging() {
        let root = root();
        let path = root.join("中文 流程.json");
        let cancel = AtomicBool::new(false);
        assert!(
            publish_using(
                &path,
                &cancel,
                || {
                    assert!(!path.exists());
                    fs::write(&path, b"competitor").unwrap();
                },
                |f| {
                    f.write_all(b"complete new file")?;
                    Ok(())
                },
                move_new
            )
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"competitor");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let second = root.join("second.json");
        save_new_moved(&second, b"complete", &cancel).unwrap();
        assert_eq!(fs::read(&second).unwrap(), b"complete");
        assert!(save_new_moved(&second, b"replacement", &cancel).is_err());
        assert_eq!(fs::read(&second).unwrap(), b"complete");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn partial_write_and_commit_failures_clean_staging_and_leave_target_absent() {
        let root = root();
        let path = root.join("failed.json");
        let cancel = AtomicBool::new(false);
        let result = publish_using(
            &path,
            &cancel,
            || {},
            |f| {
                f.write_all(b"partial JSON")?;
                Err(std::io::Error::other("injected write failure after prefix").into())
            },
            |_, _| panic!("must not commit failed write"),
        );
        assert!(result.is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let result = publish_using(
            &path,
            &cancel,
            || {},
            |f| {
                f.write_all(b"complete")?;
                Ok(())
            },
            |_, _| anyhow::bail!("injected commit failure"),
        );
        assert!(result.is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancellation_before_move_cleans_staging_without_publishing() {
        let root = root();
        let path = root.join("cancel.json");
        let cancel = AtomicBool::new(false);
        assert!(
            publish_using(
                &path,
                &cancel,
                || {
                    cancel.store(true, Ordering::Relaxed);
                },
                |f| {
                    f.write_all(b"complete")?;
                    Ok(())
                },
                move_new
            )
            .is_err()
        );
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn simultaneous_moved_writers_publish_exactly_one_complete_file() {
        let root = root();
        let path = root.join("concurrent.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let barrier = barrier.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    let bytes = format!("{{\"writer\":{i},\"value\":\"{}\"}}", "x".repeat(65537))
                        .into_bytes();
                    barrier.wait();
                    (
                        save_new_moved(&path, &bytes, &AtomicBool::new(false)).is_ok(),
                        bytes,
                    )
                })
            })
            .collect();
        let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        let successful: Vec<_> = results.iter().filter(|(ok, _)| *ok).collect();
        assert_eq!(successful.len(), 1);
        assert_eq!(fs::read(&path).unwrap(), successful[0].1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
