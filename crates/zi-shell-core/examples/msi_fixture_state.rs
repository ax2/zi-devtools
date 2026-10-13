//! Disposable CI observer; recovery uses the same receipt-backed preview/apply as UI.
#[cfg(windows)]
#[path = "msi_fixture_state/capabilities.rs"]
mod capabilities;
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    std::hint::black_box(&capabilities::DECLARATION);
    use zi_shell_core::{
        explorer::lifecycle::{Identity, installer, store},
        msi,
    };
    let args: Vec<_> = std::env::args().skip(1).collect();
    let value = match args.as_slice() {
        [config_flag, _, no_restore, open, path]
            if config_flag == "--config" && no_restore == "--no-restore" && open == "--open-files" => {
            // Minimal headless fixture receiver; no GUI or import claim.
            let path = std::fs::canonicalize(std::path::absolute(path)?)?;
            anyhow::ensure!(path.is_file(), "Fixture receiver requires one ordinary file");
            let value = serde_json::json!({
                "fixtureOnly":true,
                "received":path,
                "arguments":args,
                "contentRead":false,
                "executed":false
            });
            // The installed Shell invocation probe uses a fresh, named file
            // with exact synthetic bytes. Only this test fixture writes a
            // receipt, and it is placed beside that runner-owned input.
            if path.file_name().is_some_and(|name| name == "真实文件 %1 & 空格.json") {
                anyhow::ensure!(
                    std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                        && std::env::var("CI").as_deref() == Ok("true")
                        && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
                    "Synthetic Shell receipt is limited to a disposable hosted runner"
                );
                let temporary = std::fs::canonicalize(
                    std::env::var_os("RUNNER_TEMP").ok_or_else(|| anyhow::anyhow!("RUNNER_TEMP missing"))?,
                )?;
                anyhow::ensure!(
                    path.starts_with(temporary),
                    "Synthetic Shell input is outside the disposable runner directory"
                );
                anyhow::ensure!(
                    std::fs::read(&path)? == b"synthetic installed Shell invocation fixture",
                    "Refusing a receipt for non-synthetic input"
                );
                let receipt = path.with_file_name(".zidevtools-shell-receipt.json");
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(receipt)?;
                file.write_all(&serde_json::to_vec(&value)?)?;
                file.sync_all()?;
            }
            value
        }
        [] => serde_json::to_value(store::load()?)?,
        [flag] if flag == "--products" => serde_json::to_value(msi::registered_products()?)?,
        [flag] if flag == "--journal" => serde_json::Value::Array(store::recovery::list()?.into_iter()
            .map(|record| serde_json::json!({"id":record.operation_id,"status":format!("{:?}",record.status)})).collect()),
        [flag, path] if flag == "--package" => {
            serde_json::to_value(msi::inspect(std::path::Path::new(path))?)?
        }
        [flag, id] if flag == "--resume" || flag == "--forget" || flag == "--owner-commit" || flag == "--cancel-prepared" || flag == "--restore-menu" => {
            anyhow::ensure!(
                std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                    && std::env::var("CI").as_deref() == Ok("true")
                    && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
                "Fixture mutation requires a disposable CI runner"
            );
            if flag == "--resume" {
                store::recovery::completion::apply(store::recovery::completion::preview(id)?)?;
            } else if flag == "--owner-commit" {
                // Fixture calls only after real MsiEndTransaction(COMMIT) returned 0.
                store::recovery::completion::mark_committed(id)?;
            } else if flag == "--cancel-prepared" {
                store::recovery::cancellation::apply(store::recovery::cancellation::preview(id)?)?;
            } else if flag == "--restore-menu" {
                store::recovery::restoration::apply(store::recovery::restoration::preview(id)?)?;
            } else {
                store::recovery::forget(id)?;
            }
            serde_json::to_value(store::load()?)?
        }
        [flag, config] if flag == "--portable" || flag == "--remove-portable" => {
            anyhow::ensure!(
                std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
                    && std::env::var("CI").as_deref() == Ok("true")
                    && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
                "Fixture mutation requires a disposable CI runner"
            );
            let request = if flag == "--portable" {
                installer::Request::Install
            } else {
                let executable = std::fs::canonicalize(std::env::current_exe()?)?;
                let ledger = store::load()?;
                let own = ledger
                    .entries()
                    .iter()
                    .find(|e| {
                        matches!(e.installation.identity, Identity::Portable(_))
                            && e.installation.executable == executable
                    })
                    .ok_or_else(|| anyhow::anyhow!("Fixture portable record absent"))?;
                installer::Request::Remove(own.installation.identity.clone())
            };
            let config = std::path::Path::new(config);
            installer::apply(installer::preview(config, request, "9.8.7")?, config)?;
            serde_json::to_value(store::load()?)?
        }
        _ => anyhow::bail!("Unsupported fixture arguments"),
    };
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}
#[cfg(not(windows))]
fn main() {}
