//! Native back/forward, exact data instances, library context and review gating.
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
    frame: u32,
    first: String,
    second: String,
    fingerprint: String,
    shots: Vec<Arc<egui::ColorImage>>,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let back = match self.frame {
            30 | 31 | 40 | 41 | 90 | 91 | 120 | 121 | 130 | 131 | 142 | 143 | 152 | 153 => {
                Some(true)
            }
            50 | 51 => Some(false),
            _ => None,
        };
        if let Some(back) = back {
            let modifiers = egui::Modifiers::ALT;
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key: if back {
                    egui::Key::ArrowLeft
                } else {
                    egui::Key::ArrowRight
                },
                physical_key: None,
                pressed: self.frame.is_multiple_of(2),
                repeat: false,
                modifiers,
            });
        }
        if matches!(self.frame, 60 | 61) {
            let pos = self.app.preview_sidebar_position("history-forward");
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame == 60,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(100));
        if self.frame == 0 {
            (self.first, self.fingerprint) = self.app.preview_history_prepare(ctx, self.light);
        }
        let phase = match self.frame {
            10 => Some(1),
            20 => Some(2),
            70 => Some(3),
            80 => Some(4),
            100 => Some(5),
            110 => Some(6),
            140 => Some(7),
            148 => Some(8),
            _ => None,
        };
        if let Some(phase) = phase {
            let id = self.app.preview_history_step(phase, &self.second);
            if phase == 2 {
                self.second = id;
            }
        }
        self.app.update(ctx, frame);
        let check = match self.frame {
            34 | 54 | 134 => Some(1),
            44 | 156 => Some(2),
            64 => Some(3),
            94 | 124 => Some(4),
            104 => Some(5),
            144 => Some(6),
            _ => None,
        };
        if let Some(check) = check {
            self.app
                .preview_history_check(check, &self.first, &self.second, &self.fingerprint);
        }
        if matches!(self.frame, 44 | 64 | 94 | 134 | 144 | 156) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                self.shots.push(image);
            }
        }
        if self.frame == 162 {
            assert_eq!(self.shots.len(), 6);
            for (i, image) in self.shots.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("history-{i}.png")),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            println!(
                "PASS native history keys/clicks, exact instances, closed target skipping, query restore, branch truncation and review blocking"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("output"));
    std::fs::create_dir_all(&output).unwrap();
    let root = std::env::temp_dir().join(format!("zi-history-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Navigation history acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                output,
                light,
                frame: 0,
                first: String::new(),
                second: String::new(),
                fingerprint: String::new(),
                shots: Vec::new(),
                start: Instant::now(),
            }))
        }),
    )
}
