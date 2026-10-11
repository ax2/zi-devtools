#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use eframe::egui;
use zi_devtools::{app::DevToolsApp, config::import_config, fixture, launch::parse_args, tray};

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
            let mut app = DevToolsApp::new(
                cc,
                arguments.config_path.clone(),
                arguments.restore_services,
            );
            app.open_startup_arguments(&arguments)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

use std::sync::Arc;
