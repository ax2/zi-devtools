//! Synthetic native viewport: production canvas, real egui gesture handling and screenshots.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::{
    recorder::{DisplayInfo, Region},
    recorder_ui::RecorderState,
};

struct Preview {
    state: RecorderState,
    display: DisplayInfo,
    folder: PathBuf,
    frame: u32,
    shots: u32,
    canvas: Option<egui::Rect>,
    accepted: Option<Region>,
    started: Instant,
    requested_scale: f32,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let Some(rect) = self.canvas else { return };
        let origin = rect.min + rect.size() * egui::vec2(0.2, 0.3);
        let small =
            origin + egui::vec2(rect.width() / 1920.0 * 80.0, rect.height() / 1080.0 * 24.0);
        let end = rect.min + rect.size() * egui::vec2(0.7, 0.8);
        let event = match self.frame {
            20 | 50 => Some((origin, None)),
            21 | 51 => Some((origin, Some(true))),
            22 => Some((small, None)),
            24 => Some((small, Some(false))),
            52 => Some((end, None)),
            70 => Some((end, Some(false))),
            _ => None,
        };
        if let Some((pos, pressed)) = event {
            input.events.push(egui::Event::PointerMoved(pos));
            if let Some(pressed) = pressed {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // The native viewport scale is available now; creation can still report 1.0.
        if self.frame == 0 {
            ctx.set_pixels_per_point(self.requested_scale);
        }
        assert!(
            self.started.elapsed() < Duration::from_secs(30),
            "native fixture timed out"
        );
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.canvas = Some(ui.available_rect_before_wrap());
                let (region, error) = self.state.preview_selection_canvas(ui, &self.display);
                if let Some(region) = region {
                    assert_eq!(
                        region,
                        Region {
                            x: 384,
                            y: 324,
                            width: 960,
                            height: 540
                        }
                    );
                    self.accepted = Some(region);
                }
                if self.frame == 32 {
                    assert!(error, "undersized drag must display an error");
                    assert!(self.accepted.is_none());
                }
                if self.frame == 62 {
                    assert!(!error, "new drag must clear the old error");
                    assert!(
                        self.accepted.is_none(),
                        "drag preview must not commit early"
                    );
                }
            });
        if matches!(self.frame, 32 | 62) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<_> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                image::save_buffer(
                    self.folder.join(format!("selection-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.frame == 85 {
            assert!((ctx.pixels_per_point() - self.requested_scale).abs() < 0.001);
            assert_eq!(self.shots, 2);
            assert!(self.accepted.is_some(), "release must return the crop");
            std::fs::write(self.folder.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "source": "synthetic-gradient-no-desktop-capture",
                "display": [1920,1080], "canvasPoints": [self.canvas.unwrap().width(),self.canvas.unwrap().height()],
                "pixelsPerPoint": ctx.pixels_per_point(), "requestedPixelsPerPoint": self.requested_scale,
                "cropPixels": [384,324,960,540],
                "smallDragRejected": true, "retryClearedError": true, "shots": self.shots
            })).unwrap()).unwrap();
            println!(
                "PASS production selection canvas: small drag rejected, retry clears error, crop equals 960x540 pixels, 2 native screenshots"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(25));
    }
}

fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("screenshot directory"));
    std::fs::create_dir_all(&folder).unwrap();
    let width = std::env::args()
        .nth(2)
        .unwrap_or("960".into())
        .parse::<f32>()
        .unwrap();
    let scale = std::env::args()
        .nth(3)
        .unwrap_or("1.5".into())
        .parse::<f32>()
        .unwrap();
    eframe::run_native(
        "Recorder selection synthetic acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([width, 540.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut fonts = egui::FontDefinitions::default();
            let data = std::fs::read(r"C:\Windows\Fonts\msyh.ttc").expect("Chinese UI font");
            fonts.font_data.insert(
                "selection-cjk".into(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "selection-cjk".into());
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(Preview {
                state: RecorderState::default(),
                display: DisplayInfo {
                    device_name: "synthetic".into(),
                    name: "Synthetic".into(),
                    x: -1920,
                    y: -120,
                    width: 1920,
                    height: 1080,
                    primary: false,
                },
                folder,
                frame: 0,
                shots: 0,
                canvas: None,
                accepted: None,
                started: Instant::now(),
                requested_scale: scale,
            }))
        }),
    )
}
