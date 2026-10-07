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
    handoff: bool,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input.events.push(egui::Event::PointerGone);
        let ordinary =
            matches!(self.frame, 20 | 21) || (!self.handoff && matches!(self.frame, 80 | 81));
        let handoff_click = self.handoff && matches!(self.frame, 40 | 41 | 60 | 61);
        if ordinary || handoff_click {
            let pos = if handoff_click {
                self.app.preview_date_handoff_position(self.frame < 50)
            } else {
                self.app.preview_date_position()
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: matches!(self.frame, 20 | 40 | 60 | 80),
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
            if self.handoff {
                self.app.preview_date_handoff_action(0);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        if self.handoff {
            if self.frame == 50 {
                self.app.preview_date_handoff_action(1);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if self.frame == 70 {
                self.app.preview_date_handoff_action(2);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if self.frame == 95 {
                assert!(self.captured);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            match self.frame {
                10 => self.app.preview_date_check(0),
                35 => {
                    self.app.preview_date_check(1);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
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
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(if self.handoff {
                        format!(
                            "date-handoff-{}-{}.png",
                            if self.frame < 65 {
                                "preview"
                            } else {
                                "calendar"
                            },
                            self.theme
                        )
                    } else {
                        format!("dates-{}.png", self.theme)
                    }),
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
    let handoff = std::env::args().nth(3).as_deref() == Some("handoff");
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
                handoff,
            }))
        }),
    );
    std::fs::remove_dir_all(temp).unwrap();
    result
}
