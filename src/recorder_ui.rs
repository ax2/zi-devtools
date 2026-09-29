use crate::recorder::{self, AudioGains, AudioMode, Event, Region, Session};
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

pub struct RecorderState {
    selecting: bool,
    drag_start: Option<egui::Pos2>,
    region: Option<Region>,
    output: String,
    audio: AudioMode,
    gains: AudioGains,
    countdown_seconds: u64,
    countdown_deadline: Option<Instant>,
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
            selecting: false,
            drag_start: None,
            region: None,
            output: String::new(),
            audio: AudioMode::None,
            gains: AudioGains::default(),
            countdown_seconds: 3,
            countdown_deadline: None,
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
            return;
        }
        if let Some(session) = &self.session
            && !session.stop.swap(true, Ordering::AcqRel)
        {
            self.status = "正在完成 MP4 文件…".into();
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_begin_selection(&mut self) {
        self.selecting = true;
        self.drag_start = None;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_region(&self) -> Option<Region> {
        self.region
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.region = Some(Region {
            x: 180,
            y: 120,
            width: 1280,
            height: 720,
        });
        self.output = "C:\\Users\\demo\\Videos\\Zi-Recording-20260928-1928.mp4".into();
        self.audio = AudioMode::SystemAndMicrophone;
        self.preview_levels = Some((62, 38));
        self.status = "界面预览：电平为示例值，真实录制时自动更新".into();
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
                        match result {
                            Ok(path) => {
                                self.status = "录制完成，MP4 已保存".into();
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
                }
            }
        }
        repaint
    }

    fn begin_recording(&mut self) {
        let Some(region) = self.region else {
            return;
        };
        self.last_file = None;
        match recorder::start_with_gains(
            region,
            PathBuf::from(self.output.trim()),
            self.audio,
            self.gains,
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
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("屏幕录制");
        ui.label("鼠标框选主显示器上的区域，开始录制后保存为 MP4。");
        ui.add_space(16.0);
        let busy = self.session.is_some() || self.countdown_deadline.is_some();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new("① 鼠标框选区域").min_size([160.0, 36.0].into()),
                )
                .clicked()
            {
                match recorder::primary_size() {
                    Ok(_) => {
                        self.selecting = true;
                        self.drag_start = None;
                        self.status = "按住鼠标左键拖出录制区域，Esc 取消".into();
                        self.error = false;
                    }
                    Err(e) => {
                        self.status = format!("无法读取主显示器：{e:#}");
                        self.error = true;
                    }
                }
            }
            if ui
                .add_enabled(!busy, egui::Button::new("整个主屏幕"))
                .clicked()
            {
                match recorder::primary_size() {
                    Ok((width, height)) => {
                        self.region = Some(Region {
                            x: 0,
                            y: 0,
                            width: width & !1,
                            height: height & !1,
                        });
                        self.status = "已选择整个主屏幕".into();
                        self.error = false;
                    }
                    Err(e) => {
                        self.status = format!("无法读取主显示器：{e:#}");
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
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !busy && self.region.is_some(),
                    egui::Button::new("② 开始录制").min_size([140.0, 38.0].into()),
                )
                .clicked()
            {
                if let Some(region) = self.region {
                    match recorder::validate_request(region, Path::new(self.output.trim())) {
                        Ok(()) if self.countdown_seconds == 0 => self.begin_recording(),
                        Ok(()) => {
                            self.countdown_deadline =
                                Some(Instant::now() + Duration::from_secs(self.countdown_seconds));
                            self.status = "倒计时中，请准备录制区域".into();
                            self.error = false;
                        }
                        Err(e) => {
                            self.status = format!("无法开始录制：{e:#}");
                            self.error = true;
                        }
                    }
                }
            }
            if self.countdown_deadline.is_some() && ui.button("取消倒计时").clicked() {
                self.countdown_deadline = None;
                self.status = "已取消录制".into();
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
            if let Some(started) = self.started {
                let duration = started
                    .elapsed()
                    .saturating_sub(self.paused_time)
                    .saturating_sub(
                        self.pause_started
                            .map(|at| at.elapsed())
                            .unwrap_or_default(),
                    );
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
            "录制主显示器画面和鼠标光标，可录系统声音、麦克风或两者混音。已有同名文件不会被覆盖。",
        );
    }

    pub fn selection_overlay(&mut self, ctx: &egui::Context) {
        if !self.selecting {
            return;
        }
        let id = egui::ViewportId::from_hash_of("zi-recorder-select");
        let builder = egui::ViewportBuilder::default()
            .with_title("选择录制区域 · Esc 取消")
            .with_position([0.0, 0.0])
            .with_fullscreen(true)
            .with_decorations(false)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_transparent(true);
        ctx.show_viewport_immediate(id, builder, |panel, _| {
            if panel.input(|i| i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)) {
                self.selecting = false;
                self.drag_start = None;
                self.status = "已取消区域选择".into();
                panel.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::from_rgba_unmultiplied(9, 17, 29, 110)))
                .show(panel, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                    let pointer = response.interact_pointer_pos();
                    if response.drag_started() {
                        self.drag_start = pointer;
                    }
                    if let (Some(a), Some(b)) = (self.drag_start, pointer) {
                        let selection = egui::Rect::from_two_pos(a, b).intersect(rect);
                        ui.painter().rect_filled(
                            selection,
                            2.0,
                            Color32::from_rgba_unmultiplied(80, 160, 245, 45),
                        );
                        ui.painter().rect_stroke(
                            selection,
                            2.0,
                            Stroke::new(2.0, Color32::from_rgb(120, 195, 255)),
                            StrokeKind::Inside,
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
                            match recorder::primary_size().ok().and_then(|(w, h)| {
                                Region::from_points(
                                    (a.x - rect.left(), a.y - rect.top()),
                                    (b.x - rect.left(), b.y - rect.top()),
                                    (w, h),
                                    (view.x, view.y),
                                )
                            }) {
                                Some(region) => {
                                    self.region = Some(region);
                                    self.status = "区域已选好，点击“开始录制”".into();
                                    self.error = false;
                                    self.selecting = false;
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
        assert_eq!(state.tray_status(), super::TrayRecordingStatus::Countdown);
        state.request_stop();
        assert_eq!(state.tray_status(), super::TrayRecordingStatus::Idle);
        assert!(state.session.is_none());
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
        state.region = Some(region);
        state.output = path.to_string_lossy().into_owned();
        state.audio = AudioMode::None;
        state.countdown_seconds = 0;
        state.begin_recording();
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
        assert_ne!(state.output, path.to_string_lossy());
        let data = std::fs::read(&path).unwrap();
        assert!(data.windows(4).any(|v| v == b"moov"));
        std::fs::remove_file(path).unwrap();
    }
}
