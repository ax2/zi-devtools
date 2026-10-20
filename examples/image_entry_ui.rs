//! Shared image routes exercised through actual Ctrl+K search and Enter selection.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;

const TOOLS: [&str; 6] = [
    "image-crop-annotate",
    "image-tools",
    "image-batch",
    "image-metadata",
    "screenshot-workbench",
    "image-workflow",
];

struct Preview {
    app: DevToolsApp,
    output: PathBuf,
    light: bool,
    frame: u32,
    started: Instant,
    fingerprint: String,
    screenshots: Vec<Arc<egui::ColorImage>>,
}
impl Preview {
    fn save_screenshots(&self) {
        for (index, image) in self.screenshots.iter().enumerate() {
            let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                self.output.join(format!("entry-{index}.png")),
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }
    fn pair(&self) -> (&'static str, &'static str) {
        let index = (self.frame / 20) as usize;
        // Start by reproducing editor -> converter, then cover all 36 pairs.
        (TOOLS[index / 6], TOOLS[(index % 6 + 1) % 6])
    }
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let offset = self.frame % 20;
        if matches!(offset, 2 | 3 | 9 | 10) && self.frame < 720 {
            let ctrl = offset < 4;
            let modifiers = if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            };
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key: if ctrl { egui::Key::K } else { egui::Key::Enter },
                physical_key: None,
                pressed: matches!(offset, 2 | 9),
                repeat: false,
                modifiers,
            });
        }
        if offset == 5 && self.frame < 720 {
            input.events.push(egui::Event::Text(self.pair().1.into()));
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(120),
            "image route timeout"
        );
        if self.frame == 0 {
            self.fingerprint = self.app.preview_image_entry_prepare(ctx, self.light);
        }
        if self.frame < 720 && self.frame.is_multiple_of(20) {
            self.app.open_startup_tool(self.pair().0);
        }
        self.app.update(ctx, frame);
        if self.frame < 720 && self.frame % 20 == 12 && self.frame / 20 < 6 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                self.screenshots.push(image);
            }
        }
        if self.frame < 720 && self.frame % 20 == 16 {
            let (from, expected) = self.pair();
            let actual = self.app.preview_image_entry_check(&self.fingerprint);
            if actual != expected {
                self.save_screenshots();
            }
            assert_eq!(actual, expected, "shared route {from} -> {expected}");
            println!(
                "PASS shared image route {from} -> {expected}; retained source pixels, encoded bytes, paths and settings; no job started"
            );
        }
        if self.frame == 725 {
            assert_eq!(self.screenshots.len(), 6);
            self.save_screenshots();
            println!(
                "PASS all 36 image routes via startup source and actual Ctrl+K/Enter destination"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(20));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    std::fs::create_dir_all(&output).unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let root = std::env::temp_dir().join(format!("zi-image-entry-ui-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Image tool route acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 1000.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                output,
                light,
                frame: 0,
                started: Instant::now(),
                fingerprint: String::new(),
                screenshots: Vec::new(),
            }))
        }),
    )
}
