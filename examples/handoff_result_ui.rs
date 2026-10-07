//! Real native button eligibility; synthetic input; clipboard output intercepted.
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
    captured: u32,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.frame, 20 | 21) {
            let pos = self.app.preview_handoff_result_copy_position();
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(self.frame, 50 | 51) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: self.frame == 50,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app.preview_handoff_result_prepare(ctx, self.light);
        }
        self.app.update(ctx, frame);
        ctx.output_mut(|out| {
            out.commands.retain(|command| {
                if let egui::OutputCommand::CopyText(_) = command {
                    panic!("disabled copy button must not copy or erase clipboard");
                }
                true
            })
        });
        if self.frame == 30 {
            self.app.preview_handoff_result_check(false);
        }
        if self.frame == 70 {
            self.app.preview_handoff_result_check(true);
        }
        if matches!(self.frame, 15 | 75) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!("handoff-{}.png", self.captured)),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 90 {
            assert_eq!(self.captured, 2);
            println!(
                "PASS native handoff result light={}: actual disabled copy click emits no clipboard command; Ctrl+Enter enables new result; original source draft retained",
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
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Handoff result acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                frame: 0,
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
