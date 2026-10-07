//! Native confirmation buttons with injected pointer input and independently read output.
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
struct Preview {
    app: DevToolsApp,
    folder: PathBuf,
    target: PathBuf,
    sqlite: bool,
    light: bool,
    frame: u32,
    captured: u32,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        let button = match self.frame {
            30 | 31 | 80 | 81 => Some(0),
            60 | 61 => Some(1),
            110 | 111 => Some(2),
            _ => None,
        };
        if let Some(index) = button {
            let pos = self.app.preview_workflow_output_position(index);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: self.frame % 2 == 0,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(
            self.start.elapsed() < Duration::from_secs(40),
            "fixture timeout"
        );
        if self.frame == 0 {
            self.app
                .preview_workflow_output_prepare(ctx, &self.target, self.sqlite, self.light);
        }
        self.app.update(ctx, frame);
        if self.frame == 45 {
            assert!(self.app.preview_workflow_output_check(1));
            assert!(!self.target.exists());
        }
        if self.frame == 70 {
            assert!(self.app.preview_workflow_output_check(2));
            assert!(!self.target.exists());
        }
        if self.frame == 140 {
            assert!(self.app.preview_workflow_output_check(3));
            if self.sqlite {
                let db = rusqlite::Connection::open_with_flags(
                    &self.target,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                let row: (String,String,i64,i64) = db.query_row("SELECT 编号,名称,数量,(SELECT COUNT(*) FROM workflow_result) FROM workflow_result ORDER BY 编号 LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(row, ("001".into(), "Zi Tools".into(), 2, 2));
            } else {
                let mut csv = csv::Reader::from_path(&self.target).unwrap();
                let rows: Vec<_> = csv.records().map(Result::unwrap).collect();
                assert_eq!(rows.len(), 2);
                assert_eq!(&rows[0][0], "001");
                assert_eq!(&rows[0][1], "Zi Tools");
                assert_eq!(&rows[0][2], "2");
            }
        }
        if matches!(self.frame, 20 | 45 | 145) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                image::save_buffer(
                    self.folder.join(format!("output-{}.png", self.captured)),
                    &bytes,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                self.captured += 1;
            }
        }
        if self.frame == 160 {
            assert_eq!(self.captured, 3);
            println!(
                "PASS native workflow output sqlite={} light={}: actual review/back/confirm; no write before confirm; independently read complete output; original table and proposal retained",
                self.sqlite, self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    let sqlite = std::env::args().nth(2).as_deref() == Some("sqlite");
    let light = std::env::args().nth(3).as_deref() == Some("light");
    let target = folder.join(if sqlite {
        "result.sqlite"
    } else {
        "result.csv"
    });
    assert!(!target.exists());
    eframe::run_native(
        "Workflow output acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                target,
                sqlite,
                light,
                frame: 0,
                captured: 0,
                start: Instant::now(),
            }))
        }),
    )
}
