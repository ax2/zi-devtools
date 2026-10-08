//! Native instance relay pointer acceptance with actual encoded image results.
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
    extra: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if !matches!(self.tick, 29 | 149 | 239) || self.app.preview_instance_relay_ready() {
            self.tick = self.tick.wrapping_add(1);
        }
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.focused = true;
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.focused = Some(true);
        }
        let id = match self.tick {
            20 | 21 | 140 | 141 | 230 | 231 => Some("image-relay-send".into()),
            40 | 41 | 160 | 161 | 250 | 251 => Some("image-relay-workflow".into()),
            55 | 56 => Some(self.app.preview_instance_relay_control()),
            70 | 71 => Some("image-relay-replace".into()),
            90 | 91 | 180 | 181 => Some("image-relay-apply".into()),
            _ => None,
        };
        if self.tick == 270 {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if let Some(id) = id {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(&id)))
                .expect("rendered control");
            assert!(ctx.screen_rect().contains_rect(rect));
            println!("INPUT tick={} id={} rect={:?}", self.tick, id, rect);
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(
                    self.tick,
                    20 | 40 | 55 | 70 | 90 | 140 | 160 | 180 | 230 | 250
                ),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if ctx.current_pass_index() > 0 {
            self.extra += 1;
            self.app.update(ctx, frame);
            return;
        }
        if matches!(self.tick, 0 | 125 | 215) {
            self.app.preview_instance_relay_prepare(ctx, self.light);
        }
        if matches!(self.tick, 39 | 54 | 89 | 179) {
            ctx.request_discard("relay fixture advances raw input once");
        }
        self.app.update(ctx, frame);
        match self.tick {
            60 => self.app.preview_instance_relay_check(1),
            75 => self.app.preview_instance_relay_check(2),
            100 => self.app.preview_instance_relay_check(3),
            190 => self.app.preview_instance_relay_check(4),
            280 => self.app.preview_instance_relay_check(5),
            _ => {}
        }
        if matches!(self.tick, 24 | 45 | 60 | 100 | 165 | 190 | 255 | 280) {
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
                    self.folder.join(format!("relay-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 305 {
            assert_eq!(self.shots, 8);
            assert!(self.extra >= 4);
            println!(
                "PASS instance relay: actual encoded source preserved; existing target identity, replacement consent and retained definition; new instance only on apply; Escape leaves two originals; light={} multipass={}",
                self.light, self.extra
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
    let small = std::env::args().nth(3).as_deref() == Some("small");
    eframe::run_native(
        "Image instance relay",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size(if small {
                [760.0, 520.0]
            } else {
                [1100.0, 900.0]
            }),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                tick: u32::MAX,
                shots: 0,
                extra: 0,
                started: Instant::now(),
            }))
        }),
    )
}
