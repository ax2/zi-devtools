//! Actual Ctrl K/text/Enter and modal buttons, using isolated workflow files.
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
    captured: u32,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input.modifiers = egui::Modifiers::NONE;
        let key = match self.frame {
            30 | 31 | 130 | 131 => Some((
                egui::Key::K,
                egui::Modifiers::CTRL,
                matches!(self.frame, 30 | 130),
            )),
            138 | 139 => Some((egui::Key::A, egui::Modifiers::CTRL, self.frame == 138)),
            65 | 66 | 165 | 166 => Some((
                egui::Key::Enter,
                egui::Modifiers::NONE,
                matches!(self.frame, 65 | 165),
            )),
            _ => None,
        };
        if let Some((key, modifiers, pressed)) = key {
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        if matches!(self.frame, 40 | 140) {
            input.events.push(egui::Event::Text("每日资料清洗".into()));
        }
        if matches!(self.frame, 100 | 101 | 195 | 196) {
            let pos = self
                .app
                .preview_workflow_position(if self.frame < 190 { 0 } else { 1 });
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 100 | 195),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 225 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.light { 461 } else { 460 },
                self.folder.join("sample.txt"),
            );
        }
        self.app.update(ctx, frame);
        let phase = match self.frame {
            50 | 150 => Some(0),
            85 | 180 => Some(1),
            115 => Some(2),
            210 => Some(3),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app.preview_workflow_search_check(phase);
        }
        if matches!(self.frame, 50 | 85 | 115 | 210) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "search-{}-{}.png",
                        if self.light { "light" } else { "dark" },
                        self.captured
                    )),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 225 {
            assert_eq!(self.captured, 4);
            println!(
                "PASS native Ctrl K/text/Enter workflow search; cancel retains table/steps/preview; confirm only replaces steps without apply; light={}",
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
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Workflow search acceptance",
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
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
