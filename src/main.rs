#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use eframe::egui;
use zi_devtools::{
    app::DevToolsApp,
    config::{default_config_path, import_config},
    fixture, tray,
};

fn main() -> Result<()> {
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

    let (rgba, width, height) = tray::rgba_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Zi DevTools")
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
            Ok(Box::new(DevToolsApp::new(
                cc,
                arguments.config_path,
                arguments.restore_services,
            )))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

struct Arguments {
    config_path: std::path::PathBuf,
    restore_services: bool,
    fixture_port: Option<u16>,
    fixture_shutdown_port: Option<u16>,
    import_config: Option<std::path::PathBuf>,
    force_import: bool,
}

fn parse_args() -> Result<Arguments> {
    let mut config_path = default_config_path();
    let mut restore_services = true;
    let mut fixture_port = None;
    let mut fixture_shutdown_port = None;
    let mut import_config = None;
    let mut force_import = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fixture-server" => {
                let value = args.next().context("--fixture-server requires a port")?;
                fixture_port = Some(value.parse().context("invalid fixture port")?);
            }
            "--fixture-shutdown" => {
                let value = args.next().context("--fixture-shutdown requires a port")?;
                fixture_shutdown_port = Some(value.parse().context("invalid fixture port")?);
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
        config_path,
        restore_services,
        fixture_port,
        fixture_shutdown_port,
        import_config,
        force_import,
    })
}

use std::sync::Arc;
