//! Own-window synthetic desktop fixture; it never moves the user's cursor.
#[cfg(windows)]
fn main() -> eframe::Result {
    use eframe::egui;
    use std::{
        path::PathBuf,
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    use zi_devtools::recorder::{
        self, Event, Region, Session,
        tutorial::{Pointer, Settings, ZoomMode},
    };
    struct Fixture {
        folder: PathBuf,
        title: String,
        frames: u64,
        session: Option<Session>,
        started: Option<Instant>,
        deadline: Instant,
    }
    impl eframe::App for Fixture {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let (canvas, _) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                    let p = canvas.min;
                    if self.frames == 30 {
                        println!(
                            "paint origin {p:?}, clip {:?}, scale {}",
                            ui.clip_rect(),
                            ctx.pixels_per_point()
                        );
                    }
                    let unit = 1.0 / ctx.pixels_per_point();
                    for (x, y, c) in [
                        (0.0, 0.0, egui::Color32::from_rgb(220, 30, 30)),
                        (320.0, 0.0, egui::Color32::from_rgb(30, 200, 50)),
                        (0.0, 180.0, egui::Color32::from_rgb(30, 50, 220)),
                        (320.0, 180.0, egui::Color32::from_rgb(220, 200, 30)),
                    ] {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                p + egui::vec2(x, y) * unit,
                                egui::vec2(320.0, 180.0) * unit,
                            ),
                            0.0,
                            c,
                        );
                    }
                });
            if self.frames == 15 {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    FindWindowW, GetWindowThreadProcessId, HWND_TOPMOST, SWP_NOACTIVATE,
                    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
                };
                let title: Vec<u16> = self.title.encode_utf16().chain(Some(0)).collect();
                let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                assert_eq!(pid, std::process::id());
                assert_ne!(
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            HWND_TOPMOST,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        )
                    },
                    0
                );
            }
            if self.frames == 30 {
                use windows_sys::Win32::{
                    Foundation::POINT,
                    Graphics::Gdi::ClientToScreen,
                    UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId},
                };
                let title: Vec<u16> = self.title.encode_utf16().chain(Some(0)).collect();
                let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
                assert!(!hwnd.is_null());
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                assert_eq!(pid, std::process::id());
                let mut p = POINT { x: 0, y: 0 };
                assert_ne!(unsafe { ClientToScreen(hwnd, &mut p) }, 0);
                let display = recorder::primary_display().unwrap();
                let region = Region {
                    x: u32::try_from(p.x - display.x).unwrap(),
                    y: u32::try_from(p.y - display.y).unwrap(),
                    width: 640,
                    height: 360,
                };
                assert!(region.fits(display.width, display.height));
                use windows_sys::Win32::{
                    Foundation::RECT,
                    Graphics::Dwm::DwmGetWindowAttribute,
                    UI::WindowsAndMessaging::{
                        GetWindowRect, IsIconic, IsWindowVisible, SW_SHOW, ShowWindow,
                    },
                };
                let mut rect: RECT = unsafe { std::mem::zeroed() };
                unsafe { GetWindowRect(hwnd, &mut rect) };
                let mut cloaked = 0u32;
                unsafe { DwmGetWindowAttribute(hwnd, 14, (&mut cloaked as *mut u32).cast(), 4) };
                println!(
                    "visible={} iconic={} cloak={} rect={},{},{},{}",
                    unsafe { IsWindowVisible(hwnd) },
                    unsafe { IsIconic(hwnd) },
                    cloaked,
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom
                );
                unsafe { ShowWindow(hwnd, SW_SHOW) };
                if let Some(gl) = frame.gl() {
                    use eframe::glow::HasContext;
                    let mut bounds = [0; 4];
                    let mut pixel = [0u8; 4];
                    unsafe {
                        gl.get_parameter_i32_slice(eframe::glow::VIEWPORT, &mut bounds);
                        gl.read_pixels(
                            100,
                            bounds[3] - 81,
                            1,
                            1,
                            eframe::glow::RGBA,
                            eframe::glow::UNSIGNED_BYTE,
                            eframe::glow::PixelPackData::Slice(Some(&mut pixel)),
                        )
                    };
                    println!("own framebuffer red sample={pixel:?}");
                }
                use windows_sys::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
                let dc = unsafe { GetDC(std::ptr::null_mut()) };
                assert!(!dc.is_null());
                let color = unsafe { GetPixel(dc, p.x + 100, p.y + 80) };
                unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
                println!(
                    "own-client source origin {},{}; sampled color #{:06x}",
                    p.x, p.y, color
                );
                println!(
                    "Encoding fixture replaces acquired pixels with known synthetic quadrants; desktop composition acceptance is separate."
                );
                let control = std::sync::Arc::new(recorder::tutorial::Control::default());
                control
                    .set(Settings {
                        mode: ZoomMode::Fixed,
                        focus: [0.25, 0.25],
                        smooth: false,
                        ..Settings::default()
                    })
                    .unwrap();
                control.preview_source(
                    (0..360)
                        .flat_map(|y| {
                            (0..640).flat_map(move |x| match (x < 320, y < 180) {
                                (true, true) => [30, 30, 220, 255],
                                (false, true) => [50, 200, 30, 255],
                                (true, false) => [220, 50, 30, 255],
                                (false, false) => [30, 200, 220, 255],
                            })
                        })
                        .collect(),
                );
                control.preview_pointer(Pointer {
                    position: Some([480.0, 90.0]),
                    buttons: 0,
                });
                let session = recorder::preview_start_controlled(
                    region,
                    self.folder.join("tutorial-native.mp4"),
                    display,
                    control,
                )
                .unwrap();
                session.tutorial.preview_pointer(Pointer {
                    position: Some([480.0, 90.0]),
                    buttons: 0,
                });
                self.session = Some(session);
            }
            if let Some(session) = &self.session {
                for event in session.events.try_iter() {
                    match event {
                        Event::Started => self.started = Some(Instant::now()),
                        Event::Finished(result) => {
                            let path = result.unwrap();
                            assert!(path.metadata().unwrap().len() > 1024);
                            println!(
                                "PASS native synthetic tutorial MP4: {} (decode verification required)",
                                path.display()
                            );
                            std::process::exit(0);
                        }
                        Event::Interrupted { reason, .. } => {
                            panic!("unexpected interruption: {reason}")
                        }
                    }
                }
                if let Some(started) = self.started {
                    let seconds = started.elapsed().as_secs_f32();
                    session.pause.set_paused((2.0..3.0).contains(&seconds));
                    let settings = if seconds < 1.0 {
                        Settings {
                            mode: ZoomMode::Fixed,
                            focus: [0.25, 0.25],
                            smooth: false,
                            ..Settings::default()
                        }
                    } else if seconds < 2.0 {
                        Settings {
                            mode: ZoomMode::Fixed,
                            focus: [0.75, 0.75],
                            smooth: false,
                            ..Settings::default()
                        }
                    } else if seconds < 4.2 {
                        Settings {
                            mode: ZoomMode::Follow,
                            smooth: false,
                            highlight: true,
                            clicks: true,
                            ..Settings::default()
                        }
                    } else {
                        Settings::default()
                    };
                    session.tutorial.set(settings).unwrap();
                    session.tutorial.preview_pointer(Pointer {
                        position: Some([480.0, 90.0]),
                        buttons: u8::from((3.3..3.4).contains(&seconds)),
                    });
                    if seconds >= 5.2 {
                        session.stop.store(true, Ordering::Release);
                    }
                }
            }
            assert!(
                self.deadline.elapsed() < Duration::from_secs(35),
                "native fixture timeout"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(10));
        }
    }
    let folder = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&folder).unwrap();
    assert!(
        !folder.join("tutorial-native.mp4").exists(),
        "use a fresh fixture directory"
    );
    let title = format!("Zi tutorial fixture {}", uuid::Uuid::new_v4());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_position([100.0, 100.0])
            .with_inner_size([700.0, 450.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title.clone(),
        options,
        Box::new(move |_| {
            Ok(Box::new(Fixture {
                folder,
                title,
                frames: 0,
                session: None,
                started: None,
                deadline: Instant::now(),
            }))
        }),
    )
}
#[cfg(not(windows))]
fn main() {}
