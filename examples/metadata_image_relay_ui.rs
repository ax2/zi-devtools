//! Controlled native input: clean-image cancellation and explicit replacement in both targets.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    light: bool,
    tick: u32,
    shots: u32,
    extra_passes: u32,
    started: Instant,
    source: Option<Arc<Vec<u8>>>,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        // Wait for the real result early enough that its measured modal layout
        // settles before pointer presses; never set consent programmatically.
        if !matches!(self.tick, 54 | 100 | 160) || self.app.preview_metadata_relay_ready() {
            self.tick = self.tick.wrapping_add(1);
        }
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.focused = true;
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.focused = Some(true);
        }
        if matches!(self.tick, 65 | 66) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: self.tick == 65,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let id = match self.tick {
            30 | 31 | 90 | 91 | 158 | 159 => Some("image-relay-send"),
            110 | 111 | 180 | 181 => Some("image-relay-replace"),
            125 | 126 | 195 | 196 => Some("image-relay-apply"),
            170 | 171 => Some("image-relay-edit"),
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
            println!("INPUT tick={} id={} rect={:?}", self.tick, id, rect);
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 30 | 90 | 110 | 125 | 158 | 170 | 180 | 195),
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(40));
        if ctx.current_pass_index() > 0 {
            self.extra_passes += 1;
            self.app.update(ctx, frame);
            return;
        }
        if matches!(self.tick, 29 | 89 | 124 | 157 | 194) {
            ctx.request_discard("cleaned image relay fixture input remains frame based");
        }
        if self.tick == 0 {
            self.source = Some(self.app.preview_metadata_relay_prepare(ctx, self.light));
        }
        if self.tick == 150 {
            self.app.preview_metadata_relay_reopen();
        }
        self.app.update(ctx, frame);
        if matches!(self.tick, 115 | 130 | 185 | 205) {
            println!(
                "STATE tick={} {}",
                self.tick,
                self.app.preview_metadata_relay_status()
            );
        }
        let phase = match self.tick {
            55 => Some(1),
            75 => Some(2),
            135 => Some(3),
            210 => Some(4),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app
                .preview_metadata_relay_check(phase, self.source.as_ref().unwrap());
        }
        if matches!(self.tick, 55 | 75 | 115 | 135 | 175 | 210) {
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
        if self.tick == 235 {
            assert_eq!(self.shots, 6);
            assert!(self.extra_passes >= 5);
            println!(
                "PASS metadata image relay: send/Escape preserve source and both targets; explicit replacement checkbox/confirmation for convert and edit; decoded cleaned pixels match; no automatic encode/save; light={} multipass={}",
                self.light, self.extra_passes
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
        "Cleaned image relay",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                tick: u32::MAX,
                shots: 0,
                extra_passes: 0,
                started: Instant::now(),
                source: None,
            }))
        }),
    )
}
