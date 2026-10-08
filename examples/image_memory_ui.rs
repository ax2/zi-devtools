//! Native menu and editable budget acceptance with a retained 72MiB image.
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
        self.tick = self.tick.wrapping_add(1);
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.focused = true;
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.focused = Some(true);
        }
        let control = match self.tick {
            20 | 21 | 145 | 146 => Some("image-memory-menu"),
            40 | 41 | 70 | 71 | 100 | 101 => Some("image-memory-limit"),
            _ => None,
        };
        if let Some(id) = control {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(id)))
                .expect("rendered budget control");
            assert!(ctx.screen_rect().contains_rect(rect));
            println!("INPUT tick={} id={} rect={:?}", self.tick, id, rect);
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 20 | 40 | 70 | 100 | 145),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if let Some(value) = match self.tick {
            45 => Some("128"),
            75 => Some("64"),
            105 => Some("256"),
            _ => None,
        } {
            input.events.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            });
            input.events.push(egui::Event::Text(value.into()));
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(self.tick, 125 | 126) {
            let pos = egui::pos2(16.0, 16.0);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.tick == 125,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if self.tick == 170 {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if ctx.current_pass_index() > 0 {
            self.app.update(ctx, frame);
            return;
        }
        if self.tick == 0 {
            self.app.preview_budget_prepare(ctx, self.light);
        }
        self.app.update(ctx, frame);
        match self.tick {
            55 => self.app.preview_budget_check(ctx, 1),
            85 => self.app.preview_budget_check(ctx, 2),
            115 => self.app.preview_budget_check(ctx, 3),
            135 | 175 => self.app.preview_budget_check(ctx, 4),
            _ => {}
        }
        if matches!(self.tick, 30 | 55 | 85 | 115 | 135 | 160 | 175) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels = image
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("budget-{}.png", self.shots)),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 190 {
            assert_eq!(self.shots, 7);
            println!(
                "PASS budget menu: real 72MiB image retained; 128MiB accepted, 64MiB rejected with 128 preserved, 256MiB accepted; outside click closes, reopening and Escape closes; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Image memory budget",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 520.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                tick: u32::MAX,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
