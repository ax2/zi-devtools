//! Controlled native search-arrow navigation, IME guard and explicit confirmation.
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
    started: Instant,
    scroll_started: Option<Instant>,
    scene_started: bool,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let ime = match self.frame {
            5 => Some(egui::ImeEvent::Enabled),
            6 => Some(egui::ImeEvent::Preedit("候选".into())),
            11 => Some(egui::ImeEvent::Preedit(String::new())),
            12 => Some(egui::ImeEvent::Commit(String::new())),
            13 => Some(egui::ImeEvent::Disabled),
            _ => None,
        };
        if let Some(event) = ime {
            input.events.push(egui::Event::Ime(event));
        }
        let keys = [
            (8, egui::Key::ArrowDown, false),
            (20, egui::Key::ArrowDown, false),
            (25, egui::Key::ArrowDown, false),
            (30, egui::Key::ArrowUp, false),
            (50, egui::Key::ArrowDown, false),
            (55, egui::Key::Tab, false),
            (60, egui::Key::K, true),
            (70, egui::Key::ArrowDown, false),
            (78, egui::Key::K, true),
            (80, egui::Key::A, true),
            (82, egui::Key::Backspace, false),
            (90, egui::Key::ArrowUp, false),
            (108, egui::Key::K, true),
            (110, egui::Key::A, true),
            (112, egui::Key::Backspace, false),
            (120, egui::Key::ArrowDown, false),
            (125, egui::Key::Tab, false),
            (130, egui::Key::Space, false),
        ];
        for (at, key, ctrl) in keys {
            if self.frame == at || self.frame == at + 1 {
                let modifiers = if ctrl {
                    egui::Modifiers {
                        command: true,
                        ..egui::Modifiers::CTRL
                    }
                } else {
                    egui::Modifiers::NONE
                };
                input.modifiers = modifiers;
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: self.frame == at,
                    repeat: false,
                    modifiers,
                });
            }
        }
        match self.frame {
            40 | 114 => input.events.push(egui::Event::Text("JSON 工具".into())),
            65 => input
                .events
                .push(egui::Event::Text(" unmatched987654".into())),
            _ => {}
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(40),
            "arrow fixture timeout"
        );
        if self.frame == 0 && !self.scene_started {
            self.app.preview_handoff_access_prepare(ctx, self.light);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.scene_started = true;
        }
        if !ctx.input(|input| input.raw.focused) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        self.app.update(ctx, frame);
        if self.frame == 90 {
            self.scroll_started = Some(Instant::now());
        }
        // Scroll animation requests immediate repaints, so ten rendered frames
        // can be much shorter than its 300 ms duration. Observe settled output.
        if self.frame == 100 && self.scroll_started.unwrap().elapsed() < Duration::from_millis(500)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        let phase = match self.frame {
            10 => Some(0),
            18 => Some(1),
            22 | 32 => Some(2),
            27 => Some(3),
            45 => Some(4),
            52 | 122 => Some(5),
            58 | 128 => Some(6),
            62 => Some(7),
            75 => Some(8),
            100 => Some(9),
            _ => None,
        };
        if let Some(phase) = phase {
            println!("CHECK arrow phase={phase}");
            self.app.preview_handoff_arrows_check(ctx, phase);
            if matches!(self.frame, 22 | 45 | 58 | 75 | 100) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    self.frame,
                )));
            }
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot {
                image, user_data, ..
            } = event
            {
                let pixels: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "arrows-{}.png",
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
        if self.frame == 140 {
            self.app.preview_handoff_keyboard_received();
            println!(
                "PASS arrows: recommendation order, query/empty result preservation, IME guard, last-row scroll, Tab/Space explicit receive; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}

pub fn run(folder: PathBuf, light: bool) -> eframe::Result<()> {
    std::fs::create_dir_all(&folder).unwrap();
    let temp = std::env::temp_dir().join(format!("zi-handoff-arrows-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Handoff arrows acceptance",
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
                frame: 0,
                started: Instant::now(),
                scroll_started: None,
                scene_started: false,
            }))
        }),
    )
}
