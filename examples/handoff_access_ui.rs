//! Native handoff personal access filters and preserved work fixture.
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
    frame: u32,
    captured: u8,
    started: Instant,
    pointer: Option<egui::Pos2>,
}

impl eframe::App for Preview {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        // Deterministic fixture input: desktop mouse/keyboard activity must not
        // be mixed with the explicitly injected UI actions. Keep render evidence.
        input
            .events
            .retain(|event| matches!(event, egui::Event::Screenshot { .. }));
        input.modifiers = egui::Modifiers::NONE;
        if self.frame == 66 {
            input
                .events
                .push(egui::Event::Text("unmatched987654".into()));
        }
        let scope = match self.frame {
            20 | 21 => Some("收藏"),
            40 | 41 => Some("最近"),
            50 | 51 => Some("常用"),
            _ => None,
        };
        let position = if let Some(scope) = scope {
            ctx.data(|data| {
                data.get_temp::<egui::Rect>(egui::Id::new(("handoff-scope-proof", scope)))
            })
            .map(|r| r.center())
        } else {
            match self.frame {
                30 | 31 => ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new((
                            "handoff-target-proof",
                            "数据工作台 · CSV",
                        )))
                    })
                    .map(|r| r.center()),
                60 | 61 => ctx
                    .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("handoff-query-proof")))
                    .map(|r| r.center()),
                84 | 85 => ctx
                    .data(|data| {
                        data.get_temp::<egui::Rect>(egui::Id::new("handoff-clear-filter-proof"))
                    })
                    .map(|r| r.center()),
                130 | 131 => Some(self.app.preview_handoff_discovery_position(1)),
                _ => None,
            }
        };
        if let Some(position) = position {
            let pressed = matches!(self.frame, 20 | 30 | 40 | 50 | 60 | 84 | 130);
            if pressed {
                self.pointer = Some(position);
            }
            let position = if pressed {
                position
            } else {
                self.pointer.take().expect("press precedes release")
            };
            input.events.push(egui::Event::PointerMoved(position));
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.started.elapsed() < Duration::from_secs(35),
            "delimited handoff UI fixture timed out"
        );
        if self.frame == 0 {
            self.app.preview_handoff_access_prepare(ctx, self.light);
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.app.update(ctx, frame);
        let phase = match self.frame {
            10 => Some(0),
            24 => Some(1),
            34 => Some(2),
            44 => Some(3),
            54 => Some(4),
            74 => Some(5),
            94 => Some(6),
            155 => Some(7),
            _ => None,
        };
        if let Some(phase) = phase {
            self.app.preview_handoff_access_check(phase);
            if phase < 7 {
                let confirm = self.app.preview_handoff_discovery_position(1);
                assert!(
                    ctx.screen_rect().contains(confirm),
                    "confirmation must stay inside viewport"
                );
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.frame == 170 && std::env::args().nth(3).as_deref() == Some("visits") {
            self.app.preview_handoff_visit_check();
        }
        if self.frame == 180 {
            assert_eq!(self.captured, 8);
            println!(
                "PASS handoff favorites/recent/frequent, selected target preserved, empty query reset, explicit new instance: light={}",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let pixels: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                image::save_buffer(
                    self.folder.join(format!(
                        "handoff-access-{}-{}.png",
                        if self.light { "light" } else { "dark" },
                        self.captured + 1
                    )),
                    &pixels,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}

fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    let light = std::env::args().nth(2).as_deref() == Some("light");
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let temp = std::env::temp_dir().join(format!("zi-handoff-access-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let config = temp.join("services.yml");
    std::fs::write(&config, "services: {}\n").unwrap();

    // The disposable empty service config is retained for inspection.
    eframe::run_native(
        "Handoff access acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([980.0, if light { 740.0 } else { 640.0 }]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, config, false),
                folder,
                light,
                frame: 0,
                captured: 0,
                started: Instant::now(),
                pointer: None,
            }))
        }),
    )
}
