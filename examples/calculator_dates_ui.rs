//! Real application UI acceptance using synthetic, credential-free input.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    frame: u32,
    started: Instant,
    folder: PathBuf,
    theme: String,
    captured: bool,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input.events.push(egui::Event::PointerGone);
        if matches!(self.frame, 20 | 21 | 80 | 81) {
            let pos = self.app.preview_date_position();
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame == 20 || self.frame == 80,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 95 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(
            self.started.elapsed() < Duration::from_secs(40),
            "native dates timeout"
        );
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.theme == "light" { 435 } else { 434 },
                self.folder.clone(),
            );
            self.app.preview_date_fixture();
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            10 => self.app.preview_date_check(0),
            35 => {
                self.app.preview_date_check(1);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            55 => self.app.preview_date_check(2),
            65 => self.app.preview_date_check(3),
            95 => {
                self.app.preview_date_check(4);
                assert!(self.captured, "screenshot absent");
                println!(
                    "PASS native date button clicks, weekday offset, stale suppression, exact day count, preserved ans/history; {} 980x640 logical",
                    self.theme
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!("dates-{}.png", self.theme)),
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured = true;
            }
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&folder).unwrap();
    let theme = std::env::args().nth(2).unwrap_or_else(|| "dark".into());
    let temp = std::env::temp_dir().join(format!("zi-date-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let result = eframe::run_native(
        "Date workbench acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let app = DevToolsApp::new(cc, config, false);
            Ok(Box::new(Preview {
                app,
                frame: 0,
                started: Instant::now(),
                folder,
                theme,
                captured: false,
            }))
        }),
    );
    std::fs::remove_dir_all(temp).unwrap();
    result
}
