use crate::recorder::{
    self, AudioGains, AudioMode, DisplayInfo, Event, RecordingQuality, Region, Session,
};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayRecordingStatus {
    Idle,
    Countdown,
    Starting,
    Recording,
    Paused,
    Saving,
}

fn next_output_path(folder: &Path) -> PathBuf {
    let stem = format!(
        "Zi-Recording-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    available_named_path(folder, &stem)
}

fn available_named_path(folder: &Path, stem: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.mp4"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..=10_000 {
        let candidate = folder.join(format!("{stem}-{suffix}.mp4"));
        if !candidate.exists() {
            return candidate;
        }
    }
    folder.join(format!("{stem}-{}.mp4", uuid::Uuid::new_v4()))
}

fn place_selection_overlay(display: &DisplayInfo) -> anyhow::Result<()> {
    place_capture_overlay(display, "选择录制区域 · Esc 取消").map(|_| ())
}

pub(crate) fn place_capture_overlay(display: &DisplayInfo, title: &str) -> anyhow::Result<bool> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetWindowRect, GetWindowThreadProcessId, SWP_NOACTIVATE, SWP_NOZORDER,
        SetWindowPos,
    };
    let title: Vec<u16> = format!("{title}\0").encode_utf16().collect();
    let mut window = std::ptr::null_mut();
    loop {
        window = unsafe {
            FindWindowExW(
                std::ptr::null_mut(),
                window,
                std::ptr::null(),
                title.as_ptr(),
            )
        };
        if window.is_null() {
            return Ok(false);
        }
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(window, &mut owner) };
        if owner == std::process::id() {
            break;
        }
    }
    let mut rect = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        anyhow::bail!("读取框选窗口位置失败：{}", std::io::Error::last_os_error());
    }
    if (
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    ) == (
        display.x,
        display.y,
        display.width as i32,
        display.height as i32,
    ) {
        return Ok(true);
    }
    if unsafe {
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            display.x,
            display.y,
            display.width as i32,
            display.height as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    } == 0
    {
        anyhow::bail!("定位框选窗口失败：{}", std::io::Error::last_os_error());
    }
    Ok(true)
}

#[cfg(windows)]
pub(crate) fn capture_display_snapshot(display: &DisplayInfo) -> anyhow::Result<egui::ColorImage> {
    use anyhow::{bail, ensure};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    let width = usize::try_from(display.width)?;
    let height = usize::try_from(display.height)?;
    ensure!(
        width > 0 && height > 0 && width.saturating_mul(height) <= 16_000_000,
        "屏幕太大，无法安全生成框选预览"
    );
    let screen = unsafe { GetDC(std::ptr::null_mut()) };
    if screen.is_null() {
        bail!("无法读取当前屏幕画面");
    }
    let memory = unsafe { CreateCompatibleDC(screen) };
    let bitmap = if memory.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { CreateCompatibleBitmap(screen, display.width as i32, display.height as i32) }
    };
    let result = (|| {
        ensure!(
            !memory.is_null() && !bitmap.is_null(),
            "无法创建框选画面缓冲"
        );
        let old = unsafe { SelectObject(memory, bitmap as _) };
        ensure!(!old.is_null(), "无法选择框选画面缓冲");
        let copied = unsafe {
            BitBlt(
                memory,
                0,
                0,
                display.width as i32,
                display.height as i32,
                screen,
                display.x,
                display.y,
                SRCCOPY | CAPTUREBLT,
            )
        } != 0;
        unsafe { SelectObject(memory, old) };
        ensure!(copied, "无法读取所选屏幕的当前内容");
        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader.biSize = std::mem::size_of_val(&info.bmiHeader) as u32;
        info.bmiHeader.biWidth = display.width as i32;
        info.bmiHeader.biHeight = -(display.height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = vec![0u8; width * height * 4];
        ensure!(
            unsafe {
                GetDIBits(
                    screen,
                    bitmap,
                    0,
                    display.height,
                    pixels.as_mut_ptr().cast(),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            } == display.height as i32,
            "读取框选画面像素失败"
        );
        for rgba in pixels.chunks_exact_mut(4) {
            rgba.swap(0, 2);
            rgba[3] = 255;
        }
        Ok(egui::ColorImage::from_rgba_unmultiplied(
            [width, height],
            &pixels,
        ))
    })();
    unsafe {
        if !bitmap.is_null() {
            DeleteObject(bitmap as _);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(std::ptr::null_mut(), screen);
    }
    result
}

pub struct RecorderState {
    displays_loaded: bool,
    displays: Vec<DisplayInfo>,
    display: Option<DisplayInfo>,
    selecting: bool,
    selection_texture: Option<egui::TextureHandle>,
    drag_start: Option<egui::Pos2>,
    region: Option<Region>,
    output: String,
    audio: AudioMode,
    gains: AudioGains,
    quality: RecordingQuality,
    size_preview_minutes: u16,
    countdown_seconds: u64,
    countdown_deadline: Option<Instant>,
    auto_minimize: bool,
    auto_stop_minutes: u16,
    minimize_requested: bool,
    restore_after_finish: bool,
    restore_requested: bool,
    session: Option<Session>,
    started: Option<Instant>,
    pause_started: Option<Instant>,
    paused_time: Duration,
    status: String,
    error: bool,
    last_file: Option<PathBuf>,
    #[cfg(feature = "ui-preview")]
    preview_levels: Option<(u8, u8)>,
}

impl Default for RecorderState {
    fn default() -> Self {
        Self {
            displays_loaded: false,
            displays: Vec::new(),
            display: None,
            selecting: false,
            selection_texture: None,
            drag_start: None,
            region: None,
            output: String::new(),
            audio: AudioMode::None,
            gains: AudioGains::default(),
            quality: RecordingQuality::default(),
            size_preview_minutes: 5,
            countdown_seconds: 3,
            countdown_deadline: None,
            auto_minimize: true,
            auto_stop_minutes: 0,
            minimize_requested: false,
            restore_after_finish: false,
            restore_requested: false,
            session: None,
            started: None,
            pause_started: None,
            paused_time: Duration::ZERO,
            status: String::new(),
            error: false,
            last_file: None,
            #[cfg(feature = "ui-preview")]
            preview_levels: None,
        }
    }
}

impl RecorderState {
    pub fn quality(&self) -> RecordingQuality {
        self.quality
    }
    pub fn set_quality(&mut self, quality: RecordingQuality) {
        self.quality = quality;
    }
    pub fn auto_minimize(&self) -> bool {
        self.auto_minimize
    }
    pub fn set_auto_minimize(&mut self, enabled: bool) {
        self.auto_minimize = enabled;
    }
    pub fn auto_stop_minutes(&self) -> u16 {
        self.auto_stop_minutes
    }
    pub fn set_auto_stop_minutes(&mut self, minutes: u16) {
        self.auto_stop_minutes = if [0, 1, 5, 15, 30, 60].contains(&minutes) {
            minutes
        } else {
            0
        };
    }
    fn request_auto_minimize(&mut self, tray_available: bool) {
        if self.auto_minimize && tray_available {
            self.minimize_requested = true;
            self.restore_after_finish = true;
        }
    }
    fn finish_auto_minimize(&mut self) {
        self.minimize_requested = false;
        if self.restore_after_finish {
            self.restore_requested = true;
            self.restore_after_finish = false;
        }
    }
    pub fn take_minimize_request(&mut self) -> bool {
        std::mem::take(&mut self.minimize_requested)
    }
    pub fn take_restore_request(&mut self) -> bool {
        std::mem::take(&mut self.restore_requested)
    }
    fn refresh_displays(&mut self) {
        self.displays_loaded = true;
        match recorder::enumerate_displays() {
            Ok(displays) => {
                let retained = self.display.as_ref().and_then(|selected| {
                    displays
                        .iter()
                        .find(|display| display.device_name == selected.device_name)
                        .filter(|display| {
                            (display.x, display.y, display.width, display.height)
                                == (selected.x, selected.y, selected.width, selected.height)
                        })
                        .cloned()
                });
                if self.display.is_some() && retained.is_none() {
                    self.region = None;
                    self.status = "显示器布局已变化，请重新选择录制区域".into();
                    self.error = true;
                }
                self.display = retained.or_else(|| displays.first().cloned());
                self.displays = displays;
            }
            Err(error) => {
                self.displays.clear();
                self.display = None;
                self.region = None;
                self.status = format!("读取显示器失败：{error:#}");
                self.error = true;
            }
        }
    }
    fn choose_display(&mut self, display: DisplayInfo) {
        if self.display.as_ref().map(|current| &current.device_name) != Some(&display.device_name) {
            self.region = None;
            self.status = "已切换显示器，请重新框选区域".into();
            self.error = false;
        }
        self.display = Some(display);
    }
    pub fn tray_status(&self) -> TrayRecordingStatus {
        if self.countdown_deadline.is_some() {
            return TrayRecordingStatus::Countdown;
        }
        let Some(session) = &self.session else {
            return TrayRecordingStatus::Idle;
        };
        if session.stop.load(Ordering::Acquire) {
            TrayRecordingStatus::Saving
        } else if self.started.is_none() {
            TrayRecordingStatus::Starting
        } else if session.pause.is_paused() {
            TrayRecordingStatus::Paused
        } else {
            TrayRecordingStatus::Recording
        }
    }

    pub fn elapsed_duration(&self) -> Option<Duration> {
        self.started.map(|started| {
            started
                .elapsed()
                .saturating_sub(self.paused_time)
                .saturating_sub(
                    self.pause_started
                        .map(|at| at.elapsed())
                        .unwrap_or_default(),
                )
        })
    }

    pub fn select_audio_mode(&mut self, mode: AudioMode) {
        if self.session.is_none() && self.countdown_deadline.is_none() {
            self.audio = mode;
        }
    }

    pub fn toggle_pause(&mut self) {
        if !matches!(
            self.tray_status(),
            TrayRecordingStatus::Recording | TrayRecordingStatus::Paused
        ) {
            return;
        }
        let Some(session) = &self.session else { return };
        let was_paused = session.pause.is_paused();
        session.pause.set_paused(!was_paused);
        if was_paused {
            if let Some(at) = self.pause_started.take() {
                self.paused_time += at.elapsed();
            }
            self.status = "正在录制…".into();
        } else {
            self.pause_started = Some(Instant::now());
            self.status = "已暂停".into();
        }
    }

    pub fn request_stop(&mut self) {
        if self.countdown_deadline.take().is_some() {
            self.status = "已取消录制".into();
            self.finish_auto_minimize();
            return;
        }
        if let Some(session) = &self.session
            && !session.stop.swap(true, Ordering::AcqRel)
        {
            self.status = "正在完成 MP4 文件…".into();
        }
    }

    pub fn request_start(&mut self, tray_available: bool) -> bool {
        if self.session.is_some() || self.countdown_deadline.is_some() {
            return false;
        }
        if !self.displays_loaded {
            self.refresh_displays();
        }
        let Some(region) = self.region else {
            self.status = "请先用鼠标框选区域或选择整个屏幕".into();
            self.error = true;
            return false;
        };
        let check = self
            .display
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("未选择显示器"))
            .and_then(|display| {
                recorder::validate_request_for_display(
                    region,
                    Path::new(self.output.trim()),
                    display,
                )
            });
        if let Err(error) = check {
            self.status = format!("无法开始录制：{error:#}");
            self.error = true;
            return false;
        }
        if self.countdown_seconds == 0 {
            self.begin_recording();
            if self.session.is_none() {
                return false;
            }
        } else {
            self.countdown_deadline =
                Some(Instant::now() + Duration::from_secs(self.countdown_seconds));
            self.status = "倒计时中，请准备录制区域".into();
            self.error = false;
        }
        self.request_auto_minimize(tray_available);
        true
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_begin_selection(&mut self, ctx: &egui::Context) {
        self.selecting = true;
        self.drag_start = Some(egui::pos2(280.0, 220.0));
        let mut pixels = Vec::with_capacity(640 * 400 * 4);
        for y in 0..400u32 {
            for x in 0..640u32 {
                pixels.extend_from_slice(&[(32 + x / 5) as u8, (72 + y / 4) as u8, 170, 255]);
            }
        }
        self.selection_texture = Some(ctx.load_texture(
            "recorder-selection-fixture",
            egui::ColorImage::from_rgba_unmultiplied([640, 400], &pixels),
            egui::TextureOptions::LINEAR,
        ));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_region(&self) -> Option<Region> {
        self.region
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_start_auto_minimize(
        &mut self,
        folder: &Path,
        countdown_seconds: u64,
    ) -> anyhow::Result<()> {
        self.refresh_displays();
        let display = self
            .display
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("无显示器"))?;
        self.region = Some(Region {
            x: 0,
            y: 0,
            width: display.width.min(640) & !1,
            height: display.height.min(360) & !1,
        });
        self.output = folder
            .join(format!("zi-autohide-smoke-{}.mp4", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned();
        self.audio = AudioMode::None;
        if countdown_seconds == 0 {
            self.begin_recording();
            if self.session.is_none() {
                anyhow::bail!("{}", self.status);
            }
        } else {
            self.countdown_deadline = Some(Instant::now() + Duration::from_secs(countdown_seconds));
        }
        self.request_auto_minimize(true);
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_last_file(&self) -> Option<PathBuf> {
        self.last_file.clone()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.refresh_displays();
        self.region = Some(Region {
            x: 180,
            y: 120,
            width: 1280,
            height: 720,
        });
        self.output = "C:\\Users\\demo\\Videos\\Zi-Recording-20260928-1928.mp4".into();
        self.audio = AudioMode::SystemAndMicrophone;
        self.auto_stop_minutes = 5;
        self.quality = RecordingQuality::Detailed;
        self.preview_levels = Some((62, 38));
        self.status = "界面预览：电平为示例值，真实录制时自动更新".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_interrupted(&mut self) {
        self.preview_fixture();
        self.preview_levels = None;
        self.status = "录制提前结束，已保存 MP4 片段：示例显示器在录制中断开".into();
        self.error = true;
        self.last_file = Some(PathBuf::from(
            "C:\\Users\\demo\\Videos\\Zi-Recording-example.mp4",
        ));
        self.output = "C:\\Users\\demo\\Videos\\Zi-Recording-next.mp4".into();
    }
    pub fn poll(&mut self) -> bool {
        let mut repaint = self.selecting || self.countdown_deadline.is_some();
        if let Some(deadline) = self.countdown_deadline
            && Instant::now() >= deadline
        {
            self.countdown_deadline = None;
            self.begin_recording();
        }
        if let Some(session) = &self.session {
            repaint = true;
            for event in session.events.try_iter() {
                match event {
                    Event::Started => {
                        self.started = Some(Instant::now());
                        self.paused_time = Duration::ZERO;
                        self.pause_started = None;
                        self.status = "正在录制…".into();
                        self.error = false;
                    }
                    Event::Finished(result) => {
                        self.started = None;
                        self.pause_started = None;
                        self.session = None;
                        self.finish_auto_minimize();
                        match result {
                            Ok(path) => {
                                self.status = format!(
                                    "录制完成，MP4 已保存{}",
                                    std::fs::metadata(&path)
                                        .map(|m| format!(
                                            " · 实际 {:.2} MB",
                                            m.len() as f64 / 1_000_000.0
                                        ))
                                        .unwrap_or_default()
                                );
                                if self.output.trim() == path.to_string_lossy()
                                    && let Some(parent) = path.parent()
                                {
                                    self.output =
                                        next_output_path(parent).to_string_lossy().into_owned();
                                }
                                self.last_file = Some(path);
                                self.error = false;
                            }
                            Err(e) => {
                                self.status = e;
                                self.error = true;
                            }
                        }
                        break;
                    }
                    Event::Interrupted { path, reason } => {
                        self.started = None;
                        self.pause_started = None;
                        self.session = None;
                        self.finish_auto_minimize();
                        self.status = format!(
                            "录制提前结束，已保存 MP4 片段{}：{reason}",
                            std::fs::metadata(&path)
                                .map(|m| format!(" · 实际 {:.2} MB", m.len() as f64 / 1_000_000.0))
                                .unwrap_or_default()
                        );
                        self.error = true;
                        if self.output.trim() == path.to_string_lossy()
                            && let Some(parent) = path.parent()
                        {
                            self.output = next_output_path(parent).to_string_lossy().into_owned();
                        }
                        self.last_file = Some(path);
                        break;
                    }
                }
            }
        }
        if self.auto_stop_minutes > 0
            && matches!(
                self.tray_status(),
                TrayRecordingStatus::Recording | TrayRecordingStatus::Paused
            )
            && self.elapsed_duration().is_some_and(|elapsed| {
                elapsed >= Duration::from_secs(u64::from(self.auto_stop_minutes) * 60)
            })
        {
            self.request_stop();
            self.status = format!("已录满 {} 分钟，正在完成 MP4 文件…", self.auto_stop_minutes);
        }
        repaint
    }

    fn begin_recording(&mut self) {
        let Some(region) = self.region else {
            self.status = "录制区域已失效，请重新选择".into();
            self.error = true;
            self.finish_auto_minimize();
            return;
        };
        let Some(display) = self.display.clone() else {
            self.status = "请先选择显示器".into();
            self.error = true;
            self.finish_auto_minimize();
            return;
        };
        self.last_file = None;
        match recorder::start_on_display(
            region,
            PathBuf::from(self.output.trim()),
            self.audio,
            self.gains,
            self.quality,
            display,
        ) {
            Ok(session) => {
                self.session = Some(session);
                self.status = "正在启动捕获…".into();
                self.error = false;
                self.last_file = None;
            }
            Err(e) => {
                self.status = format!("启动失败：{e:#}");
                self.error = true;
                self.finish_auto_minimize();
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, tray_available: bool) {
        if !self.displays_loaded {
            self.refresh_displays();
        }
        ui.heading("屏幕录制");
        ui.label("选择显示器，用鼠标框选区域，开始录制后保存为 MP4。");
        ui.small("全局快捷键：Ctrl+Alt+Shift+R 开始 · +P 暂停/继续 · +S 停止保存。快捷键被其他程序占用时请使用页面或托盘按钮。");
        ui.add_space(16.0);
        let busy = self.session.is_some() || self.countdown_deadline.is_some();
        ui.horizontal(|ui| {
            ui.label("录制屏幕");
            let mut chosen = self
                .display
                .as_ref()
                .map(|display| display.device_name.clone());
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("recorder-display")
                    .selected_text(
                        self.display
                            .as_ref()
                            .map(DisplayInfo::label)
                            .unwrap_or_else(|| "未找到显示器".into()),
                    )
                    .show_ui(ui, |ui| {
                        for display in &self.displays {
                            ui.selectable_value(
                                &mut chosen,
                                Some(display.device_name.clone()),
                                display.label(),
                            );
                        }
                    });
            });
            if !busy
                && chosen
                    != self
                        .display
                        .as_ref()
                        .map(|display| display.device_name.clone())
                && let Some(display) = self
                    .displays
                    .iter()
                    .find(|display| Some(&display.device_name) == chosen.as_ref())
                    .cloned()
            {
                self.choose_display(display);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("刷新屏幕"))
                .clicked()
            {
                self.refresh_displays();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new("① 鼠标框选区域").min_size([160.0, 36.0].into()),
                )
                .clicked()
            {
                match self
                    .display
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("未选择显示器"))
                    .and_then(recorder::validate_display)
                {
                    Ok(_) => {
                        #[cfg(windows)]
                        match capture_display_snapshot(self.display.as_ref().unwrap()) {
                            Ok(image) => {
                                self.selection_texture = Some(ui.ctx().load_texture(
                                    "recorder-selection-snapshot",
                                    image,
                                    egui::TextureOptions::LINEAR,
                                ));
                                self.selecting = true;
                            }
                            Err(error) => {
                                self.status = format!("无法预览当前屏幕：{error:#}");
                                self.error = true;
                            }
                        }
                        self.drag_start = None;
                        if self.selecting {
                            self.status = "当前屏幕画面已显示，按住鼠标拖出区域；Esc 取消".into();
                            self.error = false;
                        }
                    }
                    Err(e) => {
                        self.status = format!("无法选择显示器：{e:#}");
                        self.error = true;
                    }
                }
            }
            if ui
                .add_enabled(!busy, egui::Button::new("整个屏幕"))
                .clicked()
            {
                match self
                    .display
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("未选择显示器"))
                    .and_then(|display| recorder::validate_display(display).map(|_| display))
                {
                    Ok(display) => {
                        self.region = Some(Region {
                            x: 0,
                            y: 0,
                            width: display.width & !1,
                            height: display.height & !1,
                        });
                        self.status = "已选择整个屏幕".into();
                        self.error = false;
                    }
                    Err(e) => {
                        self.status = format!("无法选择显示器：{e:#}");
                        self.error = true;
                    }
                }
            }
            if let Some(r) = self.region {
                ui.label(format!(
                    "{} × {} 像素 · 起点 {}, {}",
                    r.width, r.height, r.x, r.y
                ));
            } else {
                ui.label("尚未选择区域");
            }
        });
        ui.add_space(12.0);
        if self.output.is_empty() {
            let directory = dirs::video_dir()
                .or_else(dirs::document_dir)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            self.output = next_output_path(&directory).to_string_lossy().into_owned();
        }
        ui.label("保存位置");
        ui.horizontal(|ui| {
            ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.output)
                    .desired_width((ui.available_width() - 110.0).max(180.0)),
            );
            if ui
                .add_enabled(!busy, egui::Button::new("选择文件…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("MP4 视频", &["mp4"])
                    .set_file_name("Zi-Recording.mp4")
                    .save_file()
            {
                self.output = path.to_string_lossy().into_owned();
            }
        });
        ui.horizontal(|ui| {
            ui.label("声音");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("recorder-audio")
                    .selected_text(match self.audio {
                        AudioMode::None => "不录声音",
                        AudioMode::System => "系统声音",
                        AudioMode::Microphone => "麦克风",
                        AudioMode::SystemAndMicrophone => "系统声音 + 麦克风",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.audio, AudioMode::None, "不录声音");
                        ui.selectable_value(&mut self.audio, AudioMode::System, "系统声音");
                        ui.selectable_value(&mut self.audio, AudioMode::Microphone, "麦克风");
                        ui.selectable_value(
                            &mut self.audio,
                            AudioMode::SystemAndMicrophone,
                            "系统声音 + 麦克风",
                        );
                    });
            });
        });
        ui.horizontal(|ui| {
            ui.label("录制质量");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("recorder-quality")
                    .selected_text(self.quality.label())
                    .show_ui(ui, |ui| {
                        for quality in RecordingQuality::ALL {
                            ui.selectable_value(&mut self.quality, quality, quality.label());
                        }
                    });
            });
            ui.label("H.264 · 30 fps");
        });
        ui.horizontal(|ui| {
            ui.label("大小预览");
            ui.add(egui::Slider::new(&mut self.size_preview_minutes, 1..=60).suffix(" 分钟"));
            let seconds = u64::from(self.size_preview_minutes) * 60;
            ui.strong(format!(
                "约 {:.0} MB",
                self.quality.estimated_megabytes(seconds, self.audio)
            ));
        });
        ui.small("按目标码率、声音和时长推算；静止画面文件可能远小于估算值，保存后显示实际大小。此值不是文件上限。");
        if self.audio != AudioMode::None {
            ui.add_enabled_ui(!busy, |ui| {
                if matches!(
                    self.audio,
                    AudioMode::System | AudioMode::SystemAndMicrophone
                ) {
                    ui.horizontal(|ui| {
                        ui.label("系统声音");
                        ui.add(egui::Slider::new(&mut self.gains.system, 0..=200).suffix("%"));
                    });
                }
                if matches!(
                    self.audio,
                    AudioMode::Microphone | AudioMode::SystemAndMicrophone
                ) {
                    ui.horizontal(|ui| {
                        ui.label("麦克风");
                        ui.add(egui::Slider::new(&mut self.gains.microphone, 0..=200).suffix("%"));
                    });
                }
            });
            ui.small("100% 为原始音量；双声源各占一半，0% 可静音，过高可能削波。");
        }
        let live_levels = self
            .session
            .as_ref()
            .map(|session| recorder::unpack_levels(session.levels.load(Ordering::Acquire)));
        #[cfg(feature = "ui-preview")]
        let levels = live_levels.or(self.preview_levels);
        #[cfg(not(feature = "ui-preview"))]
        let levels = live_levels;
        if let Some((system, microphone)) = levels.filter(|_| self.audio != AudioMode::None) {
            ui.add_space(6.0);
            ui.small("录制电平（短时峰值）");
            if matches!(
                self.audio,
                AudioMode::System | AudioMode::SystemAndMicrophone
            ) {
                ui.horizontal(|ui| {
                    ui.label("系统声音");
                    ui.add(egui::ProgressBar::new(f32::from(system) / 100.0).desired_width(150.0));
                    if system == 0 {
                        ui.small("静音");
                    }
                });
            }
            if matches!(
                self.audio,
                AudioMode::Microphone | AudioMode::SystemAndMicrophone
            ) {
                ui.horizontal(|ui| {
                    ui.label("麦克风");
                    ui.add(
                        egui::ProgressBar::new(f32::from(microphone) / 100.0).desired_width(150.0),
                    );
                    if microphone == 0 {
                        ui.small("静音");
                    }
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label("开始前倒计时");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("recorder-countdown")
                    .selected_text(format!("{} 秒", self.countdown_seconds))
                    .show_ui(ui, |ui| {
                        for seconds in [0, 3, 5] {
                            ui.selectable_value(
                                &mut self.countdown_seconds,
                                seconds,
                                format!("{seconds} 秒"),
                            );
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            ui.label("录满后自动停止");
            ui.add_enabled_ui(!busy, |ui| {
                egui::ComboBox::from_id_salt("recorder-auto-stop")
                    .selected_text(if self.auto_stop_minutes == 0 {
                        "关闭".to_owned()
                    } else {
                        format!("{} 分钟", self.auto_stop_minutes)
                    })
                    .show_ui(ui, |ui| {
                        for minutes in [0, 1, 5, 15, 30, 60] {
                            ui.selectable_value(
                                &mut self.auto_stop_minutes,
                                minutes,
                                if minutes == 0 {
                                    "关闭".to_owned()
                                } else {
                                    format!("{minutes} 分钟")
                                },
                            );
                        }
                    });
            });
        });
        ui.small("按实际录制时间计时；暂停和倒计时不计入时长。关闭时需手动停止。");
        ui.add_enabled(
            !busy && tray_available,
            egui::Checkbox::new(
                &mut self.auto_minimize,
                "开始录制时最小化窗口，结束后自动恢复",
            ),
        );
        if !tray_available {
            ui.small("托盘不可用，录制时主窗口保持显示");
        }
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && self.region.is_some(),
                    egui::Button::new("② 开始录制").min_size([140.0, 38.0].into()),
                )
                .clicked()
            {
                self.request_start(tray_available);
            }
            if self.countdown_deadline.is_some() && ui.button("取消倒计时").clicked() {
                self.request_stop();
            }
            if let Some(session) = &self.session
                && self.started.is_some()
            {
                let paused = session.pause.is_paused();
                if ui
                    .add_enabled(
                        !session.stop.load(Ordering::Acquire),
                        egui::Button::new(if paused { "▶ 继续" } else { "Ⅱ 暂停" }),
                    )
                    .clicked()
                {
                    self.toggle_pause();
                }
            }
            if ui
                .add_enabled(
                    self.session
                        .as_ref()
                        .is_some_and(|s| !s.stop.load(Ordering::Acquire)),
                    egui::Button::new(
                        if self
                            .session
                            .as_ref()
                            .is_some_and(|s| s.stop.load(Ordering::Acquire))
                        {
                            "正在保存…"
                        } else {
                            "■ 停止并保存"
                        },
                    )
                    .min_size([140.0, 38.0].into()),
                )
                .clicked()
            {
                self.request_stop();
            }
            if let Some(duration) = self.elapsed_duration() {
                ui.strong(format!(
                    "● {:02}:{:02}",
                    duration.as_secs() / 60,
                    duration.as_secs() % 60
                ));
            }
            if let Some(deadline) = self.countdown_deadline {
                ui.strong(format!(
                    "{}…",
                    deadline.saturating_duration_since(Instant::now()).as_secs() + 1
                ));
            }
        });
        ui.add_space(10.0);
        if !self.status.is_empty() {
            ui.label(RichText::new(&self.status).color(if self.error {
                Color32::from_rgb(220, 70, 75)
            } else {
                Color32::from_rgb(80, 160, 110)
            }));
        }
        if let Some(path) = &self.last_file {
            ui.horizontal(|ui| {
                if ui.button("打开视频").clicked() {
                    if let Err(e) = open::that(path) {
                        self.status = format!("打开视频失败：{e}");
                        self.error = true;
                    }
                }
                if ui.button("打开所在文件夹").clicked() {
                    if let Some(parent) = path.parent() {
                        if let Err(e) = open::that(parent) {
                            self.status = format!("打开文件夹失败：{e}");
                            self.error = true;
                        }
                    }
                }
                ui.small(path.display().to_string());
            });
        }
        ui.add_space(12.0);
        ui.small(
            "录制所选屏幕的画面和鼠标光标，可录系统声音、麦克风或两者混音。已有同名文件不会被覆盖。",
        );
    }

    pub fn selection_overlay(&mut self, ctx: &egui::Context) {
        if !self.selecting {
            return;
        }
        let Some(display) = self.display.clone() else {
            self.selecting = false;
            self.selection_texture = None;
            self.status = "没有可用的显示器，请刷新屏幕列表".into();
            self.error = true;
            return;
        };
        let placement_error = place_selection_overlay(&display).err();
        if self.selection_texture.is_none() {
            #[cfg(windows)]
            if let Ok(image) = capture_display_snapshot(&display) {
                self.selection_texture = Some(ctx.load_texture(
                    "recorder-selection-snapshot",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        let id = egui::ViewportId::from_hash_of("zi-recorder-select");
        let builder = egui::ViewportBuilder::default()
            .with_title("选择录制区域 · Esc 取消")
            .with_position([display.x as f32, display.y as f32])
            .with_inner_size([display.width as f32, display.height as f32])
            .with_clamp_size_to_monitor_size(false)
            .with_decorations(false)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_transparent(true);
        ctx.show_viewport_immediate(id, builder, |panel, _| {
            if let Some(error) = &placement_error {
                self.selecting = false;
                self.selection_texture = None;
                self.status = format!("无法在所选屏幕框选区域：{error:#}");
                self.error = true;
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            if panel.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.selecting = false;
                self.selection_texture = None;
                self.drag_start = None;
                self.status = "已取消区域选择".into();
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::TRANSPARENT))
                .show(panel, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                    if let Some(texture) = &self.selection_texture {
                        ui.painter().image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(9, 17, 29, 58),
                    );
                    let pointer = response.interact_pointer_pos();
                    if response.drag_started() {
                        self.drag_start = pointer;
                    }
                    if let (Some(a), Some(b)) = (self.drag_start, pointer) {
                        let selection = egui::Rect::from_two_pos(a, b).intersect(rect);
                        if let Some(texture) = &self.selection_texture {
                            ui.painter().with_clip_rect(selection).image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        ui.painter().rect_filled(
                            selection,
                            2.0,
                            Color32::from_rgba_unmultiplied(80, 160, 245, 18),
                        );
                        ui.painter().rect_stroke(
                            selection,
                            2.0,
                            Stroke::new(2.0, Color32::from_rgb(120, 195, 255)),
                            StrokeKind::Inside,
                        );
                        ui.painter().text(
                            selection.right_bottom() + egui::vec2(-8.0, -8.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("{:.0} × {:.0}", selection.width(), selection.height()),
                            egui::FontId::proportional(16.0),
                            Color32::WHITE,
                        );
                    }
                    ui.painter().text(
                        rect.center_top() + egui::vec2(0.0, 48.0),
                        egui::Align2::CENTER_TOP,
                        "按住鼠标拖出录制区域  ·  Esc 取消",
                        egui::FontId::proportional(24.0),
                        Color32::WHITE,
                    );
                    if response.drag_stopped() {
                        if let (Some(a), Some(b)) = (self.drag_start.take(), pointer) {
                            let view = rect.size();
                            match recorder::validate_display(&display).ok().and_then(|_| {
                                Region::from_points(
                                    (a.x - rect.left(), a.y - rect.top()),
                                    (b.x - rect.left(), b.y - rect.top()),
                                    (display.width, display.height),
                                    (view.x, view.y),
                                )
                            }) {
                                Some(region) => {
                                    self.region = Some(region);
                                    self.status = "区域已选好，点击“开始录制”".into();
                                    self.error = false;
                                    self.selecting = false;
                                    self.selection_texture = None;
                                    panel.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                                None => {
                                    self.status = "区域太小，请至少选择 32 × 32 像素".into();
                                    self.error = true;
                                }
                            }
                        }
                    }
                });
        });
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

impl Drop for RecorderState {
    fn drop(&mut self) {
        if let Some(s) = &mut self.session {
            s.stop_and_join();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn consecutive_recordings_get_new_names_without_overwriting() {
        let folder =
            std::env::temp_dir().join(format!("zi-recorder-names-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let first = super::available_named_path(&folder, "Zi-Recording-fixed");
        std::fs::write(&first, b"first video").unwrap();
        let second = super::available_named_path(&folder, "Zi-Recording-fixed");
        std::fs::write(&second, b"second video").unwrap();
        let third = super::available_named_path(&folder, "Zi-Recording-fixed");
        assert_eq!(second.file_name().unwrap(), "Zi-Recording-fixed-2.mp4");
        assert_eq!(third.file_name().unwrap(), "Zi-Recording-fixed-3.mp4");
        assert_eq!(std::fs::read(&first).unwrap(), b"first video");
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn tray_stop_cancels_countdown_before_capture_starts() {
        let mut state = super::RecorderState::default();
        state.countdown_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        state.request_auto_minimize(true);
        assert!(state.take_minimize_request());
        assert_eq!(state.tray_status(), super::TrayRecordingStatus::Countdown);
        state.request_stop();
        assert_eq!(state.tray_status(), super::TrayRecordingStatus::Idle);
        assert!(state.session.is_none());
        assert!(state.take_restore_request());
        assert!(!state.take_restore_request());
    }
    #[test]
    fn auto_minimize_requires_tray_and_recovers_from_start_failure() {
        let mut state = super::RecorderState::default();
        state.request_auto_minimize(false);
        assert!(!state.take_minimize_request());
        state.region = Some(crate::recorder::Region {
            x: 0,
            y: 0,
            width: 320,
            height: 180,
        });
        state.request_auto_minimize(true);
        assert!(state.take_minimize_request());
        state.begin_recording();
        assert!(state.error);
        assert!(state.take_restore_request());
    }
    #[test]
    fn switching_or_resizing_display_discards_a_stale_region() {
        let original = crate::recorder::primary_display().unwrap();
        let mut state = super::RecorderState::default();
        state.display = Some(original.clone());
        state.region = Some(crate::recorder::Region {
            x: 0,
            y: 0,
            width: 320,
            height: 180,
        });
        let mut second = original.clone();
        second.device_name = "\\\\.\\DISPLAY99".into();
        state.choose_display(second);
        assert!(state.region.is_none());
        state.display = Some(original.clone());
        state.region = Some(crate::recorder::Region {
            x: 0,
            y: 0,
            width: 320,
            height: 180,
        });
        state.display.as_mut().unwrap().width += 2;
        state.refresh_displays();
        assert!(state.region.is_none());
        assert_eq!(
            state.display.as_ref().unwrap().device_name,
            original.device_name
        );
    }

    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and working H.264 encoder"]
    fn pause_resume_and_stop_commands_finalize_a_real_video() {
        use super::{RecorderState, TrayRecordingStatus};
        use crate::recorder::{self, AudioMode, Region};
        use std::{
            sync::atomic::Ordering,
            time::{Duration, Instant},
        };
        let (width, height) = recorder::primary_size().unwrap();
        let region = Region {
            x: 0,
            y: 0,
            width: width.min(640) & !1,
            height: height.min(360) & !1,
        };
        let path =
            std::env::temp_dir().join(format!("zi-recorder-controls-{}.mp4", uuid::Uuid::new_v4()));
        let mut state = RecorderState::default();
        state.display = Some(recorder::primary_display().unwrap());
        state.region = Some(region);
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::None;
        state.countdown_seconds = 0;
        state.begin_recording();
        state.request_auto_minimize(true);
        assert!(state.take_minimize_request());
        assert_eq!(state.tray_status(), TrayRecordingStatus::Starting);
        let deadline = Instant::now() + Duration::from_secs(15);
        while state.tray_status() == TrayRecordingStatus::Starting && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.tray_status(),
            TrayRecordingStatus::Recording,
            "{}",
            state.status
        );
        std::thread::sleep(Duration::from_millis(700));
        state.toggle_pause();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Paused);
        std::thread::sleep(Duration::from_millis(900));
        state.toggle_pause();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Recording);
        std::thread::sleep(Duration::from_millis(700));
        state.request_stop();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Saving);
        assert!(state.session.as_ref().unwrap().stop.load(Ordering::Acquire));
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.tray_status() != TrayRecordingStatus::Idle && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.tray_status(),
            TrayRecordingStatus::Idle,
            "{}",
            state.status
        );
        assert_eq!(
            state.last_file.as_deref(),
            Some(path.as_path()),
            "{}",
            state.status
        );
        assert!(state.take_restore_request());
        assert_ne!(state.output, path.to_string_lossy());
        let data = std::fs::read(&path).unwrap();
        assert!(data.windows(4).any(|v| v == b"moov"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and working H.264 encoder"]
    fn auto_stop_finalizes_a_real_video_without_counting_pause() {
        use super::{RecorderState, TrayRecordingStatus};
        use crate::recorder::{self, AudioMode, Region};
        use std::time::{Duration, Instant};

        let display = recorder::primary_display().unwrap();
        let path =
            std::env::temp_dir().join(format!("zi-recorder-autostop-{}.mp4", uuid::Uuid::new_v4()));
        let mut state = RecorderState::default();
        state.display = Some(display.clone());
        state.region = Some(Region {
            x: 0,
            y: 0,
            width: display.width.min(640) & !1,
            height: display.height.min(360) & !1,
        });
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::None;
        state.set_auto_stop_minutes(1);
        state.begin_recording();
        let deadline = Instant::now() + Duration::from_secs(15);
        while state.tray_status() == TrayRecordingStatus::Starting && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(state.tray_status(), TrayRecordingStatus::Recording);
        std::thread::sleep(Duration::from_millis(500));
        state.toggle_pause();
        state.started = Some(Instant::now() - Duration::from_secs(60));
        state.pause_started = Some(Instant::now() - Duration::from_secs(2));
        state.poll();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Paused);
        state.toggle_pause();
        state.started = Some(Instant::now() - Duration::from_secs(63));
        state.poll();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Saving);
        assert!(state.status.contains("自动") || state.status.contains("录满"));
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.tray_status() != TrayRecordingStatus::Idle && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.last_file.as_deref(),
            Some(path.as_path()),
            "{}",
            state.status
        );
        let data = std::fs::read(&path).unwrap();
        assert!(data.windows(4).any(|part| part == b"moov"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop; records for one minute plus pause"]
    fn auto_stop_waits_for_one_real_minute_excluding_pause() {
        use super::{RecorderState, TrayRecordingStatus};
        use crate::recorder::{self, AudioMode, Region};
        use std::time::{Duration, Instant};

        let display = recorder::primary_display().unwrap();
        let path = std::env::temp_dir().join(format!(
            "zi-recorder-autostop-minute-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        let mut state = RecorderState::default();
        state.display = Some(display.clone());
        state.region = Some(Region {
            x: 0,
            y: 0,
            width: display.width.min(640) & !1,
            height: display.height.min(360) & !1,
        });
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::None;
        state.set_auto_stop_minutes(1);
        state.begin_recording();
        let startup_deadline = Instant::now() + Duration::from_secs(15);
        while state.tray_status() == TrayRecordingStatus::Starting
            && Instant::now() < startup_deadline
        {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(state.tray_status(), TrayRecordingStatus::Recording);
        let started = Instant::now();
        std::thread::sleep(Duration::from_secs(2));
        state.toggle_pause();
        std::thread::sleep(Duration::from_secs(3));
        state.poll();
        assert_eq!(state.tray_status(), TrayRecordingStatus::Paused);
        state.toggle_pause();
        let deadline = started + Duration::from_secs(75);
        while state.tray_status() != TrayRecordingStatus::Idle && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(50));
        }
        let wall = started.elapsed();
        assert_eq!(
            state.last_file.as_deref(),
            Some(path.as_path()),
            "{}",
            state.status
        );
        assert!(
            (Duration::from_secs(62)..Duration::from_secs(75)).contains(&wall),
            "auto-stop wall time {wall:?}"
        );
        let data = std::fs::read(&path).unwrap();
        assert!(data.windows(4).any(|part| part == b"moov"));
        eprintln!("auto-stop after {wall:?}, MP4 bytes={}", data.len());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and working H.264 encoder"]
    fn unexpected_capture_end_preserves_video_and_explains_it() {
        use super::{RecorderState, TrayRecordingStatus};
        use crate::recorder::{self, AudioMode, Region};
        use std::{
            sync::atomic::Ordering,
            time::{Duration, Instant},
        };

        let display = recorder::primary_display().unwrap();
        let path = std::env::temp_dir().join(format!(
            "zi-recorder-interrupted-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        let mut state = RecorderState::default();
        state.display = Some(display.clone());
        state.region = Some(Region {
            x: 0,
            y: 0,
            width: display.width.min(640) & !1,
            height: display.height.min(360) & !1,
        });
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::None;
        state.begin_recording();
        state.request_auto_minimize(true);
        let deadline = Instant::now() + Duration::from_secs(15);
        while state.tray_status() == TrayRecordingStatus::Starting && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(state.tray_status(), TrayRecordingStatus::Recording);
        std::thread::sleep(Duration::from_secs(1));
        state
            .session
            .as_ref()
            .unwrap()
            .interrupt_for_test
            .store(1, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.tray_status() != TrayRecordingStatus::Idle && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.last_file.as_deref(),
            Some(path.as_path()),
            "{}",
            state.status
        );
        assert!(state.error);
        assert!(state.status.contains("提前结束"));
        assert!(state.status.contains("测试模拟"));
        assert!(state.take_restore_request());
        assert_ne!(state.output, path.to_string_lossy());
        let video = std::fs::read(&path).unwrap();
        assert!(video.len() > 1024 && video.windows(4).any(|part| part == b"moov"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and default microphone"]
    fn microphone_interruption_preserves_recorded_segment() {
        use super::{RecorderState, TrayRecordingStatus};
        use crate::recorder::{self, AudioMode, Region};
        use std::{
            sync::atomic::Ordering,
            time::{Duration, Instant},
        };

        let display = recorder::primary_display().unwrap();
        let path = std::env::temp_dir().join(format!(
            "zi-recorder-audio-interrupted-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        let mut state = RecorderState::default();
        state.display = Some(display.clone());
        state.region = Some(Region {
            x: 0,
            y: 0,
            width: display.width.min(640) & !1,
            height: display.height.min(360) & !1,
        });
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::Microphone;
        state.begin_recording();
        let deadline = Instant::now() + Duration::from_secs(15);
        while state.tray_status() == TrayRecordingStatus::Starting && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.tray_status(),
            TrayRecordingStatus::Recording,
            "{}",
            state.status
        );
        std::thread::sleep(Duration::from_secs(1));
        state
            .session
            .as_ref()
            .unwrap()
            .interrupt_for_test
            .store(2, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.tray_status() != TrayRecordingStatus::Idle && Instant::now() < deadline {
            state.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.last_file.as_deref(),
            Some(path.as_path()),
            "{}",
            state.status
        );
        assert!(state.error && state.status.contains("音频"));
        let video = std::fs::read(&path).unwrap();
        assert!(video.windows(4).any(|part| part == b"moov"));
        assert!(video.windows(4).any(|part| part == b"soun"));
        std::fs::remove_file(path).unwrap();
    }
}
