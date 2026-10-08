//! Native report generation, cancelled transfer, JSON acceptance and unsaved memo receipt.
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
    batch: bool,
    tick: u32,
    shots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 85 | 86) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: self.tick == 85,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let id = match self.tick {
            30 | 31 => Some("image-report-generate"),
            65 | 66 | 100 | 101 => Some("image-report-send"),
            110 | 111 | 145 | 146 => Some("handoff-apply"),
            _ => None,
        };
        if let Some(id) = id {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(id)))
                .expect("rendered control");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "visible {id}: {rect:?}"
            );
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 30 | 65 | 100 | 110 | 145),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if self.tick == 0 {
            if self.batch {
                self.app.preview_batch_report_prepare(ctx, self.light);
            } else {
                self.app.preview_image_report_prepare(ctx, self.light);
            }
        }
        if self.tick == 130 {
            self.app.preview_image_report_memo_prepare();
        }
        self.app.update(ctx, frame);
        match self.tick {
            55 => self.app.preview_image_report_check(0),
            75 => self.app.preview_image_report_check(1),
            90 => self.app.preview_image_report_check(2),
            120 => self.app.preview_image_report_check(3),
            155 => self.app.preview_image_report_check(4),
            _ => {}
        }
        if matches!(self.tick, 55 | 75 | 120 | 155) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("report-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 175 {
            assert_eq!(self.shots, 4);
            println!(
                "PASS native image report: actual generate/send/JSON confirm/memo confirm clicks, Escape preserves source/target; memo target selected via fixture; no automatic processing/save; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Image information handoff",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                batch: std::env::args().nth(3).as_deref() == Some("batch"),
                tick: 0,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
