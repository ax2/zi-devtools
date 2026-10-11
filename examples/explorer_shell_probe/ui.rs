//! Actual product startup and rendered confirmation, invoked by the native Shell.
use anyhow::Result;
use eframe::{App, egui};
use std::{
    fs,
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zi_devtools::{app::DevToolsApp, intake::Target, launch::Arguments};

struct Preview {
    app: DevToolsApp,
    args: Arguments,
    light: bool,
    folder: PathBuf,
    name: String,
    target: Target,
    locked: Option<fs::File>,
    tick: usize,
    shots: usize,
    applied: bool,
    after_requested: bool,
    started: Instant,
    completed: Arc<AtomicBool>,
}
impl App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 20 | 21) {
            if self.tick == 20 {
                self.locked.take();
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("intake-confirm")))
                .expect("rendered intake confirmation");
            assert!(ctx.screen_rect().contains_rect(rect));
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: self.tick == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(35),
            "application intake fixture timed out"
        );
        if ctx.current_pass_index() > 0 {
            self.app.update(ctx, frame);
            return;
        }
        if self.tick == 0 {
            self.app.preview_launch_prepare(ctx, &self.args, self.light);
        }
        self.app.update(ctx, frame);
        if self.tick < 20 {
            self.app
                .preview_launch_pending(&self.args.open_files, self.target);
            assert!(self.locked.is_some());
        }
        if self.tick == 10 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.tick > 21 && !self.applied {
            self.applied = self.app.preview_launch_applied(
                &self.args.open_files[0],
                self.target,
                super::fixture_text(self.args.file_action),
            );
        }
        if self.applied && self.tick > 26 && !self.after_requested {
            self.after_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "{}-{}-{}.png",
                        self.name,
                        if self.shots == 0 { "before" } else { "after" },
                        if self.light { "light" } else { "dark" }
                    )),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.applied && self.shots == 2 {
            let proof = serde_json::json!({"productParser":true,"productStartupRoute":true,"exclusiveFileLockBeforeConsent":true,"draftsPreservedBeforeConsent":true,"renderedConfirmationClicked":true,"correctReceiverAfterConsent":true,"automaticDiagnostic":false,"screenshots":2,"light":self.light});
            fs::write(
                self.folder.join(format!(
                    "{}-proof-{}.json",
                    self.name,
                    if self.light { "light" } else { "dark" }
                )),
                serde_json::to_vec_pretty(&proof).unwrap(),
            )
            .unwrap();
            self.completed.store(true, Ordering::Release);
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.tick += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
pub(super) fn run(args: Arguments, light: bool) -> Result<()> {
    let folder = args.config_path.parent().unwrap().join("application-ui");
    fs::create_dir_all(&folder)?;
    let name = args.file_action.map_or("choose".to_owned(), |target| {
        target.action_id().replace(':', "-")
    });
    let target = args
        .file_action
        .unwrap_or_else(|| zi_devtools::intake::recommend(&args.open_files));
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&args.open_files[0])?;
    let completed = Arc::new(AtomicBool::new(false));
    let app_completed = Arc::clone(&completed);
    eframe::run_native(
        "Zi DevTools — Shell intake fixture",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, args.config_path.clone(), args.restore_services),
                args,
                light,
                folder,
                name,
                target,
                locked: Some(locked),
                tick: 0,
                shots: 0,
                applied: false,
                after_requested: false,
                started: Instant::now(),
                completed: app_completed,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    anyhow::ensure!(
        completed.load(Ordering::Acquire),
        "application intake fixture closed before confirmation and evidence completed"
    );
    Ok(())
}
