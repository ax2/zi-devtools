//! Update-specific size policy using shared local file operations.
use super::delta::MAX_FILE;
#[cfg(test)]
use crate::local_files::leaf;
pub(super) use crate::local_files::open_regular;
use anyhow::{Result, ensure};
#[cfg(test)]
use std::fs;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
fn check(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    Ok(())
}
pub fn read(path: &Path, patch: bool, cancel: &AtomicBool) -> Result<Vec<u8>> {
    check(cancel)?;
    let limit = if patch {
        MAX_FILE + 2 * 1024 * 1024
    } else {
        MAX_FILE
    };
    let mut file = open_regular(path, limit)?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 65536];
    loop {
        check(cancel)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        ensure!(bytes.len() + n <= limit, "文件读取超过限制");
        bytes.extend_from_slice(&buffer[..n]);
    }
    Ok(bytes)
}
pub fn save_new(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<PathBuf> {
    crate::local_files::save_new(path, bytes, cancel)
}
#[cfg(test)]
fn publish(
    path: &Path,
    bytes: &[u8],
    cancel: &AtomicBool,
    before_commit: impl FnOnce(),
) -> Result<PathBuf> {
    crate::local_files::publish(path, bytes, cancel, before_commit)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_names_and_streams_are_rejected_before_io() {
        for name in [
            "CON.exe",
            "PRN",
            "LPT1.txt",
            "COM9.exe",
            "COM¹.txt",
            "file:stream",
            "trailing.",
            "space ",
        ] {
            assert!(leaf(Path::new(name)).is_err(), "{name}");
        }
        for name in [
            "更新包.zidelta",
            "ZiDevTools.exe",
            "COM10.txt",
            "example.data",
        ] {
            assert!(leaf(Path::new(name)).is_ok(), "{name}");
        }
    }
    #[test]
    fn publication_never_overwrites_even_with_a_competing_writer() {
        let root = std::env::temp_dir().join(format!("zi-update-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let cancel = AtomicBool::new(false);
        let output = root.join("output.exe");
        publish(&output, b"new", &cancel, || {
            fs::write(&output, b"competitor").unwrap();
        })
        .unwrap_err();
        assert_eq!(fs::read(&output).unwrap(), b"competitor");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let second = root.join("second.exe");
        save_new(&second, b"verified", &cancel).unwrap();
        assert_eq!(read(&second, false, &cancel).unwrap(), b"verified");
        assert!(save_new(&second, b"replacement", &cancel).is_err());
        let third = root.join("third.exe");
        assert!(
            publish(&third, b"new", &cancel, || {
                cancel.store(true, Ordering::Relaxed);
            })
            .is_err()
        );
        assert!(!third.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
