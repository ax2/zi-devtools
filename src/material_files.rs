//! Explicitly selected file materials. Paths never imply persistent authorization.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    object: (u64, u64),
    bytes: u64,
    modified: std::time::SystemTime,
}
fn identity(file: &File) -> Result<Identity> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    let object = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // The live File owns this valid handle; the output buffer has the API's exact type.
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
            "无法核对文件身份：{}",
            std::io::Error::last_os_error()
        );
        (
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        )
    };
    #[cfg(unix)]
    let object = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(not(any(windows, unix)))]
    let object = (0, 0);
    Ok(Identity {
        object,
        bytes: metadata.len(),
        modified: metadata.modified()?,
    })
}
fn check(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
    Ok(())
}

/// In-memory reference captured only after the caller's explicit file selection.
/// No file contents, grants or user credentials are serialized here.
#[derive(Clone, Debug)]
pub struct FileMaterial {
    path: PathBuf,
    identity: Identity,
}
impl FileMaterial {
    pub fn selected(path: &Path, max_bytes: usize) -> Result<Self> {
        ensure!(path.is_absolute(), "请选择完整文件路径");
        let file = crate::local_files::open_regular(path, max_bytes)?;
        let identity = identity(&file)?;
        let material = Self {
            path: path.canonicalize()?,
            identity,
        };
        // Re-open the pinned path to reject replacement during selection.
        material.open(max_bytes)?;
        Ok(material)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> u64 {
        self.identity.bytes
    }
    fn open(&self, max_bytes: usize) -> Result<File> {
        let file = crate::local_files::open_regular(&self.path, max_bytes)
            .context("源文件不可用，请重新选择")?;
        ensure!(
            identity(&file)? == self.identity,
            "源文件已改变，请重新选择并审核"
        );
        Ok(file)
    }
    /// Bounded streaming read. File size limits are supplied by the action, not
    /// inherited from the text pipeline's one-MiB input limit.
    pub fn sha256(&self, max_bytes: usize, cancel: &AtomicBool) -> Result<HashOutputs> {
        check(cancel)?;
        let mut file = self.open(max_bytes)?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut bytes = 0u64;
        loop {
            check(cancel)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes = bytes.checked_add(count as u64).context("文件大小溢出")?;
            ensure!(bytes <= max_bytes as u64, "文件超出本次允许大小");
            digest.update(&buffer[..count]);
        }
        check(cancel)?;
        ensure!(
            bytes == self.identity.bytes && identity(&file)? == self.identity,
            "读取期间源文件改变，本次结果未发布"
        );
        // Detect path replacement while the original open handle remains readable.
        self.open(max_bytes)?;
        let sha256 = format!("{:x}", digest.finalize());
        Ok(HashOutputs {
            source: self.clone(),
            report: format!("SHA-256: {sha256}\n字节数: {bytes}"),
            sha256,
        })
    }
}
/// Two separately usable outputs; the original file is never modified.
#[derive(Clone, Debug)]
pub struct HashOutputs {
    pub source: FileMaterial,
    pub report: String,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("zi-material-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("中文 文件.bin")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn binary_file_produces_full_digest_and_original_reference() {
        let fixture = Fixture::new();
        let path = fixture.file();
        let data = vec![0xff; 2 * 1024 * 1024 + 17];
        std::fs::write(&path, &data).unwrap();
        let material = FileMaterial::selected(&path, data.len()).unwrap();
        let result = material
            .sha256(data.len(), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(result.sha256, format!("{:x}", Sha256::digest(&data)));
        assert_eq!(result.source.path(), path.canonicalize().unwrap());
        assert_eq!(result.source.bytes(), data.len() as u64);
        assert!(result.report.contains(&data.len().to_string()));
        assert_eq!(std::fs::read(path).unwrap(), data);
    }
    #[test]
    fn replacement_with_same_bytes_is_still_a_different_file() {
        let fixture = Fixture::new();
        let path = fixture.file();
        std::fs::write(&path, b"same").unwrap();
        let material = FileMaterial::selected(&path, 1024).unwrap();
        std::fs::rename(&path, fixture.0.join("original.bin")).unwrap();
        std::fs::write(&path, b"same").unwrap();
        assert!(material.sha256(1024, &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn deleted_changed_over_budget_and_cancelled_sources_publish_no_results() {
        let fixture = Fixture::new();
        let path = fixture.file();
        std::fs::write(&path, b"original").unwrap();
        assert!(FileMaterial::selected(&path, 2).is_err());
        let material = FileMaterial::selected(&path, 1024).unwrap();
        assert!(material.sha256(2, &AtomicBool::new(false)).is_err());
        assert!(material.sha256(1024, &AtomicBool::new(true)).is_err());
        std::fs::write(&path, b"changed longer").unwrap();
        assert!(material.sha256(1024, &AtomicBool::new(false)).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(material.sha256(1024, &AtomicBool::new(false)).is_err());
        assert!(FileMaterial::selected(Path::new("relative.txt"), 1024).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn linked_input_is_not_a_file_material() {
        let fixture = Fixture::new();
        let path = fixture.file();
        std::fs::write(&path, b"target").unwrap();
        let link = fixture.0.join("linked.bin");
        if std::os::windows::fs::symlink_file(&path, &link).is_ok() {
            assert!(FileMaterial::selected(&link, 1024).is_err());
        } else {
            // A directory junction needs no elevated symlink privilege.
            let junction = fixture.0.join("alias");
            let status = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&junction)
                .arg(&fixture.0)
                .output()
                .unwrap();
            assert!(status.status.success());
            // Directory aliases are intentionally resolved by local_files;
            // the captured reference is the canonical ordinary file.
            let material = FileMaterial::selected(&junction.join("中文 文件.bin"), 1024).unwrap();
            assert_eq!(material.path(), path.canonicalize().unwrap());
            std::fs::remove_dir(junction).unwrap();
        }
    }
}
