#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use eframe::egui;
use zi_devtools::{
    app::DevToolsApp,
    config::{default_config_path, import_config},
    fixture, tray,
};

fn main() -> Result<()> {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    if raw.first().is_some_and(|s| s == "--apply-msi-update") {
        anyhow::ensure!(raw.len() == 5, "MSI helper needs PLAN PID STAMP CONFIG");
        return zi_devtools::updates::msi::run_helper(
            std::path::Path::new(&raw[1]),
            raw[2].to_str().context("Invalid parent PID")?.parse()?,
            raw[3].to_str().context("Invalid parent stamp")?.parse()?,
            std::path::Path::new(&raw[4]),
        );
    }
    if raw.first().is_some_and(|s| s == "--apply-portable-update") {
        anyhow::ensure!(
            matches!(raw.len(), 6 | 7),
            "Invalid portable helper arguments"
        );
        let pid = raw[2].to_str().context("Invalid parent PID")?.parse()?;
        let stamp = raw[3]
            .to_str()
            .context("Invalid parent identity")?
            .parse()?;
        let action = raw[4].to_str().context("Invalid update action")?;
        anyhow::ensure!(
            matches!(action, "apply" | "restore"),
            "Invalid update action"
        );
        anyhow::ensure!(
            raw.len() == 6 || raw[6] == "--no-restart",
            "Invalid helper restart argument"
        );
        return zi_devtools::updates::portable::run_helper(
            std::path::Path::new(&raw[1]),
            pid,
            stamp,
            action == "restore",
            std::path::Path::new(&raw[5]),
            raw.len() == 6,
        );
    }
    if raw
        .first()
        .is_some_and(|s| s == "--recover-portable-update")
    {
        anyhow::ensure!(
            matches!(raw.len(), 3 | 4),
            "Recovery needs PLAN CONFIG [--no-restart]"
        );
        anyhow::ensure!(
            raw.len() == 3 || raw[3] == "--no-restart",
            "Invalid recovery argument"
        );
        return zi_devtools::updates::portable::run_helper(
            std::path::Path::new(&raw[1]),
            0,
            0,
            true,
            std::path::Path::new(&raw[2]),
            raw.len() == 3,
        );
    }
    let arguments = parse_args()?;
    if let Some(port) = arguments.fixture_port {
        return fixture::run(port);
    }
    if let Some(port) = arguments.fixture_shutdown_port {
        return fixture::shutdown(port);
    }
    if let Some(source) = arguments.import_config {
        import_config(&source, &arguments.config_path, arguments.force_import)?;
        return Ok(());
    }
    if let Some(id) = &arguments.tool {
        zi_devtools::tool_shortcuts::set_process_identity(id)?;
    }

    let (rgba, width, height) = tray::rgba_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(arguments.tool.as_ref().map_or_else(
                || "Zi DevTools".to_owned(),
                |id| format!("Zi DevTools — {id}"),
            ))
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([980.0, 640.0])
            .with_icon(Arc::new(egui::IconData {
                rgba,
                width,
                height,
            })),
        ..Default::default()
    };
    eframe::run_native(
        "Zi DevTools",
        options,
        Box::new(move |cc| {
            let mut app = DevToolsApp::new(cc, arguments.config_path, arguments.restore_services);
            if let Some(id) = arguments.tool {
                app.open_startup_tool(&id);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

struct Arguments {
    tool: Option<String>,
    config_path: std::path::PathBuf,
    restore_services: bool,
    fixture_port: Option<u16>,
    fixture_shutdown_port: Option<u16>,
    import_config: Option<std::path::PathBuf>,
    force_import: bool,
}

fn parse_args() -> Result<Arguments> {
    parse_args_from(std::env::args_os().skip(1))
}

fn parse_args_from(input: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Arguments> {
    let mut tool = None;
    let mut config_path = default_config_path();
    let mut restore_services = true;
    let mut fixture_port = None;
    let mut fixture_shutdown_port = None;
    let mut import_config = None;
    let mut force_import = false;
    let mut args = input.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str().context("参数不是有效 Unicode")? {
            "--tool" => {
                let id = args
                    .next()
                    .context("--tool requires a tool ID")?
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("工具编号不是有效 Unicode"))?;
                zi_devtools::tool_shortcuts::validate_id(&id)?;
                anyhow::ensure!(tool.is_none(), "--tool cannot be repeated");
                tool = Some(id);
            }
            "--fixture-server" => {
                let value = args.next().context("--fixture-server requires a port")?;
                fixture_port = Some(
                    value
                        .to_str()
                        .context("invalid port")?
                        .parse()
                        .context("invalid fixture port")?,
                );
            }
            "--fixture-shutdown" => {
                let value = args.next().context("--fixture-shutdown requires a port")?;
                fixture_shutdown_port = Some(
                    value
                        .to_str()
                        .context("invalid port")?
                        .parse()
                        .context("invalid fixture port")?,
                );
            }
            "--config" => {
                config_path = args
                    .next()
                    .map(std::path::PathBuf::from)
                    .context("--config requires a path")?;
            }
            "--no-restore" => restore_services = false,
            "--import-config" => {
                import_config = Some(
                    args.next()
                        .map(std::path::PathBuf::from)
                        .context("--import-config requires a path")?,
                );
            }
            "--force-import" => force_import = true,
            _ => {}
        }
    }
    Ok(Arguments {
        restore_services: restore_services && tool.is_none(),
        tool,
        config_path,
        fixture_port,
        fixture_shutdown_port,
        import_config,
        force_import,
    })
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_launch_is_explicit_and_does_not_restore_services() {
        let parse = |args: &[&str]| parse_args_from(args.iter().map(std::ffi::OsString::from));
        let args = parse(&[
            "--tool",
            "json",
            "--config",
            "C:\\资料 folder\\services.yml",
        ])
        .unwrap();
        assert_eq!(args.tool.as_deref(), Some("json"));
        assert!(!args.restore_services);
        assert!(parse(&["--tool"]).is_err());
        assert!(parse(&["--tool", "json", "--tool", "base64"]).is_err());
        assert!(parse(&[]).unwrap().restore_services);
    }
}
