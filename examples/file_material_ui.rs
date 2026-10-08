//! Native selected-file outputs and explicit handoff acceptance.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    light: bool,
    tick: u32,
    shots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 70 | 71) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: self.tick == 70,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let id = match self.tick {
            30 | 31 => Some(egui::Id::new(("file-report", 0usize))),
            60 | 61 | 80 | 81 => Some(egui::Id::new(("file-source", 0usize))),
            100 | 101 => Some(egui::Id::new("file-material-confirm")),
            130 | 131 => Some(egui::Id::new("file-material-read")),
            _ => None,
        };
        if let Some(id) = id {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .expect("button rendered");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "button visible: {rect:?}"
            );
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 30 | 60 | 80 | 100 | 130),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if self.tick == 0 {
            self.app
                .preview_file_material_prepare(ctx, self.light, self.folder.join("source.txt"));
        }
        self.app.update(ctx, frame);
        match self.tick {
            45 => {
                self.app.preview_file_material_check(0);
                self.app.preview_file_report_dispatch();
            }
            75 => self.app.preview_file_material_check(3),
            115 => self.app.preview_file_material_check(1),
            145 => self.app.preview_file_material_check(2),
            _ => {}
        }
        if matches!(self.tick, 45 | 65 | 85 | 145) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("material-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 165 {
            assert_eq!(self.shots, 4);
            println!(
                "PASS selected report, Escape cancels while preserving full target draft and source report, original-file confirmation, no automatic read/write, explicit encoding read; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("source.txt"), b"abc").unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "File material acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                tick: 0,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
