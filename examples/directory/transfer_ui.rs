//! Actual directory tables: cancel, confirm, preserve existing work, mixed copy results.
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
    images: Vec<std::sync::Arc<egui::ColorImage>>,
    pointer: Option<egui::Pos2>,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        let offset = self.finished_at.map(|at| self.frame - at);
        let action_for = |frame, offset| match frame {
            20 | 40 => Some("directory-transfer-report"),
            30 => Some("directory-transfer-cancel"),
            50 => Some("directory-transfer-confirm"),
            90 => Some("directory-execute-copy"),
            _ => match offset {
                Some(10) => Some("directory-transfer-results"),
                Some(25) => Some("directory-transfer-confirm"),
                _ => None,
            },
        };
        let control = action_for(self.frame, offset);
        let upcoming = action_for(self.frame + 1, offset.map(|n| n + 1));
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("result review action");
            println!("transfer click {control} {rect:?}");
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
        } else if let Some(control) = upcoming {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("hover target");
            input.events.push(egui::Event::PointerMoved(rect.center()));
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
        assert!(self.started.elapsed() < Duration::from_secs(40));
        if self.frame == 0 {
            self.app
                .preview_directory_transfer_prepare(ctx, &self.root, self.light);
        }
        if self.frame == 70 {
            self.app.preview_directory_transfer_copy();
            fs::write(self.root.join("left/file-0204.txt"), b"edited 0204").unwrap();
        }
        self.app.update(ctx, frame);
        if self.frame == 15 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.frame > 90
            && self.finished_at.is_none()
            && self.app.preview_directory_result_review_ready()
        {
            self.finished_at = Some(self.frame);
        }
        let offset = self.finished_at.map(|at| self.frame - at);
        let phase = match self.frame {
            25 | 45 => Some(1),
            38 => Some(0),
            60 => Some(2),
            _ => match offset {
                Some(20) => Some(3),
                Some(35) => Some(4),
                _ => None,
            },
        };
        if let Some(phase) = phase {
            self.app.preview_directory_transfer_check(phase);
            if phase == 2 || phase == 4 {
                assert_eq!(ctx.data(|d| d.get_temp::<bool>(egui::Id::new("received-table-input-expanded"))), Some(false));
            }
            assert_eq!(
                fs::read_dir(self.root.join("output")).unwrap().count(),
                if phase >= 3 { 204 } else { 5 }
            );
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
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                // PNG compression can exceed egui's click-duration threshold.
                // Buffer captures until all pointer interactions have completed.
                self.images.push(image);
                self.shots += 1;
            }
        }
        if offset == Some(45) {
            assert_eq!(self.shots, 7);
            for (index, image) in self.images.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("transfer-{index}.png")),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            println!(
                "PASS actual table handoff: cancel creates no instance; 205 differences across pages; six mixed copy outcomes; old input/output preserved, new instances not autosaved, typed sizes/null preserved, no extra filesystem writes"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

pub fn run(output: PathBuf, light: bool) -> eframe::Result<()> {
    let root =
        std::env::temp_dir().join(format!("zi-directory-transfer-ui-{}", uuid::Uuid::new_v4()));
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
        "Directory table handoff",
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
                images: Vec::new(),
                pointer: None,
                started: Instant::now(),
            }))
        }),
    )
}
