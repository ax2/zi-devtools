//! Native selected-file outputs and explicit handoff acceptance.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
fn inject_click(input: &mut egui::RawInput, pos: egui::Pos2, phase: &mut u8) {
    input.events.push(egui::Event::PointerMoved(pos));
    match *phase {
        0 => *phase = 1,
        1 => {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            *phase = 2;
        }
        2 => {
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            *phase = 3;
        }
        _ => {}
    }
}
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    light: bool,
    file_output_ready: bool,
    file_output_selected: bool,
    output_choice_phase: u8,
    output_send_ready: bool,
    output_send_phase: u8,
    last_output_send: Option<egui::Pos2>,
    output_send_stable: u8,
    output_apply_phase: u8,
    last_output_choice: Option<egui::Pos2>,
    output_choice_stable: u8,
    report_captured: bool,
    file_output_handoff_open: bool,
    handoff_settle_frames: u8,
    handoff_screenshot_pending: bool,
    file_output_received: bool,
    handoff_apply_ready: bool,
    last_handoff_apply: Option<egui::Pos2>,
    handoff_apply_stable: u8,
    tick: u32,
    shots: u32,
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 70 | 71) {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: self.tick == 70,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let id = match self.tick {
            30 | 31 => Some(egui::Id::new(("file-report", 0usize))),
            60 | 61 | 80 | 81 => Some(egui::Id::new(("file-source", 0usize))),
            130 | 131 => Some(egui::Id::new("file-material-read")),
            _ => None,
        };
        if let Some(id) = id {
            let rect = ctx
                .data(|data| data.get_temp::<egui::Rect>(id))
                .expect("button rendered");
            assert!(
                ctx.screen_rect().contains_rect(rect),
                "button visible: {rect:?}"
            );
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: matches!(self.tick, 30 | 60 | 80 | 100 | 130),
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(self.tick, 100 | 101) {
            let pos = self.app.preview_file_material_apply_position();
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.tick == 100,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if matches!(self.tick, 179..=181) {
            let pos = self.app.preview_file_material_apply_position();
            input.events.push(egui::Event::PointerMoved(pos));
            if matches!(self.tick, 180 | 181) {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.tick == 180,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if matches!(self.tick, 183..=185) {
            let pos = self.app.preview_text_flow_position(1);
            input.events.push(egui::Event::PointerMoved(pos));
            if matches!(self.tick, 184 | 185) {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.tick == 184,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if self.file_output_ready && !self.file_output_selected {
            let pos = self
                .app
                .preview_file_flow_output_choice_position(&self.folder.join("flow-handoff.txt"));
            inject_click(input, pos, &mut self.output_choice_phase);
        }
        if self.output_send_ready && !self.file_output_handoff_open {
            let pos = self.app.preview_file_flow_output_position();
            inject_click(input, pos, &mut self.output_send_phase);
        }
        if self.handoff_apply_ready && !self.file_output_received {
            let pos = self.app.preview_file_material_apply_position();
            inject_click(input, pos, &mut self.output_apply_phase);
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if self.tick == 0 {
            self.app
                .preview_file_material_prepare(ctx, self.light, self.folder.join("source.txt"));
        }
        self.app.update(ctx, frame);
        match self.tick {
            45 => {
                self.app.preview_file_material_check(0);
                self.app.preview_file_report_dispatch();
            }
            65 | 95 => self.app.preview_file_material_check(4),
            75 => self.app.preview_file_material_check(3),
            115 => self.app.preview_file_material_check(1),
            145 => self.app.preview_file_material_check(2),
            165 => self.app.preview_file_flow_handoff_prepare(
                ctx,
                self.light,
                &self.folder.join("flow-handoff.txt"),
            ),
            178 => self
                .app
                .preview_file_flow_handoff_check(0, &self.folder.join("flow-handoff.txt")),
            182 => self
                .app
                .preview_file_flow_handoff_check(1, &self.folder.join("flow-handoff.txt")),
            _ => {}
        }
        if self.app.preview_file_flow_output_ready() && !self.file_output_ready {
            if !self.report_captured {
                assert!(self.app.preview_file_flow_report_selected());
                self.report_captured = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let pos = self
                .app
                .preview_file_flow_output_choice_position(&self.folder.join("flow-handoff.txt"));
            if self
                .last_output_choice
                .is_some_and(|last| last.distance(pos) < 0.25)
            {
                self.output_choice_stable = self.output_choice_stable.saturating_add(1);
            } else {
                self.output_choice_stable = 0;
            }
            self.last_output_choice = Some(pos);
            self.file_output_ready = self.output_choice_stable >= 3;
        }
        if self.file_output_ready
            && !self.file_output_selected
            && self
                .app
                .preview_file_flow_output_selected(&self.folder.join("flow-handoff.txt"))
        {
            self.file_output_selected = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.file_output_selected && !self.file_output_handoff_open {
            let pos = self.app.preview_file_flow_output_position();
            if self
                .last_output_send
                .is_some_and(|last| last.distance(pos) < 0.25)
            {
                self.output_send_stable = self.output_send_stable.saturating_add(1);
            } else {
                self.output_send_stable = 0;
            }
            self.last_output_send = Some(pos);
            self.output_send_ready = self.output_send_stable >= 3;
        }
        if self.output_send_phase == 3 && !self.file_output_handoff_open {
            self.app
                .preview_file_flow_output_handoff_check(0, &self.folder.join("flow-handoff.txt"));
            self.file_output_handoff_open = true;
            self.handoff_settle_frames = 15;
            self.handoff_screenshot_pending = true;
        }
        if self.file_output_handoff_open && self.handoff_screenshot_pending {
            if self.handoff_settle_frames > 0 {
                self.handoff_settle_frames -= 1;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.handoff_screenshot_pending = false;
            }
        }
        if self.file_output_handoff_open
            && !self.handoff_screenshot_pending
            && !self.handoff_apply_ready
        {
            let pos = self.app.preview_file_material_apply_position();
            if self
                .last_handoff_apply
                .is_some_and(|last| last.distance(pos) < 0.25)
            {
                self.handoff_apply_stable = self.handoff_apply_stable.saturating_add(1);
            } else {
                self.handoff_apply_stable = 0;
            }
            self.last_handoff_apply = Some(pos);
            self.handoff_apply_ready = self.handoff_apply_stable >= 2;
        }
        if self.output_apply_phase == 3 && !self.file_output_received {
            self.app
                .preview_file_flow_output_handoff_check(1, &self.folder.join("flow-handoff.txt"));
            self.file_output_received = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if matches!(self.tick, 45 | 69 | 89 | 145 | 177) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect::<Vec<_>>();
                image::save_buffer(
                    self.folder.join(format!("material-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 260 {
            assert!(self.file_output_selected);
            assert!(self.file_output_handoff_open);
            assert!(self.file_output_received);
            assert_eq!(self.shots, 9);
            println!(
                "PASS selected report, shared typed file handoff, Escape cancel/reopen, identity recheck, explicit encoding read, file-to-hash-workflow handoff, named source-file output selection and explicit encoding receipt without automatic reread/write; light={}",
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
    std::fs::write(folder.join("source.txt"), b"abc").unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "File material acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 900.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                file_output_ready: false,
                file_output_selected: false,
                output_choice_phase: 0,
                output_send_ready: false,
                output_send_phase: 0,
                last_output_send: None,
                output_send_stable: 0,
                output_apply_phase: 0,
                last_output_choice: None,
                output_choice_stable: 0,
                report_captured: false,
                file_output_handoff_open: false,
                handoff_settle_frames: 0,
                handoff_screenshot_pending: false,
                file_output_received: false,
                handoff_apply_ready: false,
                last_handoff_apply: None,
                handoff_apply_stable: 0,
                tick: 0,
                shots: 0,
                started: Instant::now(),
            }))
        }),
    )
}
