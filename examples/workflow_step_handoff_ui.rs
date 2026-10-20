//! Native acceptance of intermediate table handoff through the shared confirmation dialog.
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
    captured: u8,
    started: Instant,
    pointer: Option<egui::Pos2>,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let action = match self.frame {
            20 | 21 => Some((false, self.frame == 20)),
            80 | 81 => Some((true, self.frame == 80)),
            _ => None,
        };
        if let Some((apply, pressed)) = action {
            let position = if pressed {
                let position = if apply {
                    self.app.preview_text_flow_table_apply_position()
                } else {
                    self.app.preview_workflow_step_send_position()
                };
                self.pointer = Some(position);
                position
            } else {
                self.pointer.take().expect("pointer press before release")
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
            self.started.elapsed() < Duration::from_secs(40),
            "workflow handoff UI fixture timed out"
        );
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.light { 523 } else { 522 },
                self.folder.join("fixture.csv"),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            55 => {
                self.app.preview_workflow_table_handoff_check(0);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            110 => {
                self.app.preview_workflow_table_handoff_check(1);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            140 => {
                assert_eq!(
                    self.captured, 2,
                    "review and received table screenshots captured"
                );
                println!(
                    "PASS workflow step typed handoff: versioned source review, explicit receive into a new data instance, both themes={}",
                    self.light
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                        "workflow-step-handoff-{}-{}.png",
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
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();

    let temp =
        std::env::temp_dir().join(format!("zi-workflow-handoff-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let temp = temp.canonicalize().unwrap();
    let temp_root = std::env::temp_dir().canonicalize().unwrap();
    assert_eq!(temp.parent(), Some(temp_root.as_path()));
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();

    let result = eframe::run_native(
        "Workflow step handoff acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                folder,
                light,
                frame: 0,
                captured: 0,
                started: Instant::now(),
                pointer: None,
            }))
        }),
    );
    let _ = std::fs::remove_dir_all(&temp);
    result
}
