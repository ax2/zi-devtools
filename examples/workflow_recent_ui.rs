//! Seed and fresh reopen/remove processes, actual native input and own files.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    mode: String,
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
                egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
                matches!(self.frame, 30 | 130),
            )),
            65 | 66 => Some((egui::Key::Enter, egui::Modifiers::NONE, self.frame == 65)),
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
        if matches!(self.frame, 100 | 101 | 165 | 166) {
            let pos = if self.frame < 120 {
                self.app.preview_workflow_position(0)
            } else {
                self.app.preview_workflow_recent_position()
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 100 | 165),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let end = if self.mode == "seed" { 120 } else { 200 };
        if self.frame > end {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            if self.mode != "seed" {
                self.app.preview_workflow_recent_check(0);
            }
            self.app.preview_scene(
                ctx,
                if self.mode == "seed" {
                    if self.light { 461 } else { 460 }
                } else if self.light {
                    299
                } else {
                    298
                },
                self.folder.join("sample.txt"),
            );
        }
        self.app.update(ctx, frame);
        match self.frame {
            50 | 150 if self.mode != "seed" => self.app.preview_workflow_recent_check(3),
            85 => self.app.preview_workflow_recent_check(1),
            115 => self.app.preview_workflow_recent_check(2),
            180 if self.mode != "seed" => self.app.preview_workflow_recent_check(4),
            _ => {}
        }
        if self.mode == "seed" && matches!(self.frame, 85 | 115)
            || self.mode != "seed" && matches!(self.frame, 50 | 180)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder
                        .join(format!("recent-{}-{}.png", self.mode, self.captured)),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == end {
            assert_eq!(self.captured, 2);
            println!(
                "PASS native workflow recent {} light={}: real successful load/persistence/reopen; no automatic scan; Enter/cancel retains full table/preview; remove retains file",
                self.mode, self.light
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    std::fs::write(folder.join("sample.txt"), b"fixture").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let mode = std::env::args().nth(3).expect("seed/reopen");
    assert!(matches!(mode.as_str(), "seed" | "reopen"));
    eframe::run_native(
        "Workflow recent acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                mode,
                frame: 0,
                light,
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
