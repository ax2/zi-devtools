//! Actual mixed-result copy followed by native result filtering and paging.
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
    finished_at: Option<u32>,
    shots: u32,
    pointer: Option<egui::Pos2>,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        let offset = self.finished_at.map(|at| self.frame - at);
        let control = if self.frame == 20 {
            Some("directory-execute-copy")
        } else {
            match offset {
                Some(10) => Some("directory-review-conflicts"),
                Some(25) => Some("directory-review-failed"),
                Some(40) => Some("directory-review-all"),
                Some(45 | 55) => Some("directory-copy-results-next"),
                Some(75) => Some("directory-review-query"),
                _ => None,
            }
        };
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("result review action");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "result action not fully visible: {control} {rect:?}"
            );
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
        if offset == Some(77) {
            input.events.push(egui::Event::Text("file-0204".into()));
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(40));
        if self.frame == 0 {
            self.app
                .preview_directory_result_review_prepare(ctx, &self.root, self.light);
            fs::write(self.root.join("left/file-0204.txt"), b"edited 0204").unwrap();
        }
        self.app.update(ctx, frame);
        if self.frame > 20
            && self.finished_at.is_none()
            && self.app.preview_directory_result_review_ready()
        {
            self.finished_at = Some(self.frame);
        }
        let offset = self.finished_at.map(|at| self.frame - at);
        let step = match offset {
            Some(5) => Some(0),
            Some(20) => Some(1),
            Some(35) => Some(2),
            Some(65) => Some(3),
            Some(85) => Some(4),
            _ => None,
        };
        if let Some(step) = step {
            self.app.preview_directory_result_review_check(ctx, step);
            assert_eq!(fs::read_dir(self.root.join("output")).unwrap().count(), 204);
            for index in 0..5 {
                assert_eq!(
                    fs::read(
                        self.root
                            .join("output")
                            .join(format!("file-{index:04}.txt"))
                    )
                    .unwrap(),
                    format!("keep {index:04}").as_bytes()
                );
            }
            assert!(!self.root.join("output/file-0204.txt").exists());
            assert_eq!(
                fs::read(self.root.join("output/file-0005.txt")).unwrap(),
                b"source 0005"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("results-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if offset == Some(100) {
            assert_eq!(self.shots, 5);
            println!(
                "PASS actual mixed copy: 199 successes, 5 preserved conflicts, 1 changed-source failure; old plan collapsed, actual result filters, last page and query, no automatic retry or extra writes"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

pub fn run(output: PathBuf, light: bool) -> eframe::Result<()> {
    let root =
        std::env::temp_dir().join(format!("zi-directory-review-ui-{}", uuid::Uuid::new_v4()));
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    for index in 0..205 {
        fs::write(
            root.join("left").join(format!("file-{index:04}.txt")),
            format!("source {index:04}"),
        )
        .unwrap();
        if index < 5 {
            fs::write(
                root.join("output").join(format!("file-{index:04}.txt")),
                format!("keep {index:04}"),
            )
            .unwrap();
        }
    }
    let config = root.join("services.yml");
    fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Directory copy result review",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 1000.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                root,
                output,
                light,
                frame: 0,
                finished_at: None,
                shots: 0,
                pointer: None,
                started: Instant::now(),
            }))
        }),
    )
}
