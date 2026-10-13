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
            // Minimal headless fixture receiver; no GUI/import claim or file reads.
            let path = std::path::absolute(path)?;
            anyhow::ensure!(path.is_file(), "Fixture receiver requires one ordinary file");
            serde_json::json!({"fixtureOnly":true,"received":path,"contentRead":false,"executed":false})
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
