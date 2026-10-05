//! Offline integrity probe; never executes the selected release file.
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: verify_update VERSION SHA256SUMS ASSET"
    );
    let version = semver::Version::parse(&args[0])?;
    let report = zi_devtools::updates::verification::verify(
        std::path::Path::new(&args[1]),
        std::path::Path::new(&args[2]),
        &version,
        &std::sync::atomic::AtomicBool::new(false),
    )?;
    println!(
        "{}",
        serde_json::json!({"version": report.version.to_string(), "name": report.name,
        "size": report.size, "sha256": report.sha256, "publisher_signature_verified": false})
    );
    Ok(())
}
