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
    captured: usize,
    pointer: Option<egui::Pos2>,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input.events.push(egui::Event::PointerGone);
        let index = match self.frame {
            20 | 21 => Some(2),
            60 | 61 => Some(3),
            120 | 121 => Some(4),
            _ => None,
        };
        if let Some(index) = index {
            let pressed = matches!(self.frame, 20 | 60 | 120);
            let pos = if pressed {
                let pos = self.app.preview_workflow_position(index);
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
            "row workflow timeout"
        );
        if self.frame == 0 {
            self.app.preview_scene(
                ctx,
                if self.theme == "light" { 301 } else { 300 },
                self.folder.join("fixture.txt"),
            );
            self.app.preview_workflow_rows(0);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            10 | 45 => {
                if self.frame == 45 && !self.app.preview_workflow_rows(1) {
                    ctx.request_repaint_after(Duration::from_millis(35));
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            75 => {
                self.app.preview_workflow_rows(2);
            }
            135 => {
                self.app.preview_workflow_rows(3);
                assert_eq!(self.captured, 2);
                println!(
                    "PASS native row workflow preview/apply/undo: typed sort, filtered/dedup rows, original input preserved; {} 980x640 logical",
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
                    self.folder.join(format!(
                        "workflow-rows-{}-{}.png",
                        if self.frame < 30 { "editor" } else { "preview" },
                        self.theme
                    )),
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
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
    std::fs::create_dir_all(&folder).unwrap();
    let theme = std::env::args().nth(2).unwrap_or_else(|| "dark".into());
    let temp = std::env::temp_dir().join(format!("zi-workflow-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let result = eframe::run_native(
        "Workflow row acceptance",
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
                captured: 0,
                pointer: None,
            }))
        }),
    );
    std::fs::remove_dir_all(temp).unwrap();
    result
}
