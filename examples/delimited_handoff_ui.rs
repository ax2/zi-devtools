//! Native CSV/TSV recommendation, cancellation and new-instance handoff fixture.
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
    tsv: bool,
    frame: u32,
    captured: u8,
    started: Instant,
    pointer: Option<egui::Pos2>,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        // Deterministic fixture input: desktop mouse/keyboard activity must not
        // be mixed with the explicitly injected UI actions. Keep render evidence.
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        if matches!(self.frame, 40 | 41) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: self.frame == 40,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let position = match self.frame {
            20 | 21 | 70 | 71 => Some(self.app.preview_handoff_discovery_position(0)),
            100 | 101 => Some(self.app.preview_handoff_discovery_position(1)),
            _ => None,
        };
        if let Some(position) = position {
            let pressed = matches!(self.frame, 20 | 70 | 100);
            if pressed {
                self.pointer = Some(position);
            }
            let position = if pressed {
                position
            } else {
                self.pointer.take().expect("press precedes release")
            };
            input.events.push(egui::Event::PointerMoved(position));
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(35),
            "delimited handoff UI fixture timed out"
        );
        if self.frame == 0 {
            self.app
                .preview_delimited_handoff_prepare(ctx, self.light, self.tsv);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            10 => {
                self.app.preview_delimited_handoff_check(0, self.tsv);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            32 => {
                self.app.preview_delimited_handoff_check(1, self.tsv);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            52 => {
                self.app.preview_delimited_handoff_check(2, self.tsv);
                self.app
                    .preview_delimited_handoff_prepare(ctx, self.light, self.tsv);
            }
            88 => self.app.preview_delimited_handoff_check(1, self.tsv),
            125 => {
                self.app.preview_delimited_handoff_check(3, self.tsv);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            145 => {
                assert_eq!(self.captured, 3);
                println!(
                    "PASS recommendation click, Escape cancellation, explicit receive, two parsed rows, original draft/export and source preserved: tsv={} light={}",
                    self.tsv, self.light
                );
                self.app.preview_tray_workflow_finish(ctx);
            }
            _ => {}
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "delimited-handoff-{}-{}.png",
                        if self.light { "light" } else { "dark" },
                        self.captured + 1
                    )),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}

fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let tsv = std::env::args().nth(3).as_deref() == Some("tsv");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let temp = std::env::temp_dir().join(format!("zi-delimited-handoff-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();

    // The disposable empty service config is retained for inspection.
    eframe::run_native(
        "Delimited handoff acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                folder,
                light,
                tsv,
                frame: 0,
                captured: 0,
                started: Instant::now(),
                pointer: None,
            }))
        }),
    )
}
