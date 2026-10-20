//! Real copy paused after one staged chunk to exercise live progress and cancellation.
use eframe::egui;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;

struct Preview {
    app: DevToolsApp,
    root: PathBuf,
    output: PathBuf,
    light: bool,
    frame: u32,
    shots: u32,
    pointer: Option<egui::Pos2>,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        let control = match self.frame {
            20 => Some("directory-task-open-1"),
            40 => Some("directory-copy-cancel"),
            _ => None,
        };
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("visible copy action");
            assert!(rect.is_positive() && ctx.screen_rect().contains(rect.center()));
            let pos = rect.center();
            self.pointer = Some(pos);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        } else if let Some(pos) = self.pointer.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app
                .preview_directory_progress_prepare(ctx, &self.root, self.light);
        }
        self.app.update(ctx, frame);
        match self.frame {
            15 => self.app.preview_directory_progress_check(true, false),
            30 => self.app.preview_directory_progress_check(true, true),
            60 => {
                self.app.preview_directory_progress_check(false, true);
                assert!(!self.root.join("output/a.txt").exists());
                assert_eq!(
                    fs::read(self.root.join("output/b.txt")).unwrap(),
                    b"existing destination"
                );
                assert!(fs::read_dir(self.root.join("output")).unwrap().all(|e| {
                    !e.unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".zi-local-")
                }));
            }
            _ => {}
        }
        if [15, 30, 60].contains(&self.frame) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("progress-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.frame == 75 {
            assert_eq!(self.shots, 3);
            println!(
                "PASS real copy: staged 65536/131072 bytes and zero committed, 25 percent processing on task/page, actual cancel click removes partial temporary file and preserves existing target"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}
pub fn run(output: PathBuf, light: bool) -> eframe::Result<()> {
    let root =
        std::env::temp_dir().join(format!("zi-directory-progress-ui-{}", uuid::Uuid::new_v4()));
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    fs::write(root.join("left/a.txt"), vec![7u8; 131072]).unwrap();
    fs::write(root.join("left/b.txt"), b"source b").unwrap();
    fs::write(root.join("output/b.txt"), b"existing destination").unwrap();
    let config = root.join("services.yml");
    fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Directory copy progress acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                root,
                output,
                light,
                frame: 0,
                shots: 0,
                pointer: None,
                started: Instant::now(),
            }))
        }),
    )
}
