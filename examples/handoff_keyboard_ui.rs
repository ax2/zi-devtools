//! Native keyboard regression with disposable configuration and controlled input.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
#[path = "handoff/arrow_ui.rs"]
mod arrows;

struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    light: bool,
    escape_only: bool,
    frame: u32,
    started: Instant,
    pointer: Option<egui::Pos2>,
    confirm_at: Option<u32>,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        if matches!(self.frame, 20 | 21) {
            let pressed = self.frame == 20;
            let position = if pressed {
                let rect = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("handoff-category-proof")))
                    .unwrap();
                assert!(ctx.screen_rect().contains(rect.center()));
                self.pointer = Some(rect.center());
                rect.center()
            } else {
                self.pointer.take().unwrap()
            };
            input.events.push(egui::Event::PointerMoved(position));
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut key = match self.frame {
            30 | 31 | 50 | 51 | 120 | 121 => Some((egui::Key::Escape, false)),
            80 | 81 => Some((egui::Key::Tab, false)),
            88 | 89 => Some((egui::Key::K, true)),
            98 | 99 => Some((egui::Key::Enter, true)),
            106 | 107 => Some((egui::Key::Enter, false)),
            _ => None,
        };
        if self.frame >= 150 {
            assert!(self.frame < 300, "Tab must reach the confirmation button");
            if let Some(at) = self.confirm_at {
                if self.frame == at + 1 {
                    key = Some((egui::Key::Space, false));
                }
            } else if self.frame.is_multiple_of(2) {
                let button = ctx
                    .data(|d| d.get_temp::<egui::Id>(egui::Id::new("handoff-apply-focus-proof")))
                    .unwrap();
                if ctx.memory(|m| m.has_focus(button)) {
                    self.confirm_at = Some(self.frame);
                    key = Some((egui::Key::Space, false));
                } else {
                    key = Some((egui::Key::Tab, false));
                }
            } else {
                key = Some((egui::Key::Tab, false));
            }
        }
        if let Some((key, ctrl)) = key {
            let modifiers = if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: self.frame.is_multiple_of(2),
                repeat: false,
                modifiers,
            });
        }
        if matches!(self.frame, 70 | 140) {
            input.events.push(egui::Event::Text("JSON 工具".into()));
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(45),
            "keyboard fixture timeout"
        );
        if self.frame == 0 {
            self.app.preview_handoff_access_prepare(ctx, self.light);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if matches!(self.frame, 60 | 130) {
            self.app.preview_handoff_keyboard_reopen();
        }
        self.app.update(ctx, frame);
        let check = match self.frame {
            25 => Some((Some(""), None, true)),
            40 => Some((Some(""), None, false)),
            55 | 125 => Some((None, None, false)),
            65 | 135 => Some((Some(""), Some(true), false)),
            75 | 94 => Some((Some("JSON 工具"), Some(true), false)),
            84 => Some((Some("JSON 工具"), Some(false), false)),
            102 | 110 | 145 => Some((Some("JSON 工具"), None, false)),
            _ => None,
        };
        if let Some((query, focus, popup)) = check {
            println!("CHECK frame={}", self.frame);
            self.app
                .preview_handoff_keyboard_check(ctx, query, focus, popup);
            if matches!(self.frame, 25 | 40 | 75 | 94 | 145) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    self.frame,
                )));
            }
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot {
                image, user_data, ..
            } = event
            {
                let pixels: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "keyboard-{}.png",
                        user_data.data.unwrap().downcast_ref::<u32>().unwrap()
                    )),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
        if (self.escape_only && self.frame == 58)
            || self.confirm_at.is_some_and(|at| self.frame == at + 5)
        {
            if !self.escape_only {
                self.app.preview_handoff_keyboard_received();
                println!(
                    "PASS Tab/Space explicit receive: one persisted JSON visit; source preserved; no conversion"
                );
            }
            println!(
                "PASS keyboard: escape_only={} light={}",
                self.escape_only, self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}

fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("output folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    if std::env::args().nth(3).as_deref() == Some("arrows") {
        return arrows::run(folder, light);
    }
    let escape_only = std::env::args().nth(3).as_deref() == Some("escape");
    let temp = std::env::temp_dir().join(format!("zi-handoff-keyboard-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Handoff keyboard acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([980.0, if light { 740.0 } else { 640.0 }]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                folder,
                light,
                escape_only,
                frame: 0,
                started: Instant::now(),
                pointer: None,
                confirm_at: None,
            }))
        }),
    )
}
