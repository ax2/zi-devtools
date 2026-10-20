//! Native UI check for selecting and previewing an intermediate workflow result.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;

struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    target: PathBuf,
    theme: bool,
    frame: u32,
    started: Instant,
    pointer: Option<egui::Pos2>,
    captured: bool,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let action = match self.frame {
            20 | 21 => Some((10, self.frame == 20)),
            60 | 61 => Some((0, self.frame == 60)),
            120 | 121 => Some((1, self.frame == 120)),
            _ => None,
        };
        if let Some((index, pressed)) = action {
            let pos = if pressed {
                let pos = if index == 10 {
                    self.app.preview_workflow_position(index)
                } else {
                    self.app.preview_workflow_output_position(index)
                };
                self.pointer = Some(pos);
                pos
            } else {
                self.pointer.take().expect("pointer press before release")
            };
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.frame > 135 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        assert!(
            self.started.elapsed() < Duration::from_secs(40),
            "fixture timeout"
        );
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.theme { 523 } else { 522 },
                self.folder.join("fixture.txt"),
            );
            self.app
                .preview_workflow_output_prepare(ctx, &self.target, false, self.theme);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            40 => self.app.preview_workflow_step_output_check(1, &self.target),
            80 => self.app.preview_workflow_step_output_check(2, &self.target),
            90 => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()))
            }
            130 => {
                self.app.preview_workflow_step_output_check(3, &self.target);
                assert!(!self.target.exists());
                assert!(self.captured);
                println!(
                    "PASS intermediate result CSV save preview and return without writing; {} theme",
                    if self.theme { "light" } else { "dark" }
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "workflow-step-output-{}.png",
                        if self.theme { "light" } else { "dark" }
                    )),
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
    let light = std::env::args().nth(2).as_deref() == Some("light");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let temp = std::env::temp_dir().join(format!("zi-step-output-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let temp = temp.canonicalize().unwrap();
    let temp_root = std::env::temp_dir().canonicalize().unwrap();
    assert_eq!(temp.parent(), Some(temp_root.as_path()));
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let cleanup_config = config.clone();
    let target = folder.join(if light {
        "never-created-light.csv"
    } else {
        "never-created-dark.csv"
    });
    let result = eframe::run_native(
        "Workflow step output acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config.clone(), false),
                folder,
                target,
                theme: light,
                frame: 0,
                started: Instant::now(),
                pointer: None,
                captured: false,
            }))
        }),
    );
    let _ = std::fs::remove_file(cleanup_config);
    std::fs::remove_dir_all(&temp).unwrap();
    result
}
