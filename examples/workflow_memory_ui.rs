//! Real eframe buttons with isolated fixture preferences and no user files.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;

struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    frame: u32,
    light: bool,
    screenshots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.frame, 30 | 31 | 65 | 66) {
            let position = self
                .app
                .preview_workflow_memory_position(if self.frame < 60 { 0 } else { 1 });
            input.events.push(egui::Event::PointerMoved(position));
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 30 | 65),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 100 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.started.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.light { 461 } else { 460 },
                self.folder.join("sample.txt"),
            );
        }
        self.app.update(ctx, frame);
        if self.frame == 45 {
            self.app.preview_workflow_memory_check(true);
        }
        if self.frame == 80 {
            self.app.preview_workflow_memory_check(false);
        }
        if matches!(self.frame, 45 | 80) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "memory-{}-{}.png",
                        if self.light { "light" } else { "dark" },
                        self.screenshots
                    )),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.screenshots += 1;
            }
        }
        if self.frame == 100 {
            assert_eq!(self.screenshots, 2);
            println!(
                "PASS native remember/forget actual button clicks, isolated preferences roundtrip, files/list/table/steps/preview preserved; light={}",
                self.light
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    std::fs::write(folder.join("sample.txt"), b"fixture").unwrap();
    // Each theme gets its own fixture directory and preference file.
    assert!(!folder.join("ui-preferences.json").exists());
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Workflow folder memory acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                frame: 0,
                light,
                screenshots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
