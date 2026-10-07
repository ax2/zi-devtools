//! Real native unified search/bookmark/reopen/review controls with fixture data.
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
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let key = match self.frame {
            30 | 31 | 130 | 131 => Some((egui::Key::K, egui::Modifiers::CTRL, self.frame % 2 == 0)),
            138 | 139 => Some((egui::Key::A, egui::Modifiers::CTRL, self.frame == 138)),
            70 | 71 | 165 | 166 => Some((
                egui::Key::Enter,
                egui::Modifiers::NONE,
                matches!(self.frame, 70 | 165),
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
            input.events.push(egui::Event::Text("教程编码".into()));
        }
        let pos = match self.frame {
            55 | 56 => Some(self.app.preview_workflow_bookmark_position()),
            100 | 101 => Some(self.app.preview_text_flow_position(13)),
            200 | 201 => Some(self.app.preview_text_flow_position(12)),
            _ => None,
        };
        if let Some(pos) = pos {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 55 | 100 | 200),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app
                .preview_recipe_library_prepare(ctx, self.light, self.folder.join("recipes"));
        }
        self.app.update(ctx, frame);
        match self.frame {
            50 => self.app.preview_recipe_library_check(0),
            60 => self.app.preview_recipe_library_check(1),
            85 | 190 => self.app.preview_recipe_library_check(2),
            115 => self.app.preview_recipe_library_check(3),
            220 => self.app.preview_recipe_library_check(4),
            _ => {}
        }
        if matches!(self.frame, 50 | 60 | 85 | 220) {
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
                    self.folder.join(format!("library-{}.png", self.captured)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 240 {
            assert_eq!(self.captured, 4);
            println!(
                "PASS unified native CtrlK/tool recipe search/star persisted/open/review/return/reopen/confirm; recent persisted and canonical tool ID; source and table definition kept/no autorun; light={}",
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
    std::fs::create_dir_all(folder.join("recipes")).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    std::fs::write(
        folder.join("recipes/教程编码.json"),
        br#"{"version":1,"steps":[{"action":"base64.encode","version":1}]}"#,
    )
    .unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Recipe library acceptance",
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
                start: Instant::now(),
            }))
        }),
    )
}
