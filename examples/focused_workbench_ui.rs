//! Native pinned selector acceptance with synthetic pointer input.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
const VIEWS: &[&str] = &[
    "data",
    "data-transform",
    "csv-merge",
    "pipeline",
    "data-sqlite-export",
    "workspace-sessions",
    "text-flow",
];
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    light: bool,
    tick: u32,
    shots: u32,
    started: Instant,
    header: Option<egui::Rect>,
    scrolled: bool,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.tick >= 30 {
            let index = ((self.tick - 30) / 25) as usize;
            let phase = (self.tick - 30) % 25;
            if index < VIEWS.len() && (5..=8).contains(&phase) {
                let (_, body, _) = ctx
                    .data(|data| {
                        data.get_temp::<(egui::Rect, egui::Rect, egui::Vec2)>(egui::Id::new(
                            "workspace-layout",
                        ))
                    })
                    .expect("workspace layout visible");
                input.events.push(egui::Event::PointerMoved(body.center()));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -250.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if index < VIEWS.len() && phase <= 1 {
                let rect = ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new(("workbench-view", VIEWS[index])))
                    })
                    .expect("pinned selector visible");
                assert!(ctx.screen_rect().contains(rect.center()));
                input.events.push(egui::Event::PointerMoved(rect.center()));
                input.events.push(egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: phase == 0,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(40),
            "native fixture timed out"
        );
        if self.tick == 0 {
            self.app.preview_focused_view_prepare(ctx, self.light);
        }
        self.app.update(ctx, frame);
        if self.tick >= 30 {
            let index = ((self.tick - 30) / 25) as usize;
            if index < VIEWS.len() && (self.tick - 30) % 25 == 15 {
                self.app.preview_focused_view_check(VIEWS[index]);
                let (header, body, offset) = ctx
                    .data(|data| {
                        data.get_temp::<(egui::Rect, egui::Rect, egui::Vec2)>(egui::Id::new(
                            "workspace-layout",
                        ))
                    })
                    .expect("workspace layout recorded");
                assert!(
                    ctx.screen_rect().contains_rect(header),
                    "instance controls clipped"
                );
                assert!(header.bottom() < body.top());
                if let Some(previous) = self.header {
                    assert_eq!(
                        previous, header,
                        "instance controls moved while tool content scrolls"
                    );
                }
                self.header = Some(header);
                if VIEWS[index] == "data" {
                    assert!(offset.y > 100.0, "data content did not scroll: {offset:?}");
                    self.scrolled = true;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("view-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 220 {
            assert_eq!(self.shots, 7);
            assert!(self.scrolled);
            println!(
                "PASS native pinned selector seven views; same-instance draft retained; light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Focused workbenches acceptance",
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
                header: None,
                scrolled: false,
            }))
        }),
    )
}
