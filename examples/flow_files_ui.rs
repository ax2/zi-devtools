//! Native output/input review and file worker acceptance; picker selection is a fixture path.
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
    frame: u32,
    captured: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let index = match self.frame {
            20 | 21 => Some(3),
            50 | 51 => Some(1),
            120 | 121 | 180 | 181 => Some(6),
            142 | 143 => Some(8),
            200 | 201 => Some(7),
            250 | 251 | 360 | 361 => Some(9),
            320 | 321 => Some(11),
            390 | 391 => Some(10),
            _ => None,
        };
        if let Some(index) = index {
            let pos = self.app.preview_text_flow_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame % 2 == 0,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        let path = self.folder.join("result.json");
        if self.frame == 0 {
            self.app.preview_text_flow_prepare(ctx, self.light);
        }
        if matches!(self.frame, 115 | 175 | 245 | 355) {
            self.app.preview_text_flow_file_path(path.clone());
        }
        match self.frame {
            130 => self.app.preview_text_flow_file_check(&path, 0),
            160 => self.app.preview_text_flow_file_check(&path, 1),
            225 | 340 => self.app.preview_text_flow_file_check(&path, 2),
            300 => self.app.preview_text_flow_file_check(&path, 3),
            420 => self.app.preview_text_flow_file_check(&path, 4),
            _ => {}
        }
        self.app.update(ctx, frame);
        if matches!(self.frame, 135 | 225 | 300 | 420) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|c| c.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("files-{}.png", self.captured)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 440 {
            assert_eq!(self.captured, 4);
            println!(
                "PASS native full table result output review/cancel/review/save, actual file bytes; input read/review/cancel/read/confirm, original data draft preserved/no autorun; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    assert!(!folder.join("result.json").exists());
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Flow files acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                frame: 0,
                captured: 0,
                started: Instant::now(),
            }))
        }),
    )
}
