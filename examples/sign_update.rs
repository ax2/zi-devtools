//! Release engineering: hashes artifacts and signs metadata, never executes files.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::Ed25519KeyPair;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::atomic::AtomicBool};
use zeroize::Zeroizing;
use zi_devtools::{
    credentials::{self, Secret, Target},
    updates::{
        delta_files,
        signed::{self, File, Manifest},
    },
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5 || (args.len() == 6 && args[5] == "--local-vault"),
        "usage: sign_update ASSET_DIR CATALOG VERSION SOURCE_COMMIT OUTPUT_DIR [--local-vault]"
    );
    let version = semver::Version::parse(&args[2])?.to_string();
    let catalog = std::fs::read(&args[1]).context("Cannot read public catalog")?;
    let tools = signed::tools(&catalog, &version)?;
    let cancelled = AtomicBool::new(false);
    let mut files = Vec::new();
    for name in [
        format!("ZiDevTools-{version}-windows-x64.exe"),
        format!("ZiDevTools-{version}-windows-x64.msi"),
        format!("ZiDevToolsMcp-{version}-windows-x64.exe"),
    ] {
        let bytes = delta_files::read(&PathBuf::from(&args[0]).join(&name), false, &cancelled)?;
        files.push(File {
            name,
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        });
    }
    let manifest = Manifest {
        schema: 1,
        version,
        source_commit: args[3].clone(),
        platform: "windows-x64".into(),
        minimum_windows_major: 10,
        files,
        tools,
    };
    let secret = if args.len() == 6 {
        credentials::read(&Target::update_publisher())?
            .context("Local publisher credential missing")?
    } else {
        let value = Zeroizing::new(
            std::env::var("ZI_DEVTOOLS_UPDATE_SIGNING_KEY")
                .map_err(|_| anyhow::anyhow!("Release signing secret missing"))?,
        );
        Secret::new(value.to_string())?
    };
    let decoded = Zeroizing::new(
        STANDARD
            .decode(secret.expose())
            .map_err(|_| anyhow::anyhow!("Release signing secret invalid"))?,
    );
    let key = Ed25519KeyPair::from_pkcs8(&decoded)
        .map_err(|_| anyhow::anyhow!("Release signing secret invalid"))?;
    let (raw, sig) = signed::sign(&manifest, &key)?;
    let authenticated = signed::verify_official(&raw, &sig)
        .context("Signing key does not match embedded publisher trust")?;
    let output = PathBuf::from(&args[4]);
    ensure!(output.is_absolute(), "Output directory must be absolute");
    delta_files::save_new(&output.join("update-manifest.json"), &raw, &cancelled)?;
    delta_files::save_new(&output.join("update-manifest.sig"), &sig, &cancelled)?;
    println!(
        "{}",
        serde_json::json!({"version":manifest.version,"files":manifest.files.len(),"tools":manifest.tools.len(),"key_id":authenticated.key_id,"signature_verified":true})
    );
    Ok(())
}
