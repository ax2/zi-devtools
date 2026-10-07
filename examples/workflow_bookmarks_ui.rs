//! Separate seed/reopen/remove processes verify durable bookmarks through UI.
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
    mode: String,
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
            65 | 66 | 165 | 166 if self.mode == "reopen" => Some((
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
        let star = matches!(self.frame, 55 | 56) && self.mode != "reopen";
        let modal = matches!(self.frame, 100 | 101 | 195 | 196) && self.mode == "reopen";
        if star || modal {
            let pos = if star {
                self.app.preview_workflow_bookmark_position()
            } else {
                self.app
                    .preview_workflow_position(if self.frame < 190 { 0 } else { 1 })
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 55 | 100 | 195),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let end = if self.mode == "reopen" { 225 } else { 90 };
        if self.frame > end {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            if self.mode != "seed" {
                self.app.preview_workflow_bookmark_check(0);
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
        match (self.mode.as_str(), self.frame) {
            ("seed", 50) => self.app.preview_workflow_search_check(0),
            ("seed", 60) => self.app.preview_workflow_bookmark_check(1),
            ("reopen" | "remove", 50) | ("reopen", 150) => {
                self.app.preview_workflow_bookmark_check(2)
            }
            ("remove", 60) => self.app.preview_workflow_bookmark_check(3),
            ("reopen", 85 | 180) => self.app.preview_workflow_search_check(1),
            ("reopen", 115) => self.app.preview_workflow_search_check(2),
            ("reopen", 210) => self.app.preview_workflow_search_check(3),
            _ => {}
        }
        if self.frame == 50
            || self.frame == 60 && self.mode != "reopen"
            || self.frame == 85 && self.mode == "reopen"
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                image::save_buffer(
                    self.folder
                        .join(format!("bookmark-{}-{}.png", self.mode, self.captured)),
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
                "PASS native workflow bookmark {} light={}; durable metadata only, no scan; actual star/key/modal actions preserve table",
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
    let mode = std::env::args().nth(3).expect("seed/reopen/remove");
    assert!(matches!(mode.as_str(), "seed" | "reopen" | "remove"));
    eframe::run_native(
        "Workflow bookmarks acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                frame: 0,
                light,
                mode,
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
