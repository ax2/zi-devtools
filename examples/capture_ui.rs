//! Renders synthetic, credential-free fixtures through the real eframe renderer.
use eframe::egui;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use zi_devtools::app::DevToolsApp;
use zi_devtools::recorder::{self, AudioGains, AudioMode, Event, Region, Session};

const NAMES: [&str; 104] = [
    "home-dark",
    "home-light",
    "yaml-dark",
    "yaml-light-compact",
    "data-dark",
    "data-light",
    "files-dark",
    "files-light-compact",
    "launcher-light",
    "json-path-light",
    "json-path-dark",
    "json-diff-light",
    "json-diff-dark",
    "quality-light",
    "quality-dark",
    "cron-light",
    "cron-dark",
    "random-light",
    "random-dark",
    "unicode-light",
    "unicode-dark",
    "plugins-light",
    "plugins-dark",
    "category-light",
    "recent-dark",
    "plugin-tool-light",
    "plugin-search-dark",
    "java-trace-light",
    "java-trace-dark",
    "django-trace-light",
    "django-trace-dark",
    "directory-page-two-light",
    "java-environment-dark",
    "java-environment-light",
    "django-environment-dark",
    "django-environment-light",
    "java-threads-dark",
    "java-threads-light",
    "dependencies-dark",
    "dependencies-light",
    "migrations-dark",
    "migrations-light",
    "sql-dark",
    "sql-light",
    "gc-dark",
    "gc-light",
    "jfr-dark",
    "jfr-light",
    "spring-config-dark",
    "spring-config-light",
    "actuator-dark",
    "actuator-light",
    "django-urls-dark",
    "django-urls-light",
    "drf-dark",
    "drf-light",
    "checks-dark",
    "checks-light",
    "celery-dark",
    "celery-light",
    "tray-settings-dark",
    "tray-favorites-light",
    "tray-recent-dark",
    "tray-frequent-light",
    "file-intake-dark",
    "file-intake-light",
    "handoff-dark",
    "handoff-light",
    "transform-dark",
    "transform-light",
    "types-dark",
    "types-light",
    "columns-dark",
    "columns-light",
    "join-dark",
    "join-light",
    "tasks-dark",
    "tasks-light",
    "credentials-dark",
    "credentials-light",
    "connection-profiles-dark",
    "connection-profiles-light",
    "model-discovery-dark",
    "model-discovery-light",
    "conversation-dark",
    "conversation-light",
    "stream-dark",
    "stream-light",
    "import-conversation-dark",
    "import-conversation-light",
    "attachments-dark",
    "attachments-light",
    "library-dark",
    "library-light",
    "prompts-dark",
    "prompts-light",
    "prompt-editor-dark",
    "prompt-editor-light",
    "mcp-dark",
    "mcp-light",
    "recorder-dark",
    "recorder-light",
    "recorder-interrupted-dark",
    "recorder-interrupted-light",
];

struct Capture {
    app: DevToolsApp,
    folder: PathBuf,
    fixture: PathBuf,
    scene: usize,
    frames: usize,
    pending: bool,
    started: Instant,
    recorder_smoke: Option<Session>,
    recorder_smoke_started: Option<Instant>,
    recorder_smoke_size: Option<(u32, u32)>,
    auto_minimize_smoke_at: Option<Instant>,
    auto_minimize_recording_at: Option<Instant>,
    auto_minimize_smoke_stop_requested: bool,
    quick_smoke_phase: u8,
}
impl eframe::App for Capture {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input.events.push(egui::Event::PointerGone);
        if self.scene == 64 && self.frames == 2 {
            input.dropped_files.push(egui::DroppedFile {
                path: Some(self.fixture.with_file_name("订单数据.csv")),
                ..Default::default()
            });
        }
        if self.scene != NAMES.len() {
            return;
        }
        let event = match self.frames {
            2 => Some((egui::Key::K, egui::Modifiers::CTRL)),
            4 => Some((egui::Key::ArrowDown, egui::Modifiers::NONE)),
            6 | 14 | 22 => Some((egui::Key::Enter, egui::Modifiers::NONE)),
            10 | 18 => Some((egui::Key::K, egui::Modifiers::CTRL)),
            26 => Some((egui::Key::Enter, egui::Modifiers::CTRL)),
            _ => None,
        };
        if self.frames == 12 {
            input.events.push(egui::Event::Text("保持顺序去重".into()));
        }
        if self.frames == 20 {
            input.events.push(egui::Event::Text("django-sql".into()));
        }
        if let Some((key, modifiers)) = event {
            input.modifiers = modifiers;
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let smoke_mode = std::env::args().nth(3);
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-quick-panel-preview"
                    | "recorder-quick-panel-smoke"
                    | "recorder-quick-cancel-smoke"
            )
        ) {
            let interactive = smoke_mode.as_deref() == Some("recorder-quick-panel-smoke");
            let cancel = smoke_mode.as_deref() == Some("recorder-quick-cancel-smoke");
            if self.frames == 0 {
                self.app
                    .preview_scene(ctx, self.scene, self.fixture.clone());
                self.app
                    .preview_start_recorder_auto_minimize(&self.folder, if cancel { 5 } else { 0 })
                    .unwrap();
            }
            self.app.update(ctx, frame);
            if self.quick_smoke_phase == 0
                && self.app.preview_minimized()
                && self.app.preview_recorder_status()
                    == if cancel {
                        zi_devtools::recorder_ui::TrayRecordingStatus::Countdown
                    } else {
                        zi_devtools::recorder_ui::TrayRecordingStatus::Recording
                    }
            {
                self.app.preview_open_quick(ctx);
                self.quick_smoke_phase = 1;
            }
            if self.quick_smoke_phase == 1 && self.app.preview_panel_rendered() {
                println!("READY recorder quick panel");
                self.quick_smoke_phase = 2;
                if interactive || cancel {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(300));
                        click_quick_recorder_control(cancel);
                    });
                }
            }
            if cancel
                && self.quick_smoke_phase == 2
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Idle
                && !self.app.preview_minimized()
            {
                assert!(self.app.preview_recorder_last_file().is_none());
                println!("PASS quick panel countdown cancellation restores window");
                std::process::exit(0);
            }
            if interactive
                && self.quick_smoke_phase == 2
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Paused
            {
                println!("PASS quick panel pause");
                self.quick_smoke_phase = 3;
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    click_quick_recorder_control(false);
                });
            }
            if interactive
                && self.quick_smoke_phase == 3
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Recording
            {
                println!("PASS quick panel resume");
                self.quick_smoke_phase = 4;
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    click_quick_recorder_control(true);
                });
            }
            if !interactive
                && !cancel
                && self.quick_smoke_phase == 2
                && self.started.elapsed() > Duration::from_secs(30)
            {
                self.app.preview_stop_recorder();
                self.quick_smoke_phase = 4;
            }
            if let Some(path) = self.app.preview_recorder_last_file() {
                assert!(!self.app.preview_minimized());
                if interactive {
                    assert_eq!(self.quick_smoke_phase, 4);
                }
                fs::remove_file(path).unwrap();
                println!("PASS recorder quick panel rendered and stopped recording");
                std::process::exit(0);
            }
            assert!(self.started.elapsed() < Duration::from_secs(45));
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-auto-minimize-smoke"
                    | "recorder-auto-minimize-countdown-smoke"
                    | "recorder-auto-minimize-cancel-smoke"
            )
        ) {
            let countdown = smoke_mode.as_deref() == Some("recorder-auto-minimize-countdown-smoke");
            let cancel = smoke_mode.as_deref() == Some("recorder-auto-minimize-cancel-smoke");
            if self.frames == 0 {
                self.app.preview_scene(ctx, 100, self.fixture.clone());
                self.app
                    .preview_start_recorder_auto_minimize(
                        &self.folder,
                        if cancel {
                            5
                        } else if countdown {
                            3
                        } else {
                            0
                        },
                    )
                    .unwrap();
            }
            self.app.update(ctx, frame);
            if self.app.preview_minimized() {
                self.auto_minimize_smoke_at.get_or_insert_with(Instant::now);
            }
            if self.app.preview_recorder_status()
                == zi_devtools::recorder_ui::TrayRecordingStatus::Recording
            {
                self.auto_minimize_recording_at
                    .get_or_insert_with(Instant::now);
            }
            if !self.auto_minimize_smoke_stop_requested
                && (if cancel {
                    self.auto_minimize_smoke_at
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(1))
                } else {
                    self.auto_minimize_recording_at
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(2))
                })
            {
                self.app.preview_stop_recorder();
                self.auto_minimize_smoke_stop_requested = true;
            }
            if cancel
                && self.auto_minimize_smoke_stop_requested
                && self.app.preview_recorder_status()
                    == zi_devtools::recorder_ui::TrayRecordingStatus::Idle
                && !self.app.preview_minimized()
            {
                assert!(self.app.preview_recorder_last_file().is_none());
                println!("PASS recorder countdown cancellation restores window");
                std::process::exit(0);
            }
            if let Some(path) = self.app.preview_recorder_last_file() {
                assert!(self.auto_minimize_smoke_at.is_some());
                assert!(!self.app.preview_minimized(), "主窗口未在录制完成后恢复");
                let bytes = fs::read(&path).unwrap();
                assert!(bytes.len() > 1024 && bytes.windows(4).any(|part| part == b"moov"));
                fs::remove_file(path).unwrap();
                println!("PASS recorder auto-minimize and restore");
                std::process::exit(0);
            }
            assert!(
                self.started.elapsed() < Duration::from_secs(25),
                "自动最小化录屏超时"
            );
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if matches!(
            smoke_mode.as_deref(),
            Some(
                "recorder-smoke"
                    | "recorder-fullscreen-smoke"
                    | "recorder-dynamic-smoke"
                    | "recorder-long-smoke"
                    | "recorder-long-mix-smoke"
            )
        ) {
            let dynamic = matches!(
                smoke_mode.as_deref(),
                Some("recorder-dynamic-smoke" | "recorder-long-smoke" | "recorder-long-mix-smoke")
            );
            let record_seconds = if matches!(
                smoke_mode.as_deref(),
                Some("recorder-long-smoke" | "recorder-long-mix-smoke")
            ) {
                60
            } else if dynamic {
                12
            } else {
                2
            };
            let audio_mode = if smoke_mode.as_deref() == Some("recorder-long-mix-smoke") {
                AudioMode::SystemAndMicrophone
            } else if dynamic {
                AudioMode::Microphone
            } else {
                AudioMode::None
            };
            if self.frames == 0 {
                let display = recorder::primary_display().unwrap();
                println!(
                    "eframe primary display: {}x{}",
                    display.width, display.height
                );
                let region = Region {
                    x: 0,
                    y: 0,
                    width: if smoke_mode.as_deref() == Some("recorder-fullscreen-smoke") {
                        display.width & !1
                    } else {
                        display.width.min(640) & !1
                    },
                    height: if smoke_mode.as_deref() == Some("recorder-fullscreen-smoke") {
                        display.height & !1
                    } else {
                        display.height.min(360) & !1
                    },
                };
                self.recorder_smoke_size = Some((region.width, region.height));
                self.recorder_smoke = Some(
                    recorder::start_on_display(
                        region,
                        self.folder.join(format!(
                            "recorder-eframe-smoke-{}.mp4",
                            uuid::Uuid::new_v4()
                        )),
                        audio_mode,
                        AudioGains::default(),
                        display,
                    )
                    .unwrap(),
                );
            }
            if let Some(session) = &self.recorder_smoke {
                for event in session.events.try_iter() {
                    match event {
                        Event::Started => self.recorder_smoke_started = Some(Instant::now()),
                        Event::Finished(result) => {
                            let path = result.unwrap();
                            let data = fs::read(&path).unwrap();
                            assert!(data.len() > 1024 && data.windows(4).any(|v| v == b"moov"));
                            let track_size = data
                                .windows(4)
                                .enumerate()
                                .find_map(|(pos, tag)| {
                                    if tag != b"tkhd" || pos < 4 {
                                        return None;
                                    }
                                    let start = pos - 4;
                                    let size = u32::from_be_bytes(data[start..pos].try_into().ok()?)
                                        as usize;
                                    let end = start.checked_add(size)?;
                                    if size < 16 || end > data.len() {
                                        return None;
                                    }
                                    let width =
                                        u32::from_be_bytes(data[end - 8..end - 4].try_into().ok()?)
                                            >> 16;
                                    let height =
                                        u32::from_be_bytes(data[end - 4..end].try_into().ok()?)
                                            >> 16;
                                    Some((width, height))
                                })
                                .expect("MP4 track dimensions");
                            assert_eq!(Some(track_size), self.recorder_smoke_size);
                            if dynamic {
                                let tracks = mp4_track_durations(&data);
                                let video =
                                    tracks.iter().find(|(kind, _)| kind == b"vide").unwrap().1;
                                let audio =
                                    tracks.iter().find(|(kind, _)| kind == b"soun").unwrap().1;
                                assert!(
                                    (video - record_seconds as f64).abs() < 1.5,
                                    "video={video:.3}s expected={record_seconds}s"
                                );
                                assert!(
                                    (audio - video).abs() < 0.75,
                                    "audio={audio:.3}s video={video:.3}s"
                                );
                                println!(
                                    "MP4 tracks: mode={audio_mode:?} video={video:.3}s audio={audio:.3}s"
                                );
                            }
                            fs::remove_file(path).unwrap();
                            println!("PASS eframe recorder smoke");
                            std::process::exit(0);
                        }
                        Event::Interrupted { reason, .. } => {
                            panic!("recorder smoke interrupted: {reason}");
                        }
                    }
                }
                if self
                    .recorder_smoke_started
                    .is_some_and(|at| at.elapsed() >= Duration::from_secs(record_seconds))
                {
                    session.stop.store(true, Ordering::Release);
                }
            }
            if self.started.elapsed() > Duration::from_secs(record_seconds + 25) {
                panic!("eframe recorder smoke timed out");
            }
            if dynamic {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    let phase = self.started.elapsed().as_secs_f32() * 2.0;
                    ui.painter()
                        .rect_filled(rect, 0.0, egui::Color32::from_rgb(23, 31, 46));
                    let x = rect.left() + 100.0 + (phase.sin() + 1.0) * 140.0;
                    ui.painter().circle_filled(
                        egui::pos2(x, rect.top() + 120.0),
                        55.0,
                        egui::Color32::from_rgb(51, 208, 174),
                    );
                    ui.label(format!("Animated capture fixture · frame {}", self.frames));
                });
            } else {
                self.app.update(ctx, frame);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if std::env::args().nth(3).as_deref() == Some("recorder-interaction") {
            if self.frames == 0 {
                self.app.preview_scene(ctx, 100, self.fixture.clone());
                self.app.preview_begin_recorder_selection();
                std::thread::spawn(drag_recorder_region);
            }
            self.app.update(ctx, frame);
            if let Some(region) = self.app.preview_recorder_region() {
                if region.x != 180 || region.y != 120 {
                    println!("PASS recorder drag: {region:?}");
                    std::process::exit(0);
                }
            }
            if self.started.elapsed() > Duration::from_secs(30) {
                panic!("recorder drag timed out");
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if self.started.elapsed() > Duration::from_secs(240) {
            panic!("UI capture timed out before verification completed");
        }
        let screenshots = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for image in screenshots {
            let file =
                fs::File::create(self.folder.join(format!("{}.png", NAMES[self.scene]))).unwrap();
            let mut encoder = png::Encoder::new(file, image.size[0] as u32, image.size[1] as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let bytes = image
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect::<Vec<_>>();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&bytes)
                .unwrap();
            println!(
                "Captured {}: {}x{}",
                NAMES[self.scene], image.size[0], image.size[1]
            );
            if std::env::args().nth(3).as_deref() == Some("single") {
                std::process::exit(0);
            }
            self.scene += 1;
            self.frames = 0;
            self.pending = false;
        }
        if self.scene == NAMES.len() {
            if self.frames == 0 {
                self.app.preview_keyboard_fixture();
                self.app.preview_scene(ctx, 0, self.fixture.clone());
            }
            self.app.update(ctx, frame);
            if self.frames == 3 {
                assert!(self.app.preview_navigation().0, "Ctrl K opens launcher");
            }
            if self.frames == 8 {
                assert_eq!(
                    self.app.preview_navigation(),
                    (false, true),
                    "ArrowDown + Enter opens Files"
                );
                println!("PASS keyboard: Ctrl K, ArrowDown, Enter navigation");
            }
            if self.frames == 16 {
                assert!(
                    self.app.preview_plugin_navigation(),
                    "Ctrl K routes installed plugin tool"
                );
                println!("PASS keyboard: plugin search and navigation");
            }
            if self.frames == 24 {
                assert!(
                    self.app.preview_framework_navigation().0,
                    "Ctrl K routes Django SQL"
                );
            }
            if self.frames == 42 {
                let (navigated, completed) = self.app.preview_framework_navigation();
                assert!(
                    navigated && completed,
                    "Ctrl Enter executes diagnostic background task"
                );
                println!(
                    "PASS keyboard: diagnostic search, navigation, Ctrl Enter background completion"
                );
                self.app.preview_import_routes();
                self.app.preview_hidden_panel(ctx, false);
            }
            if self.frames >= 90 && std::env::args().nth(3).as_deref() == Some("panel") {
                ctx.request_repaint_after(Duration::from_millis(60));
                return;
            }
            if self.frames == 95 {
                assert!(
                    self.app.preview_hidden(),
                    "Quick panel must keep main workbench hidden"
                );
                assert!(
                    self.app.preview_panel_rendered(),
                    "Hidden workbench wakes and renders child viewport"
                );
                self.app.preview_hidden_panel(ctx, true);
            }
            if self.frames == 145 {
                assert!(self.app.preview_hidden());
                assert!(self.app.preview_panel_rendered());
                println!(
                    "PASS hidden-workbench wake, independent quick panel dark/light, file import routes"
                );
                std::process::exit(0);
            }
            self.frames += 1;
            ctx.request_repaint_after(Duration::from_millis(60));
            return;
        }
        if self.frames == 0 {
            self.app
                .preview_scene(ctx, self.scene, self.fixture.clone());
            let size = if (96..=99).contains(&self.scene) {
                egui::vec2(1280.0, 1180.0)
            } else if self.scene == 3 || self.scene == 7 || self.scene == 59 {
                egui::vec2(980.0, 760.0)
            } else {
                egui::vec2(1280.0, 900.0)
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        self.app.update(ctx, frame);
        self.frames += 1;
        if self.frames >= 18 && !self.pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.pending = true;
        }
        ctx.request_repaint_after(Duration::from_millis(60));
    }
}

#[cfg(windows)]
fn drag_recorder_region() {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, mouse_event},
        WindowsAndMessaging::FindWindowW,
    };
    let title: Vec<u16> = "选择录制区域 · Esc 取消\0".encode_utf16().collect();
    let deadline = Instant::now() + Duration::from_secs(12);
    while unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }.is_null() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    unsafe {
        SetCursorPos(200, 200);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
    }
    for step in 1..=8 {
        unsafe {
            SetCursorPos(200 + step * 60, 200 + step * 35);
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    unsafe {
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
}
#[cfg(windows)]
fn click_quick_recorder_control(stop: bool) {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, mouse_event},
        WindowsAndMessaging::{FindWindowW, GetWindowRect, SetCursorPos},
    };
    let title: Vec<u16> = "Zi DevTools · 快捷面板\0".encode_utf16().collect();
    let window = unsafe { FindWindowW(std::ptr::null_mut(), title.as_ptr()) };
    assert!(!window.is_null(), "快捷面板窗口未创建");
    let mut rect = unsafe { std::mem::zeroed() };
    assert_ne!(unsafe { GetWindowRect(window, &mut rect) }, 0);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let x = rect.left + width * if stop { 87 } else { 69 } / 100;
    let y = rect.top + height * 17 / 100;
    unsafe {
        SetCursorPos(x, y);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
}
#[cfg(not(windows))]
fn click_quick_recorder_control(_stop: bool) {}
#[cfg(not(windows))]
fn drag_recorder_region() {}
fn mp4_boxes<'a>(data: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if size < 8 || offset + size > data.len() {
            break;
        }
        if &data[offset + 4..offset + 8] == kind {
            result.push(&data[offset + 8..offset + size]);
        }
        offset += size;
    }
    result
}
fn mp4_track_durations(data: &[u8]) -> Vec<([u8; 4], f64)> {
    let mut result = Vec::new();
    for movie in mp4_boxes(data, b"moov") {
        for track in mp4_boxes(movie, b"trak") {
            for media in mp4_boxes(track, b"mdia") {
                let Some(handler) = mp4_boxes(media, b"hdlr").into_iter().next() else {
                    continue;
                };
                let Some(header) = mp4_boxes(media, b"mdhd").into_iter().next() else {
                    continue;
                };
                if handler.len() < 12 || header.len() < 20 {
                    continue;
                }
                let kind = handler[8..12].try_into().unwrap();
                let (timescale, duration) = if header[0] == 1 && header.len() >= 32 {
                    (
                        u32::from_be_bytes(header[20..24].try_into().unwrap()),
                        u64::from_be_bytes(header[24..32].try_into().unwrap()),
                    )
                } else {
                    (
                        u32::from_be_bytes(header[12..16].try_into().unwrap()),
                        u32::from_be_bytes(header[16..20].try_into().unwrap()) as u64,
                    )
                };
                if timescale > 0 {
                    result.push((kind, duration as f64 / timescale as f64));
                }
            }
        }
    }
    result
}
fn main() -> Result<(), eframe::Error> {
    let folder = PathBuf::from(std::env::args().nth(1).expect("capture output directory"));
    fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let fixture = folder.join("sample.txt");
    fs::write(&fixture, b"abc").unwrap();
    fs::write(folder.join("订单数据.csv"), "name,count\nexample,3\n").unwrap();
    let config = folder.join("services.yml");
    fs::write(&config,format!("state_dir: '{}'\nservices:\n  demo:\n    name: Demo fixture\n    repo: '{}'\n    command: 'echo fixture'\n",folder.join("state").display(),folder.display())).unwrap();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Zi DevTools — UI preview fixture")
        .with_inner_size([1280.0, 900.0]);
    if std::env::args().nth(3).as_deref() == Some("recorder-dynamic-smoke") {
        viewport = viewport.with_position([0.0, 0.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Zi DevTools UI capture",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Capture {
                app: DevToolsApp::new(cc, config, false),
                folder,
                fixture,
                scene: std::env::args()
                    .nth(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                frames: 0,
                pending: false,
                started: Instant::now(),
                recorder_smoke: None,
                recorder_smoke_started: None,
                recorder_smoke_size: None,
                auto_minimize_smoke_at: None,
                auto_minimize_recording_at: None,
                auto_minimize_smoke_stop_requested: false,
                quick_smoke_phase: 0,
            }))
        }),
    )
}
