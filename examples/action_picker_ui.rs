//! Real native alias search, focused-key selection and typed composition.
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
    started: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if matches!(self.tick, 20 | 21) {
            let (rect, clip) = ctx
                .data(|data| {
                    data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("flow-picker-search"))
                })
                .expect("search rendered");
            assert!(
                clip.contains_rect(rect),
                "search visible: {rect:?} / {clip:?}"
            );
            input.events.push(egui::Event::PointerMoved(rect.center()));
            input.events.push(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: self.tick == 20,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let text = match self.tick {
            30 => Some("ＣＳＶ 解析"),
            60 => Some("去空白"),
            90 => Some("列结构 导出"),
            _ => None,
        };
        if let Some(text) = text {
            input.events.push(egui::Event::Text(text.into()));
        }
        let key = match self.tick {
            35 | 36 => Some((egui::Key::ArrowDown, egui::Modifiers::NONE)),
            38 | 39 => Some((egui::Key::ArrowUp, egui::Modifiers::NONE)),
            45 | 46 | 75 | 76 | 105 | 106 => Some((egui::Key::Enter, egui::Modifiers::NONE)),
            125 | 126 => Some((egui::Key::Enter, egui::Modifiers::CTRL)),
            _ => None,
        };
        if let Some((key, modifiers)) = key {
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: matches!(self.tick, 35 | 38 | 45 | 75 | 105 | 125),
                repeat: false,
                modifiers,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.started.elapsed() < Duration::from_secs(35));
        if self.tick == 0 {
            self.app.preview_action_picker_prepare(ctx, self.light);
        }
        self.app.update(ctx, frame);
        if self.tick == 15 {
            let viewport = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("flow-picker-viewport")))
                .expect("action list rendered");
            assert!(
                viewport.height() >= 179.0,
                "action list collapsed: {viewport:?}"
            );
            let (_, clip) = ctx
                .data(|data| {
                    data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::new("flow-picker-search"))
                })
                .expect("search rendered");
            assert!(
                clip.contains_rect(viewport),
                "list must be visible: {viewport:?} / {clip:?}"
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let phase = match self.tick {
            40 => Some(0),
            55 => Some(1),
            85 => Some(2),
            115 => Some(3),
            160 => Some(4),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app.preview_action_picker_check(phase);
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
                    self.folder.join(format!("picker-{}.png", self.shots)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.shots += 1;
            }
        }
        if self.tick == 180 {
            assert_eq!(self.shots, 6);
            println!(
                "PASS native fullwidth/multi-term alias search, focused arrows/Enter compose CSV -> trim -> schema; CtrlEnter explicit run, typed result and original drafts retained; light={}",
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
        "Action discovery acceptance",
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
            }))
        }),
    )
}
