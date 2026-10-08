//! Real native task-row cancel/open and closed-instance history, using actual workers.
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
        if self.tick != 34 || self.app.preview_image_tasks_ready() {
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
            20 | 21 => Some(self.app.preview_image_task_control("cancel")),
            55 | 56 | 120 | 121 => Some(self.app.preview_image_task_control("open")),
            80 | 81 => Some("image-instance-close".into()),
            90 | 91 => Some("image-instance-close-confirm".into()),
            140 | 141 => Some("task-history-clear".into()),
            _ => None,
        };
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
                pressed: matches!(self.tick, 20 | 55 | 80 | 90 | 120 | 140),
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
            self.app.preview_image_tasks_prepare(ctx, self.light);
        }
        if self.tick == 100 {
            self.app.preview_image_tasks_show();
        }
        if matches!(self.tick, 19 | 54 | 79) {
            ctx.request_discard("task fixture advances input frames once");
        }
        self.app.update(ctx, frame);
        match self.tick {
            35 => self.app.preview_image_tasks_check(1),
            65 => self.app.preview_image_tasks_check(2),
            95 => self.app.preview_image_tasks_check(3),
            130 => self.app.preview_image_tasks_check(4),
            150 => self.app.preview_image_tasks_check(5),
            _ => {}
        }
        if matches!(self.tick, 15 | 35 | 65 | 95 | 115 | 130 | 150) {
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
                    self.folder.join(format!("tasks-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 175 {
            assert_eq!(self.shots, 7);
            assert!(self.extra >= 3);
            println!(
                "PASS image task rows: actual worker cancellation preserves other instance, opens correct owner, closed history retained/open refused, clear does not resurrect; light={} multipass={}",
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
    eframe::run_native(
        "Image task center",
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
                extra: 0,
                started: Instant::now(),
            }))
        }),
    )
}
