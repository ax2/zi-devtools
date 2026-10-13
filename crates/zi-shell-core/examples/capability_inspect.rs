//! Read-only declaration inspection; never starts the supplied executable.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        arguments.len() == 1 || (arguments.len() == 3 && arguments[1] == "--plan"),
        "Expected executable [--plan config-path]"
    );
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
    let declaration_property = declaration
        .as_ref()
        .map(|value| value.encode_msi_property())
        .transpose()
        .map_err(anyhow::Error::msg)?;
    let plan = if arguments.len() == 3 {
        let declaration = declaration
            .as_ref()
            .context("Unknown legacy capabilities: cannot generate a declared menu plan")?;
        Some(zi_shell_core::explorer::plan_declared(
            &std::fs::canonicalize(&arguments[0])?,
            &std::path::absolute(&arguments[2])?,
            declaration,
        )?)
    } else {
        None
    };
    println!(
        "{}",
        serde_json::json!({"sha256":format!("{:x}",digest.finalize()), "bytes":length,
        "declarationPresent":declaration.is_some(), "declarationProperty":declaration_property,
        "actions":targets, "executableStarted":false,
        "publisherAuthenticated":false, "ledgerUpdated":false, "declaredPlan":plan})
    );
    std::hint::black_box(file.metadata().context("Final consistency check")?);
    Ok(())
}
