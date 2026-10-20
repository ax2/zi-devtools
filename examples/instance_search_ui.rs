//! Native current-instance search at the minimum supported window size.
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
    flow_folder: PathBuf,
    light: bool,
    quick: bool,
    frame: u32,
    start: Instant,
    ids: (String, String, String),
    tasks: (String, String),
    click: Option<egui::Pos2>,
    shots: Vec<Arc<egui::ColorImage>>,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|e| matches!(e, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        let ime = match self.frame {
            22 => Some(egui::ImeEvent::Enabled),
            23 => Some(egui::ImeEvent::Preedit("tu".into())),
            26 => Some(egui::ImeEvent::Disabled),
            _ => None,
        };
        if let Some(ime) = ime {
            input.events.push(egui::Event::Ime(ime));
        }
        let key = match self.frame {
            15 | 16 if !self.quick => {
                Some((egui::Key::Num5, egui::Modifiers::CTRL, self.frame == 15))
            }
            28 | 29 => Some((egui::Key::A, egui::Modifiers::CTRL, self.frame == 28)),
            20 | 21 | 32 | 33 | 66 | 67 => Some((
                egui::Key::ArrowDown,
                egui::Modifiers::NONE,
                self.frame % 2 == 0,
            )),
            24 | 25 | 36 | 37 | 70 | 71 | 76 | 77 | 86 | 87 | 94 | 95 | 114 | 115 => {
                Some((egui::Key::Enter, egui::Modifiers::NONE, self.frame % 2 == 0))
            }
            160 | 162 | 164 if !self.quick => {
                Some((egui::Key::Enter, egui::Modifiers::NONE, false))
            }
            161 | 163 | 165 if !self.quick => Some((egui::Key::Enter, egui::Modifiers::NONE, true)),
            52 | 53 | 124 | 125 => Some((
                egui::Key::ArrowLeft,
                egui::Modifiers::ALT,
                self.frame % 2 == 0,
            )),
            56 | 57 => Some((
                egui::Key::ArrowRight,
                egui::Modifiers::ALT,
                self.frame % 2 == 0,
            )),
            74 | 75 => Some((
                egui::Key::ArrowUp,
                egui::Modifiers::NONE,
                self.frame % 2 == 0,
            )),
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
        if self.frame == 30 {
            input.events.push(egui::Event::Text("重复材料".into()));
        }
        if matches!(self.frame, 45 | 46) {
            let pos = self.click.unwrap();
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame == 45,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(90));
        match self.frame {
            0 => {
                self.ids = self
                    .app
                    .preview_instances_prepare(ctx, self.light, &self.flow_folder);
                self.app.preview_instances_guard(ctx, &self.ids.2);
            }
            10 => self.app.preview_instances_open(ctx, self.quick, ""),
            14 | 62 | 82 => self.app.preview_instances_open(ctx, self.quick, "重复材料"),
            40 => self
                .app
                .preview_instances_open(ctx, self.quick, "接收工作 B"),
            108 => self.app.preview_instances_open(ctx, self.quick, "教程图 B"),
            68 => self
                .app
                .preview_instances_mutate(1, &self.ids.0, &self.ids.1),
            84 => self
                .app
                .preview_instances_mutate(2, &self.ids.0, &self.ids.1),
            90 => self
                .app
                .preview_instances_mutate(3, &self.ids.0, &self.ids.1),
            100 => self.tasks = self.app.preview_instances_tasks(ctx),
            102 => self.app.preview_instances_task_open(ctx, &self.tasks.0),
            130 => self.app.preview_history_task_cancel(),
            152 => self.app.preview_instances_open(ctx, self.quick, "最长名字"),
            160 | 162 | 164 if !self.quick => self
                .app
                .preview_instances_mixed_begin(((self.frame - 160) / 2) as u8, &self.ids.0),
            _ => {}
        }
        if self.frame == 14 && !self.quick {
            self.app.preview_instances_scope_all();
        }
        ctx.style_mut(|style| style.animation_time = 0.0);
        self.app.update(ctx, frame);
        if !self.quick {
            match self.frame {
                160 => self.app.preview_instances_mixed_arm(0, &self.ids.0),
                161 => self.app.preview_instances_mixed_check(0, &self.ids.0),
                162 => self.app.preview_instances_mixed_arm(1, &self.ids.0),
                163 => self.app.preview_instances_mixed_check(1, &self.ids.0),
                164 => self.app.preview_instances_mixed_arm(2, &self.ids.0),
                165 => self.app.preview_instances_mixed_check(2, &self.ids.0),
                _ => {}
            }
        }
        if self.frame == 116 && !self.app.preview_instances_b_finished() {
            ctx.request_repaint_after(Duration::from_millis(20));
            return;
        }
        if matches!(self.frame, 18 | 25 | 28 | 30) {
            println!(
                "FOCUS frame={} {:?}",
                self.frame,
                ctx.memory(|m| m.focused())
            );
        }
        if matches!(self.frame, 25 | 72 | 88) {
            assert!(
                ctx.memory(|m| m.focused()).is_some(),
                "guarded Enter lost input focus"
            );
        }
        let phase = match self.frame {
            16 => Some(0),
            25 => Some(1),
            38 | 54 | 78 => Some(2),
            48 | 58 => Some(3),
            72 | 88 => Some(4),
            96 => Some(5),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app
                .preview_instances_check(phase, &self.ids.0, &self.ids.1, &self.ids.2);
        }
        if self.frame == 44 {
            self.click = Some(self.app.preview_instances_rect(ctx, &self.ids.2).center());
        }
        if matches!(self.frame, 118 | 128 | 146) {
            self.app.preview_instances_task_check(
                if self.frame == 118 {
                    &self.tasks.1
                } else {
                    &self.tasks.0
                },
                self.frame == 146,
            );
        }
        if matches!(self.frame, 11 | 18 | 42 | 72 | 88 | 118 | 154) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                self.shots.push(image);
            }
        }
        if self.frame == 168 {
            assert_eq!(self.shots.len(), 7);
            for (i, image) in self.shots.iter().enumerate() {
                let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.output.join(format!("instances-{i}.png")),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            println!(
                "PASS native instance search: mouse/keyboard, IME, rename/close, guards, history, real background ownership; quick={}",
                self.quick
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
    let root = std::env::temp_dir().join(format!("zi-instance-search-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("services.yml");
    let flow_folder = root.join("flows");
    std::fs::write(&config, "services: {}\n").unwrap();
    eframe::run_native(
        "Instance search acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 640.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                output,
                flow_folder,
                light: std::env::args().nth(2).as_deref() == Some("light"),
                quick: std::env::args().nth(3).as_deref() == Some("quick"),
                frame: 0,
                start: Instant::now(),
                ids: Default::default(),
                tasks: Default::default(),
                click: None,
                shots: Vec::new(),
            }))
        }),
    )
}
