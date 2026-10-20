//! Native UI acceptance for an image workflow result entering the common handoff dialog.
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
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let position = match self.frame {
            20 | 21 => ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("image-report-send")))
                .map(|rect| rect.center()),
            50 | 51 => Some(self.app.preview_common_image_handoff_apply_position()),
            _ => None,
        };
        if let Some(position) = position {
            let pressed = matches!(self.frame, 20 | 50);
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
            "common image handoff UI fixture timed out"
        );
        if self.frame == 0 {
            self.app
                .preview_common_image_handoff_prepare(ctx, self.light);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        match self.frame {
            38 => {
                self.app.preview_common_image_handoff_check(false);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            68 => {
                self.app.preview_common_image_handoff_check(true);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            90 => {
                assert_eq!(self.captured, 2);
                println!(
                    "PASS common image result handoff: source bar and confirmation clicked; image stays in memory; independent image.info flow receives without auto-run; theme={}",
                    if self.light { "light" } else { "dark" }
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
                        "common-image-handoff-{}-{}.png",
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
        std::env::temp_dir().join(format!("zi-common-image-handoff-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();

    let result = eframe::run_native(
        "Common image handoff acceptance",
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
