//! Actual native workflow execution and per-step selection with controlled input.
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
    saved: bool,
    instances: bool,
    tick: u32,
    shots: u32,
    extra: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if (self.tick != 54 || self.app.preview_image_workflow_ready())
            && (!self.saved || self.tick != 9 || self.app.preview_saved_image_workflow_pending())
        {
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
            10 | 11 if self.saved => Some("image-flow-import-apply"),
            30 | 31 => Some("image-flow-run"),
            70 | 71 => Some("image-flow-step-0"),
            90 | 91 if self.instances => Some("image-instance-create"),
            105 | 106 if self.instances => Some("image-instance-picker"),
            110 | 111 if self.instances => Some("image-instance-select-0"),
            130 | 131 | 150 | 151 if self.instances => Some("image-instance-close"),
            140 | 141 if self.instances => Some("image-instance-close-keep"),
            160 | 161 if self.instances => Some("image-instance-close-confirm"),
            _ => None,
        };
        if let Some(id) = id {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(id)))
                .expect("rendered control");
            assert!(ctx.screen_rect().contains_rect(rect));
            println!("INPUT tick={} id={} rect={:?}", self.tick, id, rect);
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(
                    self.tick,
                    10 | 30 | 70 | 90 | 105 | 110 | 130 | 140 | 150 | 160
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
        if self.tick == 0 {
            if self.saved {
                self.app
                    .preview_saved_image_workflow_prepare(ctx, self.light, &self.folder);
            } else {
                self.app.preview_image_workflow_prepare(ctx, self.light);
            }
        }
        if matches!(self.tick, 29 | 69 | 90) {
            ctx.request_discard("workflow fixture counts input frames once");
        }
        self.app.update(ctx, frame);
        if self.tick == 55 {
            self.app.preview_image_workflow_check(1);
            if self.saved {
                self.app.preview_saved_image_workflow_check();
            }
        }
        if self.tick == 85 {
            self.app.preview_image_workflow_check(2);
        }
        if self.instances {
            match self.tick {
                100 => self.app.preview_image_instances_check(1),
                120 => self.app.preview_image_instances_check(2),
                145 => self.app.preview_image_instances_check(3),
                170 => self.app.preview_image_instances_check(4),
                _ => {}
            }
        }
        if matches!(self.tick, 20 | 55 | 85)
            || (self.instances && matches!(self.tick, 100 | 120 | 145 | 170))
        {
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
                    self.folder.join(format!("workflow-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == if self.instances { 190 } else { 110 } {
            assert_eq!(self.shots, if self.instances { 7 } else { 3 });
            assert!(self.extra >= 3);
            println!(
                "PASS image workflow: actual run button, four outputs, precise crop/resize/WebP pixels and retained original; actual step selection; no implicit save; light={} multipass={}",
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
    let instances = std::env::args().nth(3).as_deref() == Some("instances");
    let saved = instances || std::env::args().nth(3).as_deref() == Some("saved");
    eframe::run_native(
        "Image workflow",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                saved,
                instances,
                tick: u32::MAX,
                shots: 0,
                extra: 0,
                started: Instant::now(),
            }))
        }),
    )
}
