//! Actual native surface, synthetic keyboard interaction, both themes.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    frame: u32,
    folder: PathBuf,
    light: bool,
    captured: u32,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.frame, 20 | 60 | 110) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            });
        }
        if self.frame == 130 {
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(800.0, 520.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -240.0),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 160 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app.preview_text_plugin(ctx, self.light, 0);
        }
        if self.frame == 50 {
            self.app.preview_text_plugin(ctx, self.light, 2);
        }
        if self.frame == 100 {
            self.app.preview_text_plugin(ctx, self.light, 4);
        }
        self.app.update(ctx, frame);
        if self.frame == 145 {
            if let Some(layer) = ctx.layer_id_at(egui::pos2(600.0, 400.0)) {
                ctx.graphics(|layers| {
                    if let Some(list) = layers.get(layer) {
                        for item in list.all_entries() {
                            let rect = item.shape.visual_bounding_rect();
                            if rect.width() < 5.0 && rect.height() > 120.0 {
                                println!(
                                    "PAINT thin tall shape rect={rect:?} clip={:?} shape={:?}",
                                    item.clip_rect, item.shape
                                );
                            }
                        }
                    }
                });
            }
        }
        if matches!(self.frame, 35 | 75 | 120 | 145) {
            self.app.preview_text_plugin(
                ctx,
                self.light,
                match self.frame {
                    35 => 1,
                    75 => 3,
                    _ => 5,
                },
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "native-text-{}-{}.png",
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
        if self.frame == 160 {
            assert_eq!(self.captured, 4);
            println!(
                "PASS native plugin-compatible JSON Ctrl+Enter rejects duplicate keys then returns sorted JSON; 10002-byte result and synthetic wheel screenshots captured; light={}",
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
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let temp = std::env::temp_dir().join(format!("zi-text-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let result = eframe::run_native(
        "Text plugin compatibility",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                frame: 0,
                folder,
                light,
                captured: 0,
                start: Instant::now(),
            }))
        }),
    );
    std::fs::remove_dir_all(temp).unwrap();
    result
}
