//! Publisher setup only. Stdout contains public information, never key material.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::Ed25519KeyPair;
use std::{
    io::Write,
    process::{Command, Stdio},
};
use zeroize::Zeroizing;
use zi_devtools::{
    credentials::{self, Secret, Target},
    updates::signed::{self, TrustStore},
};

fn main() -> Result<()> {
    let action = std::env::args()
        .nth(1)
        .context("usage: publisher_key init|upload|check-env")?;
    ensure!(
        matches!(action.as_str(), "init" | "upload" | "check-env"),
        "unsupported publisher action"
    );
    let target = Target::update_publisher();
    let value = if action == "check-env" {
        let value = Zeroizing::new(
            std::env::var("ZI_DEVTOOLS_UPDATE_SIGNING_KEY")
                .map_err(|_| anyhow::anyhow!("Project signing secret missing"))?,
        );
        Secret::new(value.to_string())?
    } else {
        match credentials::read(&target)? {
            Some(value) => value,
            None if action == "init" => {
                let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                    .map_err(|_| anyhow::anyhow!("Cannot generate publisher key"))?;
                let value = Secret::new(STANDARD.encode(pkcs8.as_ref()))?;
                credentials::save(&target, value.expose())?;
                value
            }
            None => anyhow::bail!("Publisher credential missing; initialize locally first"),
        }
    };
    let bytes = Zeroizing::new(
        STANDARD
            .decode(value.expose())
            .map_err(|_| anyhow::anyhow!("Invalid stored publisher key"))?,
    );
    let key = Ed25519KeyPair::from_pkcs8(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid stored publisher key"))?;
    let public = signed::publisher(&key);
    if action == "init" {
        println!(
            "{}",
            serde_json::to_string_pretty(&TrustStore {
                schema: 1,
                keys: vec![public]
            })?
        );
    } else if action == "check-env" {
        let trust: TrustStore = serde_json::from_str(include_str!("../docs/update-trust.json"))?;
        ensure!(
            trust.schema == 1
                && trust
                    .keys
                    .iter()
                    .any(|k| k.id == public.id && k.public_key == public.public_key),
            "Project signing secret does not match embedded publisher trust"
        );
        let test = key.sign(b"ZiDevTools publisher key diagnostic v1");
        let public_bytes = STANDARD.decode(&public.public_key)?;
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &public_bytes)
            .verify(b"ZiDevTools publisher key diagnostic v1", test.as_ref())
            .map_err(|_| anyhow::anyhow!("Publisher key diagnostic failed"))?;
        println!(
            "Publisher signing secret verified against public fingerprint {}",
            public.id
        );
    } else {
        let listing = Command::new("gh")
            .args([
                "secret",
                "list",
                "--repo",
                "ax2/zi-devtools",
                "--json",
                "name",
            ])
            .output()
            .context("Cannot check project secret names")?;
        ensure!(
            listing.status.success(),
            "Cannot check project secret names; no upload attempted"
        );
        let names: Vec<serde_json::Value> =
            serde_json::from_slice(&listing.stdout).context("Invalid project secret response")?;
        ensure!(
            !names
                .iter()
                .any(|n| n["name"] == "ZI_DEVTOOLS_UPDATE_SIGNING_KEY"),
            "Signing secret already exists; refusing to overwrite it"
        );
        let mut child = Command::new("gh")
            .args([
                "secret",
                "set",
                "ZI_DEVTOOLS_UPDATE_SIGNING_KEY",
                "--repo",
                "ax2/zi-devtools",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start project secret upload")?;
        let write = child
            .stdin
            .take()
            .context("Cannot open secret input")?
            .write_all(value.expose().as_bytes());
        let status = child
            .wait()
            .context("Cannot confirm project secret upload")?;
        ensure!(
            write.is_ok() && status.success(),
            "Project secret upload failed; encrypted local publisher credential retained"
        );
        println!(
            "Publisher public key {} configured in project secret ZI_DEVTOOLS_UPDATE_SIGNING_KEY",
            public.id
        );
    }
    Ok(())
}
