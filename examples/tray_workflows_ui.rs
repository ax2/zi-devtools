//! Isolated native secondary viewport screenshot and injected Enter acceptance.
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
    frame: usize,
    start: Instant,
}
impl eframe::App for Preview {
    fn raw_input_hook(&mut self, _: &egui::Context, input: &mut egui::RawInput) {
        input.system_theme = Some(if self.light {
            egui::Theme::Light
        } else {
            egui::Theme::Dark
        });
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        assert!(self.start.elapsed() < Duration::from_secs(30));
        if self.frame == 0 {
            self.app
                .preview_tray_workflow_prepare(ctx, &self.folder, self.light);
        }
        self.app.update(ctx, frame);
        if self.frame == 10 {
            capture_owned_panel(&self.folder.join("tray-workflows.png"));
        }
        if self.frame == 100 {
            self.app.preview_tray_workflow_check(self.light);
            println!(
                "PASS native tray workflows light={}: secondary viewport screenshot; injected Enter restores ordinary review; original full table and proposal retained; main theme unchanged",
                self.light
            );
            self.app.preview_tray_workflow_finish(ctx);
        }
        self.frame += 1;
        ctx.request_repaint_after(Duration::from_millis(35));
    }
}
fn main() -> eframe::Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).expect("isolated fixture folder"));
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("services.yml"), "services: {}\n").unwrap();
    std::fs::write(folder.join("sample.txt"), "fixture").unwrap();
    let light = std::env::args().nth(2).as_deref() == Some("light");
    eframe::run_native(
        "Tray workflow acceptance",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 740.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(Preview {
                app: DevToolsApp::new(cc, folder.join("services.yml"), false),
                folder,
                light,
                frame: 0,
                start: Instant::now(),
            }))
        }),
    )
}

#[cfg(windows)]
fn capture_owned_panel(path: &std::path::Path) {
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
    let title: Vec<u16> = "Zi DevTools · 快捷面板"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        assert!(!hwnd.is_null());
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        assert_eq!(
            pid,
            std::process::id(),
            "capture only this fixture's window"
        );
        let mut rect = std::mem::zeroed::<RECT>();
        assert_ne!(GetClientRect(hwnd, &mut rect), 0);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        assert!(width > 0 && height > 0 && width < 2000 && height < 2000);
        let dc = GetDC(hwnd);
        assert!(!dc.is_null());
        let memory = CreateCompatibleDC(dc);
        assert!(!memory.is_null());
        let bitmap = CreateCompatibleBitmap(dc, width, height);
        assert!(!bitmap.is_null());
        let old = SelectObject(memory, bitmap);
        assert_ne!(BitBlt(memory, 0, 0, width, height, dc, 0, 0, SRCCOPY), 0);
        SelectObject(memory, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bytes = vec![0u8; (width * height * 4) as usize];
        assert_eq!(
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                bytes.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS
            ),
            height
        );
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(hwnd, dc);
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        image::save_buffer(
            path,
            &bytes,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}
#[cfg(not(windows))]
fn capture_owned_panel(_: &std::path::Path) {
    panic!("Windows fixture only");
}
