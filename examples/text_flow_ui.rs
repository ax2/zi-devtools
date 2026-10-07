//! Actual native chain controls, synthetic pointer input, screenshots of real rendering.
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
    table: bool,
    frame: u32,
    captured: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let index = match self.frame {
            20 | 21 => Some(if self.table { 3 } else { 0 }),
            50 | 51 => Some(1),
            120 | 121 => Some(2),
            142 | 143 if self.table => Some(5),
            180 | 181 if self.table => Some(2),
            200 | 201 if self.table => Some(4),
            _ => None,
        };
        if let Some(index) = index {
            let pos = self.app.preview_text_flow_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame % 2 == 0,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app.preview_text_flow_prepare(ctx, self.light);
        }
        if self.frame == 100 {
            if self.table {
                self.app.preview_text_flow_table_check(0);
            } else {
                self.app.preview_text_flow_check(false);
            }
        }
        if self.frame == 130 {
            if self.table {
                self.app.preview_text_flow_table_check(1);
            } else {
                self.app.preview_text_flow_check(true);
            }
        }
        if self.table && self.frame == 160 {
            self.app.preview_text_flow_table_check(2);
        }
        if self.table && self.frame == 190 {
            self.app.preview_text_flow_table_check(1);
        }
        if self.table && self.frame == 225 {
            self.app.preview_text_flow_table_check(3);
        }
        self.app.update(ctx, frame);
        if matches!(self.frame, 100 | 135)
            || (self.table && matches!(self.frame, 160 | 185 | 210 | 225))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|c| c.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("flow-{}.png", self.captured)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 150 && !self.table {
            assert_eq!(self.captured, 2);
            println!(
                "PASS actual native sample/run/intermediate results/send review; original table draft retained; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        if self.table && self.frame == 240 {
            assert_eq!(self.captured, 6);
            println!(
                "PASS typed native table sample/run/review/cancel/review/confirm: new visible typed instance, source and results retained; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let table = std::env::args().nth(3).as_deref() == Some("table");
    eframe::run_native(
        "Text flow acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                table,
                frame: 0,
                captured: 0,
                started: Instant::now(),
            }))
        }),
    )
}
