//! Real tab/shortcut switching preserves the query and does not execute work.
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
            30 | 31 => Some((egui::Key::K, egui::Modifiers::CTRL, self.frame == 30)),
            70 | 71 => Some((egui::Key::Num2, egui::Modifiers::CTRL, self.frame == 70)),
            90 | 91 | 200 | 201 => Some((
                egui::Key::Num3,
                egui::Modifiers::CTRL,
                matches!(self.frame, 90 | 200),
            )),
            130 | 131 | 180 | 181 => Some((
                egui::Key::A,
                egui::Modifiers::CTRL,
                matches!(self.frame, 130 | 180),
            )),
            160 | 161 => Some((egui::Key::Num1, egui::Modifiers::CTRL, self.frame == 160)),
            175 | 176 => Some((egui::Key::Num4, egui::Modifiers::CTRL, self.frame == 175)),
            185 | 186 => Some((
                egui::Key::Backspace,
                egui::Modifiers::NONE,
                self.frame == 185,
            )),
            215 | 216 => Some((egui::Key::Enter, egui::Modifiers::NONE, self.frame == 215)),
            _ => None,
        };
        if let Some((key, mut modifiers, pressed)) = key {
            if modifiers.ctrl {
                modifiers.command = true;
            }
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        if self.frame == 40 {
            input.events.push(egui::Event::Text("每日资料清洗".into()));
        }
        if self.frame == 140 {
            input.events.push(egui::Event::Text("json".into()));
        }
        if matches!(self.frame, 55 | 56 | 110 | 111 | 245 | 246) {
            let pos = match self.frame {
                55 | 56 => self.app.preview_workflow_bookmark_position(),
                110 | 111 => self.app.preview_search_scope_position(3),
                _ => self.app.preview_workflow_position(0),
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 55 | 110 | 245),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 270 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.start.elapsed() < Duration::from_secs(35));
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.light { 461 } else { 460 },
                self.folder.join("sample.txt"),
            );
        }
        self.app.update(ctx, frame);
        let phase = match self.frame {
            80 => Some(1),
            100 => Some(2),
            120 => Some(3),
            150 => Some(4),
            170 => Some(5),
            178 => Some(4),
            210 => Some(6),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app.preview_search_scope_check(phase);
        }
        if self.frame == 230 {
            self.app.preview_workflow_search_check(1);
        }
        if self.frame == 260 {
            self.app.preview_workflow_search_check(2);
        }
        if matches!(self.frame, 80 | 120 | 210 | 230) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!("scopes-{}.png", self.captured)),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 270 {
            assert_eq!(self.captured, 4);
            println!(
                "PASS native search scopes: actual mouse/Ctrl1-4; query retained; tools/favorites/flows filtered; blank flow listing; Enter/cancel retains full table/preview; light={}",
                self.light
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
    eframe::run_native(
        "Search scopes acceptance",
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
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
