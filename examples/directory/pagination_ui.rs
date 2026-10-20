//! Real 205-file fixture: browse, retain cross-page selection, review, and filter.
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
            20 | 40 => Some("directory-results-next"),
            30 | 50 => Some("directory-select-visible"),
            60 => Some("directory-results-previous"),
            70 => Some("directory-preview-plan"),
            85 => Some("directory-copy-directories-next"),
            100 => Some("directory-copy-files-next"),
            120 => Some("directory-filter"),
            _ => None,
        };
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("pagination control");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "pagination control not fully visible: {control} {rect:?}"
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
        if self.frame == 122 {
            input.events.push(egui::Event::Text("file-0204.txt".into()));
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app
                .preview_directory_layout_prepare(ctx, &self.root, self.light);
        }
        self.app.update(ctx, frame);
        let step = match self.frame {
            15 => Some(0),
            55 => Some(1),
            110 => Some(2),
            140 => Some(3),
            _ => None,
        };
        if let Some(step) = step {
            self.app.preview_directory_pagination_check(ctx, step);
            assert_eq!(
                fs::read_dir(self.root.join("output")).unwrap().count(),
                0,
                "browse/preview must not write files"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("pagination-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.frame == 155 {
            assert_eq!(self.shots, 4);
            println!(
                "PASS actual pagination: 205 files, last 5 reachable, 105 selections retained across pages, second directory/file preview pages reviewed, actual filter input resets result page, zero destination writes"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

pub fn run(output: PathBuf, light: bool) -> eframe::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "zi-directory-pagination-ui-{}",
        uuid::Uuid::new_v4()
    ));
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    for index in 0..205 {
        let group = format!("group-{index:04}");
        for side in ["left", "right"] {
            fs::create_dir_all(root.join(side).join(&group)).unwrap();
        }
        fs::write(
            root.join("left")
                .join(group)
                .join(format!("file-{index:04}.txt")),
            format!("fixture {index}"),
        )
        .unwrap();
    }
    let config = root.join("services.yml");
    fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Directory pagination acceptance",
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
                shots: 0,
                pointer: None,
                started: Instant::now(),
            }))
        }),
    )
}
