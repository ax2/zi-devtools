//! Native global search composition and modified-key regression.
use eframe::egui;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    output: PathBuf,
    light: bool,
    quick: bool,
    frame: u32,
    start: Instant,
    fingerprint: String,
    expected: String,
    shots: Vec<Arc<egui::ColorImage>>,
    prepared: bool,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let ime = match self.frame {
            20 | 32 => Some(egui::ImeEvent::Enabled),
            21 | 33 => Some(egui::ImeEvent::Preedit("tu".into())),
            28 => Some(egui::ImeEvent::Commit("图片".into())),
            40 => Some(egui::ImeEvent::Disabled),
            _ => None,
        };
        if let Some(event) = ime {
            input.events.push(egui::Event::Ime(event));
        }
        let key = match self.frame {
            10 | 11 if !self.quick => Some((egui::Key::K, egui::Modifiers::CTRL, self.frame == 10)),
            19 | 20 | 46 | 47 => Some((
                egui::Key::A,
                egui::Modifiers::CTRL,
                matches!(self.frame, 19 | 46),
            )),
            24 | 25 | 50 | 51 => Some((
                egui::Key::ArrowDown,
                egui::Modifiers::NONE,
                matches!(self.frame, 24 | 50),
            )),
            28 | 29 | 58 | 59 => Some((
                egui::Key::Enter,
                egui::Modifiers::NONE,
                matches!(self.frame, 28 | 58),
            )),
            36 | 37 => Some((egui::Key::Escape, egui::Modifiers::NONE, self.frame == 36)),
            42 | 43 => Some((
                egui::Key::ArrowDown,
                egui::Modifiers::SHIFT,
                self.frame == 42,
            )),
            54 | 55 => Some((egui::Key::ArrowUp, egui::Modifiers::NONE, self.frame == 54)),
            _ => None,
        };
        if let Some((key, mut modifiers, pressed)) = key {
            if modifiers.ctrl {
                modifiers.command = true;
            }
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        if matches!(self.frame, 15 | 48) {
            input.events.push(egui::Event::Text("图片".into()));
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(90));
        if self.frame == 0 && !self.prepared {
            self.fingerprint = self.app.preview_image_entry_prepare(ctx, self.light);
            self.app.open_startup_tool("image-tools");
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.prepared = true;
        }
        if !ctx.input(|input| input.raw.focused) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        if self.frame == 10 && self.quick {
            self.app.preview_launcher_keyboard_quick(ctx);
        }
        self.app.update(ctx, frame);
        if matches!(self.frame, 18 | 26 | 30 | 38 | 44 | 52 | 56) {
            let (open, index, query, first) = self.app.preview_launcher_keyboard_state();
            println!(
                "state frame={} open={} index={} query={:?} first={:?}",
                self.frame, open, index, query, first
            );
            assert!(
                open,
                "search closed during composition at frame {}",
                self.frame
            );
            if self.frame == 18 {
                self.expected = first;
                assert!(!self.expected.is_empty());
            }
            assert_eq!(
                index,
                if self.frame == 52 { 1 } else { 0 },
                "candidate/modified key changed selection at frame {}",
                self.frame
            );
            println!("PASS launcher keyboard phase {}", self.frame);
        }
        if matches!(self.frame, 18 | 44 | 62) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                self.shots.push(image);
            }
        }
        if self.frame == 64 {
            self.app
                .preview_launcher_keyboard_opened(&self.expected, &self.fingerprint);
            assert_eq!(self.shots.len(), 3);
            for (i, image) in self.shots.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("keyboard-{i}.png")),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            println!(
                "PASS native IME arrows/Escape/commit Enter and ShiftDown do not navigate; plain arrows/Enter resume; drafts retained"
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("output"));
    std::fs::create_dir_all(&output).unwrap();
    let root = std::env::temp_dir().join(format!("zi-launcher-keyboard-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Launcher keyboard acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                output,
                light,
                quick: std::env::args().nth(3).as_deref() == Some("quick"),
                frame: 0,
                start: Instant::now(),
                fingerprint: String::new(),
                expected: String::new(),
                shots: Vec::new(),
                prepared: false,
            }))
        }),
    )
}
