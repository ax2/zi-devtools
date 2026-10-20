//! Actual directory-copy UI with disposable files and controlled egui clicks.
use eframe::egui;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zi_devtools::directory_compare::State;

struct Preview {
    state: State,
    root: PathBuf,
    output: PathBuf,
    frame: u32,
    retry_at: Option<u32>,
    pointer: Option<egui::Pos2>,
    shots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        let target = if self.frame == 15 {
            Some(2)
        } else if self.retry_at == Some(self.frame) {
            Some(3)
        } else {
            None
        };
        if let Some(index) = target {
            self.pointer = Some(
                self.state
                    .preview_copy_position(index)
                    .expect("visible action"),
            );
            input
                .events
                .push(egui::Event::PointerMoved(self.pointer.unwrap()));
            input.events.push(egui::Event::PointerButton {
                pos: self.pointer.unwrap(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        } else if let Some(pos) = self.pointer.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(30),
            "native retry timeout"
        );
        self.state.poll_background();
        egui::CentralPanel::default().show(ctx, |ui| self.state.ui(ui));
        let a = self.state.preview_copy_state(Path::new("a.txt"));
        if self.frame > 20 && self.retry_at.is_none() && a.3 && !a.2 {
            assert!(a.0 && a.1);
            self.retry_at = Some(self.frame + 8);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.retry_at.is_some_and(|at| self.frame == at + 3) {
            assert_eq!(
                a,
                (false, true, false, true),
                "success deselected, old result retained"
            );
            assert_eq!(
                self.state.preview_copy_state(Path::new("b.txt")),
                (true, true, false, true)
            );
            assert_eq!(
                fs::read(self.root.join("output/a.txt")).unwrap(),
                b"source a"
            );
            assert_eq!(
                fs::read(self.root.join("output/b.txt")).unwrap(),
                b"existing destination"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("retry-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.retry_at.is_some_and(|at| self.frame > at + 12) && self.shots == 2 {
            println!(
                "PASS native execute then select-uncopied: success excluded, conflict selected, result retained, existing bytes unchanged; 2 screenshots"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(25));
    }
}
fn main() -> eframe::Result<()> {
    let output = PathBuf::from(std::env::args_os().nth(1).expect("evidence directory"));
    fs::create_dir_all(&output).unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    let root = std::env::temp_dir().join(format!("zi-retry-ui-{}", uuid::Uuid::new_v4()));
    for part in ["left", "right", "output"] {
        fs::create_dir_all(root.join(part)).unwrap();
    }
    fs::write(root.join("left/a.txt"), b"source a").unwrap();
    fs::write(root.join("left/b.txt"), b"source b").unwrap();
    fs::write(root.join("output/b.txt"), b"existing destination").unwrap();
    let mut state = State::default();
    state
        .preview_copy_fixture(
            &root.join("left"),
            &root.join("right"),
            &root.join("output"),
        )
        .unwrap();
    state.preview_retry_plan();
    let fixture_root = root.clone();
    let result = eframe::run_native(
        "目录复制 · 未完成项重试",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 850.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(if light {
                egui::Visuals::light()
            } else {
                egui::Visuals::dark()
            });
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "cjk".into(),
                egui::FontData::from_owned(fs::read(r"C:\Windows\Fonts\msyh.ttc").unwrap()).into(),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .get_mut(&family)
                    .unwrap()
                    .insert(0, "cjk".into());
            }
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(Preview {
                state,
                root: fixture_root,
                output,
                frame: 0,
                retry_at: None,
                pointer: None,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    );
    let resolved = fs::canonicalize(&root).unwrap();
    let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
    assert_eq!(resolved.parent(), Some(temp.as_path()));
    assert!(
        resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("zi-retry-ui-")
    );
    fs::remove_dir_all(resolved).unwrap();
    result
}
