//! Native image-to-text-flow relay: explicit target selection, confirmation and receipt.
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
    shots: u8,
    started: Instant,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.focused = true;
        if let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            viewport.focused = Some(true);
        }

        if matches!(self.tick, 29 | 49) && !self.app.preview_text_flow_image_relay_ready() {
            return;
        }
        let id = match self.tick {
            30 | 31 => Some("image-relay-text-flow"),
            50 | 51 => Some("image-relay-apply"),
            _ => None,
        };
        if let Some(id) = id {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(id)))
                .expect("visible image relay control");
            assert!(ctx.screen_rect().contains_rect(rect), "{id}: {rect:?}");
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 30 | 50),
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
            self.app
                .preview_text_flow_image_relay_prepare(ctx, self.light);
        }
        if matches!(self.tick, 29 | 49) {
            ctx.request_discard("wait for the prepared in-memory image before relay input");
        }
        self.app.update(ctx, frame);
        match self.tick {
            40 => self.app.preview_text_flow_image_relay_check(0),
            60 => self.app.preview_text_flow_image_relay_check(1),
            _ => {}
        }
        if matches!(self.tick, 40 | 60) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("image-flow-{}.png", self.shots)),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 70 {
            assert_eq!(self.shots, 2);
            println!(
                "PASS native image-to-text-flow relay: target and confirmation clicked; new image.info flow received without auto-run; source retained; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick = self.tick.wrapping_add(1);
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}

fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let config = folder.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Image to text-flow relay",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                folder,
                light,
                tick: u32::MAX,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
