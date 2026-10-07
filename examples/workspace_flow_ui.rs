//! Actual save confirmation, SQLite snapshot restore and explicit pipeline rerun.
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
    tick: u32,
    shots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 30 | 31) {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("workspace-save-confirm")))
                .unwrap();
            assert!(ctx.screen_rect().contains_rect(rect));
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: self.tick == 30,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(self.tick, 140 | 141) {
            input.modifiers = egui::Modifiers::CTRL;
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: self.tick == 140,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if self.tick == 0 {
            self.app.preview_flow_save_prepare(ctx, self.light);
        }
        if self.tick == 80 {
            self.app.preview_flow_disk_restore();
        }
        self.app.update(ctx, frame);
        if self.tick == 115 {
            self.app.preview_flow_restored_check(false);
        }
        if self.tick == 180 {
            self.app.preview_flow_restored_check(true);
        }
        if matches!(self.tick, 20 | 120 | 180) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("snapshot-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 200 {
            assert_eq!(self.shots, 3);
            println!(
                "PASS native save confirmation -> actual SQLite snapshot -> close live instance -> restore pipeline idle without preview -> explicit CtrlEnter preview; source not applied; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).unwrap());
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Workspace pipeline snapshots",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                tick: 0,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
