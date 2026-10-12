//! Read-only declaration inspection; never starts the supplied executable.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(arguments.len() == 1, "Expected one executable path");
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    let mut file = options.open(&arguments[0])?;
    let length = file.metadata()?.len();
    ensure!(
        (64..=256 * 1024 * 1024).contains(&length),
        "Image outside supported bound"
    );
    let declaration = zi_shell_core::capability_image::read(&mut file, length)?;
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(total <= length, "Image size changed");
        digest.update(&bytes[..count]);
    }
    ensure!(
        total == length && file.metadata()?.len() == length,
        "Image changed"
    );
    let targets = declaration.as_ref().map(|value| {
        value
            .targets()
            .iter()
            .map(|t| t.action_id())
            .collect::<Vec<_>>()
    });
    println!(
        "{}",
        serde_json::json!({"sha256":format!("{:x}",digest.finalize()), "bytes":length,
        "declarationPresent":declaration.is_some(), "actions":targets, "executableStarted":false,
        "publisherAuthenticated":false, "ledgerUpdated":false})
    );
    std::hint::black_box(file.metadata().context("Final consistency check")?);
    Ok(())
}
