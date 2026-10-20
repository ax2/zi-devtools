//! Native task-center routing: controlled waiting worker, then real temporary copy.
use eframe::egui;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::{app::DevToolsApp, tasks::Phase};

#[path = "directory/layout_ui.rs"]
mod layout;
#[path = "directory/pagination_ui.rs"]
mod pagination;
#[path = "directory/progress_ui.rs"]
mod progress;
#[path = "directory/result_review_ui.rs"]
mod result_review;
#[path = "directory/transfer_ui.rs"]
mod transfer;

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
            20 => Some("directory-task-cancel-1"),
            70 => Some("directory-task-open-2"),
            _ => None,
        };
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("task action visible");
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
                .preview_directory_tasks_prepare(ctx, &self.root, self.light);
        }
        if self.frame == 45 {
            self.app.preview_directory_tasks_copy();
        }
        self.app.update(ctx, frame);
        match self.frame {
            15 => self
                .app
                .preview_directory_tasks_check(Phase::Running, false),
            38 => {
                self.app
                    .preview_directory_tasks_check(Phase::Cancelled, false);
                assert!(!self.root.join("output/a.txt").exists());
            }
            65 => self.app.preview_directory_tasks_check(Phase::Failed, false),
            80 => {
                self.app.preview_directory_tasks_check(Phase::Failed, true);
                assert_eq!(
                    fs::read(self.root.join("output/a.txt")).unwrap(),
                    b"source a"
                );
                assert_eq!(
                    fs::read(self.root.join("output/b.txt")).unwrap(),
                    b"existing destination"
                );
            }
            _ => {}
        }
        if [15, 38, 65, 80].contains(&self.frame) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("tasks-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.frame == 95 {
            assert_eq!(self.shots, 4);
            println!(
                "PASS directory tasks: actual cancel click, preview cancelled without writes, real copy completes on task page, conflict retained, actual open click returns with results; four screenshots"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence folder"));
    fs::create_dir_all(&output).unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    if std::env::args().nth(3).as_deref() == Some("progress") {
        return progress::run(output, light);
    }
    if std::env::args().nth(3).as_deref() == Some("transfer") {
        return transfer::run(output, light);
    }
    if std::env::args().nth(3).as_deref() == Some("results") {
        return result_review::run(output, light);
    }
    if std::env::args().nth(3).as_deref() == Some("pagination") {
        return pagination::run(output, light);
    }
    if let Some(mode) = std::env::args().nth(3)
        && matches!(
            mode.as_str(),
            "layout-compact" | "layout-narrow" | "layout-wide"
        )
    {
        let width = match mode.as_str() {
            "layout-compact" => 760.0,
            "layout-narrow" => 900.0,
            _ => 1200.0,
        };
        return layout::run(output, light, width);
    }
    let root = std::env::temp_dir().join(format!("zi-directory-task-ui-{}", uuid::Uuid::new_v4()));
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    fs::write(root.join("left/a.txt"), b"source a").unwrap();
    fs::write(root.join("left/b.txt"), b"source b").unwrap();
    fs::write(root.join("output/b.txt"), b"existing destination").unwrap();
    let config = root.join("services.yml");
    fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Directory background task acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 850.0]),
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
