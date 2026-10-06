//! Read-only MSI identity probe. Never installs or repairs.
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--portable-directory" {
        zi_devtools::updates::portable_process::reject_installer_marker(std::path::Path::new(
            &args[1],
        ))?;
        println!("PORTABLE_DIRECTORY_ALLOWED");
        return Ok(());
    }
    anyhow::ensure!(
        args.len() <= 1,
        "Usage: inspect_installer [MSI | --portable-directory DIRECTORY]"
    );
    if let Some(path) = args.first() {
        let package = zi_devtools::updates::installer::inspect(std::path::Path::new(path))?;
        println!("{}", serde_json::to_string_pretty(&package)?);
    } else {
        let products = zi_devtools::updates::installer::registered_products()?;
        println!("{}", serde_json::to_string_pretty(&products)?);
    }
    Ok(())
}
