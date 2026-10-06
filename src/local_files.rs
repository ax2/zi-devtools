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
    let temporary = Temporary(parent.join(format!(".zi-local-{}.tmp", uuid::Uuid::new_v4())));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary.0)
        .context("无法创建暂存文件")?;
    for chunk in bytes.chunks(65536) {
        check(cancel)?;
        file.write_all(chunk)?;
    }
    file.sync_all()?;
    drop(file);
    before_commit();
    check(cancel)?;
    ordinary(&parent)?;
    fs::hard_link(&temporary.0, &path)
        .context("无法无覆盖发布文件：目标已出现或文件系统不支持硬链接")?;
    Ok(path) // Commit point; cancellation afterwards cannot delete a published result.
}
