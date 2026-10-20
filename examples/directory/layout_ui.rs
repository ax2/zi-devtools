//! Native directory layout regression with long paths and actual preview clicks.
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
            20 => Some("directory-select-visible"),
            30 => Some("directory-preview-plan"),
            _ => None,
        };
        if let Some(control) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(control)))
                .expect("layout action");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "action must be fully visible: {control} {rect:?}"
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
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app
                .preview_directory_layout_prepare(ctx, &self.root, self.light);
        }
        self.app.update(ctx, frame);
        if [15, 55].contains(&self.frame) {
            assert_eq!(
                fs::read_dir(self.root.join("output")).unwrap().count(),
                0,
                "preview cannot write destinations"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("layout-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if [18, 58].contains(&self.frame) {
            self.app
                .preview_directory_layout_check(ctx, self.frame == 58);
        }
        if self.frame == 70 {
            assert_eq!(self.shots, 2);
            println!(
                "PASS directory layout: long Unicode paths, horizontal content bounds, fully visible select/preview controls, actual preview clicks, no destination writes"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

pub fn run(output: PathBuf, light: bool, width: f32) -> eframe::Result<()> {
    let root = std::env::temp_dir()
        .join(format!("zi-directory-layout-ui-{}", uuid::Uuid::new_v4()))
        .join("长目录名称-教程素材与开发文档-layout-fixture");
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    let name = "很长的中文文件名称用于检查路径换行-abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz-abcdefghijklmnopqrstuvwxyz.txt";
    fs::write(root.join("left").join(name), b"layout copy source").unwrap();
    let config = root.join("services.yml");
    fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Directory adaptive layout acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([width, 1000.0]),
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
