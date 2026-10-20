//! Native image instances, disabled plugins, recorder settings, scroll, IME, relay and jobs.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    output: PathBuf,
    light: bool,
    mode: String,
    frame: u32,
    a: String,
    b: String,
    fingerprint: String,
    scroll: f32,
    generation: u64,
    shots: Vec<Arc<egui::ColorImage>>,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let back = match self.mode.as_str() {
            "extended" => match self.frame {
                30 | 31 | 40 | 41 | 90 | 91 | 120 | 121 | 180 | 181 | 184 | 185 | 188 | 189
                | 198 | 199 | 202 | 203 | 210 | 211 => Some(true),
                50 | 51 | 60 | 61 => Some(false),
                _ => None,
            },
            "tasks" => match self.frame {
                30 | 31 | 40 | 41 => Some(true),
                50 | 51 | 60 | 61 => Some(false),
                _ => None,
            },
            _ => match self.frame {
                40 | 41 | 70 | 71 => Some(true),
                50 | 51 => Some(false),
                _ => None,
            },
        };
        if self.mode == "extended" {
            let ime = match self.frame {
                195 => Some(egui::ImeEvent::Enabled),
                196 => Some(egui::ImeEvent::Preedit("tu".into())),
                202 => Some(egui::ImeEvent::Commit("json".into())),
                206 => Some(egui::ImeEvent::Disabled),
                _ => None,
            };
            if let Some(event) = ime {
                input.events.push(egui::Event::Ime(event));
            }
            if self.frame == 132 {
                let rect = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("history-library-scroll")))
                    .unwrap();
                input.events.push(egui::Event::PointerMoved(rect.center()));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -900.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if let Some(back) = back {
            input.modifiers = egui::Modifiers::ALT;
            input.events.push(egui::Event::Key {
                key: if back {
                    egui::Key::ArrowLeft
                } else {
                    egui::Key::ArrowRight
                },
                physical_key: None,
                pressed: self.frame.is_multiple_of(2),
                repeat: false,
                modifiers: egui::Modifiers::ALT,
            });
        }
        if self.mode == "handoff" && matches!(self.frame, 10 | 11 | 20 | 21) {
            let pos = self
                .app
                .preview_handoff_discovery_position(if self.frame < 20 { 0 } else { 1 });
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame.is_multiple_of(2),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(90));
        if self.frame == 0 {
            match self.mode.as_str() {
                "extended" => {
                    (self.a, self.b, self.fingerprint) =
                        self.app.preview_history_extended_prepare(ctx, self.light)
                }
                "tasks" => {
                    (self.a, self.b) = self.app.preview_history_tasks_prepare(ctx, self.light)
                }
                _ => self.app.preview_history_handoff_prepare(ctx, self.light),
            }
        }
        match self.mode.as_str() {
            "extended" => {
                if self.frame == 160 {
                    self.scroll = self.app.preview_history_extended_check(
                        4,
                        &self.a,
                        &self.b,
                        &self.fingerprint,
                        self.scroll,
                    );
                }
                let phase = match self.frame {
                    10 | 110 | 160 => Some(1),
                    20 => Some(2),
                    70 => Some(3),
                    80 | 170 => Some(4),
                    85 => Some(5),
                    100 => Some(6),
                    130 => Some(7),
                    166 => Some(8),
                    _ => None,
                };
                if let Some(phase) = phase {
                    self.app.preview_history_extended_step(phase, &self.b);
                }
            }
            "tasks" => {
                if self.frame == 10 {
                    self.app.preview_history_extended_step(1, "");
                }
                if self.frame == 20 {
                    self.app.preview_history_extended_step(2, &self.a);
                }
                if self.frame == 70 {
                    self.app.preview_history_task_cancel();
                }
            }
            _ => {
                if self.frame == 60 {
                    self.app.preview_history_task_page();
                }
            }
        }
        self.app.update(ctx, frame);
        let (capture, finish) = match self.mode.as_str() {
            "extended" => {
                let phase = match self.frame {
                    44 => Some(1),
                    64 | 94 => Some(2),
                    124 | 214 => Some(3),
                    150 => Some(4),
                    182 => Some(7),
                    192 => Some(5),
                    200 | 204 => Some(6),
                    _ => None,
                };
                if let Some(phase) = phase {
                    let scroll = self.app.preview_history_extended_check(
                        phase,
                        &self.a,
                        &self.b,
                        &self.fingerprint,
                        self.scroll,
                    );
                    if phase == 4 {
                        self.scroll = scroll;
                    }
                }
                (matches!(self.frame, 44 | 94 | 124 | 182 | 192 | 214), 218)
            }
            "tasks" => {
                if self.frame == 44 {
                    self.app.preview_history_task_check(&self.b, false);
                }
                if self.frame == 64 {
                    self.app.preview_history_task_check(&self.a, false);
                }
                if self.frame == 84 {
                    self.app.preview_history_task_check(&self.a, true);
                }
                (matches!(self.frame, 44 | 64 | 84), 88)
            }
            _ => {
                if self.frame == 28 {
                    self.generation = self.app.preview_history_handoff_check(false);
                }
                if matches!(self.frame, 44 | 54 | 74) {
                    assert_eq!(
                        self.app.preview_history_handoff_check(self.frame == 44),
                        self.generation,
                        "navigation must not rerun parse"
                    );
                }
                (matches!(self.frame, 28 | 44 | 54 | 74), 78)
            }
        };
        if capture {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                self.shots.push(image);
            }
        }
        if self.frame == finish {
            assert_eq!(
                self.shots.len(),
                match self.mode.as_str() {
                    "extended" => 6,
                    "tasks" => 3,
                    _ => 4,
                }
            );
            for (i, image) in self.shots.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("extended-{i}.png")),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            println!("PASS native history {} acceptance", self.mode);
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(25));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("output"));
    std::fs::create_dir_all(&output).unwrap();
    let root = std::env::temp_dir().join(format!("zi-history-extended-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let mode = std::env::args().nth(3).unwrap_or_else(|| "extended".into());
    eframe::run_native(
        "Extended history acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                output,
                light,
                mode,
                frame: 0,
                a: String::new(),
                b: String::new(),
                fingerprint: String::new(),
                scroll: 0.0,
                generation: 0,
                shots: Vec::new(),
                start: Instant::now(),
            }))
        }),
    )
}
