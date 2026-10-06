//! Offline publisher signature and artifact integrity check. Never runs an artifact.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path, sync::atomic::AtomicBool};
use zi_devtools::updates::{delta_files, signed};
fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)?.len() <= limit as u64,
        "Signed metadata exceeds limit"
    );
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= limit, "Signed metadata exceeds limit");
    Ok(data)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: verify_signed_update MANIFEST SIGNATURE ASSET"
    );
    let raw = bounded(Path::new(&args[0]), signed::MAX_MANIFEST)?;
    let sig = bounded(Path::new(&args[1]), signed::MAX_SIGNATURE)?;
    let authenticated = signed::verify_official(&raw, &sig)?;
    let asset = Path::new(&args[2]);
    let name = asset
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("Invalid asset filename"))?;
    let bytes = delta_files::read(asset, false, &AtomicBool::new(false))?;
    let file = signed::expected_file(
        &authenticated,
        &semver::Version::parse(&authenticated.manifest.version)?,
        name,
        bytes.len() as u64,
    )?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == file.sha256,
        "Artifact SHA-256 mismatch"
    );
    println!(
        "{}",
        serde_json::json!({"version":authenticated.manifest.version,"key_id":authenticated.key_id,"name":name,"size":file.size,"sha256":file.sha256,"publisher_signature_verified":true,"file_integrity_verified":true,"executed":false})
    );
    Ok(())
}
