//! Local monitor capture. Encoding stays on a worker thread.
pub mod tutorial;
use anyhow::{Context as _, Result, anyhow, bail};
use crossbeam_channel::{Receiver, unbounded};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode,
    WaveFormat,
};
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    },
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub device_name: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}
impl DisplayInfo {
    fn from_monitor(monitor: Monitor) -> Result<Self> {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(monitor.as_raw_hmonitor(), &mut info) } == 0 {
            bail!(
                "读取显示器物理边界失败：{}",
                std::io::Error::last_os_error()
            );
        }
        let bounds = info.rcMonitor;
        let width = u32::try_from(bounds.right - bounds.left).context("显示器宽度无效")?;
        let height = u32::try_from(bounds.bottom - bounds.top).context("显示器高度无效")?;
        let device_name = monitor.device_name()?;
        let short_device = device_name.rsplit('\\').next().unwrap_or(&device_name);
        let name = monitor
            .name()
            .ok()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("显示器 {}", short_device.trim_start_matches("DISPLAY")));
        Ok(Self {
            device_name,
            name,
            x: bounds.left,
            y: bounds.top,
            width,
            height,
            primary: info.dwFlags & 1 != 0,
        })
    }
    fn same_topology(&self, other: &Self) -> bool {
        self.device_name == other.device_name
            && self.primary == other.primary
            && (self.x, self.y, self.width, self.height)
                == (other.x, other.y, other.width, other.height)
    }
    pub fn label(&self) -> String {
        let short_device = self
            .device_name
            .rsplit('\\')
            .next()
            .unwrap_or(&self.device_name);
        format!(
            "{} ({short_device}) · {} × {}{}",
            self.name,
            self.width,
            self.height,
            if self.primary { " · 主屏幕" } else { "" }
        )
    }
}
struct CaptureTarget {
    monitor: Monitor,
    region: Region,
    display: DisplayInfo,
}
#[derive(Clone, Copy)]
struct CaptureOptions {
    audio: AudioMode,
    gains: AudioGains,
    quality: RecordingQuality,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecordingQuality {
    Compact,
    #[default]
    Balanced,
    Detailed,
}
impl RecordingQuality {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Balanced, Self::Detailed];
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "节省空间 · 4 Mbps",
            Self::Balanced => "均衡 · 8 Mbps",
            Self::Detailed => "高画质 · 16 Mbps",
        }
    }
    pub fn bitrate(self) -> u32 {
        match self {
            Self::Compact => 4_000_000,
            Self::Balanced => 8_000_000,
            Self::Detailed => 16_000_000,
        }
    }
    pub fn estimated_megabytes(self, seconds: u64, audio: AudioMode) -> f64 {
        let audio_bitrate = if audio == AudioMode::None { 0 } else { 192_000 };
        (f64::from(self.bitrate() + audio_bitrate) * seconds as f64 / 8.0 / 1_000_000.0) * 1.05
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMode {
    #[default]
    None,
    System,
    Microphone,
    SystemAndMicrophone,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioGains {
    pub system: u16,
    pub microphone: u16,
}
impl Default for AudioGains {
    fn default() -> Self {
        Self {
            system: 100,
            microphone: 100,
        }
    }
}
impl Region {
    pub fn from_points(
        a: (f32, f32),
        b: (f32, f32),
        display: (u32, u32),
        view: (f32, f32),
    ) -> Option<Self> {
        let (ax, ay) = a;
        let (bx, by) = b;
        let (display_width, display_height) = display;
        let (view_width, view_height) = view;
        if view_width <= 0.0 || view_height <= 0.0 {
            return None;
        }
        let map_x = |x: f32| {
            ((x / view_width * display_width as f32).round() as i64).clamp(0, display_width as i64)
                as u32
        };
        let map_y = |y: f32| {
            ((y / view_height * display_height as f32).round() as i64)
                .clamp(0, display_height as i64) as u32
        };
        let (mut x1, mut x2) = (map_x(ax), map_x(bx));
        let (mut y1, mut y2) = (map_y(ay), map_y(by));
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
        }
        if y1 > y2 {
            std::mem::swap(&mut y1, &mut y2);
        }
        // H.264 hardware encoders generally require even output dimensions.
        x2 -= (x2 - x1) % 2;
        y2 -= (y2 - y1) % 2;
        (x2 - x1 >= 32 && y2 - y1 >= 32).then_some(Self {
            x: x1,
            y: y1,
            width: x2 - x1,
            height: y2 - y1,
        })
    }
    pub fn fits(self, width: u32, height: u32) -> bool {
        self.width >= 32
            && self.height >= 32
            && self.width % 2 == 0
            && self.height % 2 == 0
            && self.x.checked_add(self.width).is_some_and(|v| v <= width)
            && self.y.checked_add(self.height).is_some_and(|v| v <= height)
    }
}

#[derive(Debug)]
pub enum Event {
    Started,
    Finished(Result<PathBuf, String>),
    Interrupted { path: PathBuf, reason: String },
}
pub struct Session {
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<PauseClock>,
    pub levels: Arc<AtomicU32>,
    pub events: Receiver<Event>,
    pub tutorial: Arc<tutorial::Control>,
    #[cfg(test)]
    pub(crate) interrupt_for_test: Arc<AtomicU32>,
    worker: Option<thread::JoinHandle<()>>,
}
struct RecordingSignals {
    pause: Arc<PauseClock>,
    levels: Arc<AtomicU32>,
    tutorial: Arc<tutorial::Control>,
    #[cfg(test)]
    interrupt_for_test: Arc<AtomicU32>,
}
impl Session {
    pub fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct PauseClock {
    state: parking_lot::Mutex<PauseState>,
}
#[derive(Default)]
struct PauseState {
    since: Option<Instant>,
    total: Duration,
}
impl PauseClock {
    pub fn is_paused(&self) -> bool {
        self.state.lock().since.is_some()
    }
    pub fn set_paused(&self, paused: bool) {
        let mut state = self.state.lock();
        match (paused, state.since.take()) {
            (true, Some(since)) => state.since = Some(since),
            (true, None) => state.since = Some(Instant::now()),
            (false, Some(since)) => state.total += since.elapsed(),
            (false, None) => {}
        }
    }
    fn elapsed_since(&self, started: Instant) -> Option<Duration> {
        let state = self.state.lock();
        state
            .since
            .is_none()
            .then(|| started.elapsed().saturating_sub(state.total))
    }
    fn duration_at_stop(&self, started: Instant) -> Duration {
        let state = self.state.lock();
        state
            .since
            .unwrap_or_else(Instant::now)
            .duration_since(started)
            .saturating_sub(state.total)
    }
}

struct Capture {
    encoder: Option<VideoEncoder>,
    region: Region,
    started: Instant,
    scratch: Vec<u8>,
    flipped: Vec<u8>,
    pause: Arc<PauseClock>,
    last_frame_elapsed: Option<Duration>,
    source: Vec<u8>,
    composed: Vec<u8>,
    effects: tutorial::Compositor,
    tutorial: Arc<tutorial::Control>,
    origin: [i32; 2],
}

pub struct CaptureFlags {
    region: Region,
    path: PathBuf,
    audio: AudioMode,
    quality: RecordingQuality,
    pause: Arc<PauseClock>,
    tutorial: Arc<tutorial::Control>,
    origin: [i32; 2],
}
type CaptureError = Box<dyn std::error::Error + Send + Sync>;
impl GraphicsCaptureApiHandler for Capture {
    type Flags = CaptureFlags;
    type Error = CaptureError;
    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let CaptureFlags {
            region,
            path,
            audio,
            quality,
            pause,
            tutorial,
            origin,
        } = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(region.width, region.height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(30)
                .bitrate(quality.bitrate()),
            AudioSettingsBuilder::default().disabled(audio == AudioMode::None),
            ContainerSettingsBuilder::default(),
            path,
        )?;
        Ok(Self {
            encoder: Some(encoder),
            region,
            started: Instant::now(),
            scratch: Vec::new(),
            flipped: Vec::new(),
            pause,
            last_frame_elapsed: None,
            source: Vec::new(),
            composed: Vec::new(),
            effects: tutorial::Compositor::default(),
            tutorial,
            origin,
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let Some(elapsed) = self.pause.elapsed_since(self.started) else {
            self.effects.suspend();
            return Ok(());
        };
        if self
            .last_frame_elapsed
            .is_some_and(|last| elapsed.saturating_sub(last) < Duration::from_millis(33))
        {
            return Ok(());
        }
        let r = self.region;
        if !r.fits(frame.width(), frame.height()) {
            return Err(anyhow!("显示器尺寸在录制中变化；已停止录制").into());
        }
        let buffer = frame.buffer_crop(r.x, r.y, r.x + r.width, r.y + r.height)?;
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch);
        self.source.clear();
        self.source.extend_from_slice(pixels);
        #[cfg(feature = "ui-preview")]
        self.tutorial.copy_preview_source(&mut self.source);
        self.compose_frame(elapsed)?;
        let timestamp = elapsed.as_nanos().saturating_div(100).min(i64::MAX as u128) as i64;
        self.encoder
            .as_mut()
            .ok_or_else(|| anyhow!("视频编码器已停止"))?
            .send_frame_buffer(&self.flipped, timestamp)?;
        self.last_frame_elapsed = Some(elapsed);
        Ok(())
    }
}
impl Capture {
    fn compose_frame(&mut self, elapsed: Duration) -> Result<()> {
        let r = self.region;
        let settings = self.tutorial.snapshot();
        let pixels = if settings.active() {
            self.effects.render(
                &self.source,
                [r.width, r.height],
                settings,
                self.tutorial.pointer(self.origin, r),
                elapsed,
                &mut self.composed,
            )?;
            &self.composed
        } else {
            self.effects.idle(elapsed);
            &self.source
        };
        let stride = r.width as usize * 4;
        self.flipped.resize(pixels.len(), 0);
        for row in 0..r.height as usize {
            self.flipped[row * stride..(row + 1) * stride].copy_from_slice(
                &pixels[(r.height as usize - 1 - row) * stride..(r.height as usize - row) * stride],
            );
        }
        Ok(())
    }
    fn keep_video_alive(&mut self, elapsed: Duration) -> Result<()> {
        let interval = if self.tutorial.snapshot().active() {
            33
        } else {
            250
        };
        if self.flipped.is_empty()
            || self
                .last_frame_elapsed
                .is_some_and(|last| elapsed.saturating_sub(last) < Duration::from_millis(interval))
        {
            return Ok(());
        }
        // Recompose from the retained raw frame: static desktops still show cursor effects
        // and changes to live controls; do not repeatedly zoom the previous output.
        self.compose_frame(elapsed)?;
        let timestamp = elapsed.as_nanos().saturating_div(100).min(i64::MAX as u128) as i64;
        self.encoder
            .as_mut()
            .context("编码器已停止")?
            .send_frame_buffer(&self.flipped, timestamp)?;
        self.last_frame_elapsed = Some(elapsed);
        Ok(())
    }
    fn extend_to_stop(&mut self, elapsed: Duration) -> Result<()> {
        if self.flipped.is_empty() {
            bail!("没有捕获到画面帧，未创建空视频");
        }
        let encoder = self.encoder.as_mut().context("编码器已停止")?;
        let last = self.last_frame_elapsed.unwrap_or_default();
        let near_end = elapsed.saturating_sub(Duration::from_millis(33));
        if near_end > last {
            let timestamp = near_end
                .as_nanos()
                .saturating_div(100)
                .min(i64::MAX as u128) as i64;
            encoder.send_frame_buffer(&self.flipped, timestamp)?;
        }
        let timestamp = elapsed.as_nanos().saturating_div(100).min(i64::MAX as u128) as i64;
        encoder.send_frame_buffer(&self.flipped, timestamp)?;
        Ok(())
    }
}

pub fn enumerate_displays() -> Result<Vec<DisplayInfo>> {
    let mut displays = Monitor::enumerate()?
        .into_iter()
        .map(DisplayInfo::from_monitor)
        .collect::<Result<Vec<_>>>()?;
    displays.sort_by_key(|display| (!display.primary, display.x, display.y));
    if displays.is_empty() {
        bail!("没有找到可录制的显示器");
    }
    Ok(displays)
}
pub fn primary_display() -> Result<DisplayInfo> {
    enumerate_displays()?
        .into_iter()
        .find(|display| display.primary)
        .context("没有找到主显示器")
}
pub fn primary_size() -> Result<(u32, u32)> {
    let display = primary_display()?;
    Ok((display.width, display.height))
}
fn resolve_display(display: &DisplayInfo) -> Result<Monitor> {
    for monitor in Monitor::enumerate()? {
        if monitor.device_name()? == display.device_name {
            let current = DisplayInfo::from_monitor(monitor)?;
            if !display.same_topology(&current) {
                bail!("显示器布局或分辨率已变化，请重新选择录制区域");
            }
            return if display.primary {
                Monitor::primary().map_err(Into::into)
            } else {
                Ok(monitor)
            };
        }
    }
    bail!("所选显示器已断开，请重新选择")
}
pub fn validate_display(display: &DisplayInfo) -> Result<()> {
    resolve_display(display).map(|_| ())
}
pub fn start(region: Region, output: PathBuf, audio: AudioMode) -> Result<Session> {
    start_with_gains(region, output, audio, AudioGains::default())
}
pub fn start_with_gains(
    region: Region,
    output: PathBuf,
    audio: AudioMode,
    gains: AudioGains,
) -> Result<Session> {
    start_on_display(
        region,
        output,
        audio,
        gains,
        RecordingQuality::default(),
        primary_display()?,
    )
}
pub fn start_on_display(
    region: Region,
    output: PathBuf,
    audio: AudioMode,
    gains: AudioGains,
    quality: RecordingQuality,
    display: DisplayInfo,
) -> Result<Session> {
    start_with_tutorial(
        region,
        output,
        audio,
        gains,
        quality,
        display,
        tutorial::Settings::default(),
    )
}
pub fn start_with_tutorial(
    region: Region,
    output: PathBuf,
    audio: AudioMode,
    gains: AudioGains,
    quality: RecordingQuality,
    display: DisplayInfo,
    settings: tutorial::Settings,
) -> Result<Session> {
    settings.validate()?;
    if settings.active() && u64::from(region.width) * u64::from(region.height) > 16_000_000 {
        bail!("教程效果录制区域最多1600万像素");
    }
    let tutorial = Arc::new(tutorial::Control::new(
        u64::from(region.width) * u64::from(region.height) <= 16_000_000,
    ));
    tutorial.set(settings)?;
    start_controlled(
        region,
        output,
        display,
        CaptureOptions {
            audio,
            gains,
            quality,
        },
        tutorial,
    )
}
#[cfg(feature = "ui-preview")]
pub fn preview_start_controlled(
    region: Region,
    output: PathBuf,
    display: DisplayInfo,
    tutorial: Arc<tutorial::Control>,
) -> Result<Session> {
    start_controlled(
        region,
        output,
        display,
        CaptureOptions {
            audio: AudioMode::None,
            gains: AudioGains::default(),
            quality: RecordingQuality::Detailed,
        },
        tutorial,
    )
}
fn start_controlled(
    region: Region,
    output: PathBuf,
    display: DisplayInfo,
    options: CaptureOptions,
    tutorial: Arc<tutorial::Control>,
) -> Result<Session> {
    let CaptureOptions {
        audio,
        gains,
        quality,
    } = options;
    validate_request_for_display(region, &output, &display)?;
    if gains.system > 200 || gains.microphone > 200 {
        bail!("音量增益必须在 0–200% 之间");
    }
    let target = CaptureTarget {
        monitor: resolve_display(&display)?,
        region,
        display,
    };
    let (tx, events) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(PauseClock::default());
    let levels = Arc::new(AtomicU32::new(0));
    let tutorial_worker = tutorial.clone();
    let stop_worker = stop.clone();
    let pause_worker = pause.clone();
    let levels_worker = levels.clone();
    #[cfg(test)]
    let interrupt_for_test = Arc::new(AtomicU32::new(0));
    #[cfg(test)]
    let interrupt_worker = interrupt_for_test.clone();
    let worker = thread::spawn(move || {
        let signals = RecordingSignals {
            pause: pause_worker,
            levels: levels_worker,
            tutorial: tutorial_worker,
            #[cfg(test)]
            interrupt_for_test: interrupt_worker,
        };
        let result = record(
            target,
            &output,
            CaptureOptions {
                audio,
                gains,
                quality,
            },
            signals,
            &stop_worker,
            &tx,
        );
        let event = match result {
            Ok(Some(reason)) => Event::Interrupted {
                path: output,
                reason,
            },
            Ok(None) => Event::Finished(Ok(output)),
            Err(error) => Event::Finished(Err(format!("{error:#}"))),
        };
        let _ = tx.send(event);
    });
    Ok(Session {
        stop,
        pause,
        levels,
        events,
        tutorial,
        #[cfg(test)]
        interrupt_for_test,
        worker: Some(worker),
    })
}

pub fn validate_request(region: Region, output: &Path) -> Result<()> {
    validate_request_for_display(region, output, &primary_display()?)
}
pub fn validate_request_for_display(
    region: Region,
    output: &Path,
    display: &DisplayInfo,
) -> Result<()> {
    resolve_display(display)?;
    if !region.fits(display.width, display.height) {
        bail!("选区超出所选显示器，或尺寸小于 32 像素");
    }
    if output
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mp4"))
    {
        bail!("输出文件需要 .mp4 后缀");
    }
    if output.exists() {
        bail!("输出文件已存在，请选择新名称");
    }
    let parent = output.parent().context("输出路径没有文件夹")?;
    if !parent.is_dir() {
        bail!("输出文件夹不存在");
    }
    Ok(())
}

fn record(
    target: CaptureTarget,
    output: &Path,
    options: CaptureOptions,
    signals: RecordingSignals,
    stop: &AtomicBool,
    tx: &crossbeam_channel::Sender<Event>,
) -> Result<Option<String>> {
    let filename = output
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("Zi-Recording");
    let temp = output.with_file_name(format!(
        ".{filename}.{}.recording.mp4",
        uuid::Uuid::new_v4()
    ));
    let warning = match record_to_temp(target, &temp, options, signals, stop, tx) {
        Ok(warning) => warning,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
    };
    commit_temp(&temp, output)?;
    Ok(warning)
}

fn commit_temp(temp: &Path, output: &Path) -> Result<()> {
    // MoveFileW fails when the destination exists; unlike a replace-capable rename it preserves
    // another file that appeared during recording. Source and target are in the same directory.
    let from: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(temp.as_os_str())
        .chain(Some(0))
        .collect();
    let to: Vec<u16> = std::os::windows::ffi::OsStrExt::encode_wide(output.as_os_str())
        .chain(Some(0))
        .collect();
    if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) }
        == 0
    {
        bail!(
            "视频已录制，但无法保存到目标路径（可能出现同名文件）：{}；完整临时视频保留在 {}",
            std::io::Error::last_os_error(),
            temp.display()
        );
    }
    Ok(())
}

fn record_to_temp(
    target: CaptureTarget,
    output: &Path,
    options: CaptureOptions,
    signals: RecordingSignals,
    stop: &AtomicBool,
    tx: &crossbeam_channel::Sender<Event>,
) -> Result<Option<String>> {
    // Device startup may take seconds. Complete it before the video clock begins so that the
    // output never includes an invisible pre-roll while the UI still says "starting".
    let mut audio_timeline =
        AudioTimeline::new(options.audio, options.gains, signals.levels.clone())
            .context("音频设备启动失败")?;
    if let Some(timeline) = &mut audio_timeline
        && let Err(error) = timeline.discard_initial()
    {
        let _ = timeline.stop();
        return Err(error.context("清理音频启动缓冲失败"));
    }
    let CaptureTarget {
        monitor,
        region,
        display,
    } = target;
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::WithCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        CaptureFlags {
            region,
            path: output.to_path_buf(),
            audio: options.audio,
            quality: options.quality,
            pause: signals.pause.clone(),
            tutorial: signals.tutorial.clone(),
            origin: [display.x, display.y],
        },
    );
    let control = match Capture::start_free_threaded(settings) {
        Ok(control) => control,
        Err(error) => {
            if let Some(timeline) = &mut audio_timeline {
                let _ = timeline.stop();
            }
            return Err(anyhow!("捕获启动失败：{error}"));
        }
    };
    let callback = control.callback();
    let capture_started = callback.lock().started;
    let _ = tx.send(Event::Started);
    let mut warnings = Vec::new();
    let mut audio_fault = false;
    let mut last_topology_check = Instant::now();
    while !stop.load(Ordering::Acquire) && !control.is_finished() {
        #[cfg(test)]
        if let injected @ 1..=2 = signals.interrupt_for_test.load(Ordering::Acquire) {
            if injected == 2 {
                warnings.push("测试模拟音频采集中断".to_owned());
                audio_fault = true;
            } else {
                warnings.push("测试模拟画面采集中断".to_owned());
            }
            break;
        }
        if last_topology_check.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = validate_display(&display) {
                warnings.push(format!("显示器在录制中变化或断开：{error:#}"));
                break;
            }
            last_topology_check = Instant::now();
        }
        if let Some(timeline) = &mut audio_timeline {
            if let Err(error) =
                timeline.tick(&callback, signals.pause.elapsed_since(capture_started))
            {
                warnings.push(format!("音频捕获失败：{error:#}"));
                audio_fault = true;
                break;
            }
        }
        if let Some(elapsed) = signals.pause.elapsed_since(capture_started)
            && let Err(error) = callback.lock().keep_video_alive(elapsed)
        {
            warnings.push(format!("维持视频时间线失败：{error:#}"));
            break;
        }
        if signals.pause.is_paused() {
            callback.lock().effects.suspend();
        }
        thread::sleep(Duration::from_millis(
            if audio_timeline.is_some() || signals.tutorial.snapshot().active() {
                10
            } else {
                80
            },
        ));
    }
    if control.is_finished() && !stop.load(Ordering::Acquire) && warnings.is_empty() {
        warnings.push("画面采集意外结束".to_owned());
    }
    // Capture shutdown can take seconds. Freeze the timeline before stopping it so the
    // terminal video frame and audio tail refer to the same user-visible stop instant.
    let final_duration = signals.pause.duration_at_stop(capture_started);
    if !audio_fault
        && let Some(timeline) = &mut audio_timeline
        && let Err(error) = timeline.tick(&callback, Some(final_duration))
    {
        warnings.push(format!("补齐末尾音频失败：{error:#}"));
    }
    if let Some(timeline) = &mut audio_timeline
        && let Err(error) = timeline.stop()
    {
        warnings.push(format!("停止音频设备失败：{error:#}"));
    }
    signals.levels.store(0, Ordering::Release);
    signals.pause.set_paused(true);
    if let Err(error) = control.stop() {
        warnings.push(format!("停止画面采集失败：{error}"));
    }
    let (extend_error, encoder) = {
        let mut capture = callback.lock();
        if capture.flipped.is_empty() {
            bail!("没有捕获到画面帧，未创建空视频");
        }
        let extend_error = capture.extend_to_stop(final_duration).err();
        let encoder = capture.encoder.take().context("编码器未创建")?;
        (extend_error, encoder)
    };
    if let Some(error) = extend_error {
        warnings.push(format!("补齐最后画面失败：{error:#}"));
    }
    encoder
        .finish()
        .map_err(|e| anyhow!("保存 MP4 失败：{e}"))?;
    Ok((!warnings.is_empty()).then(|| {
        warnings
            .into_iter()
            .map(|warning| warning.chars().take(300).collect::<String>())
            .collect::<Vec<_>>()
            .join("；")
    }))
}

struct AudioInput {
    client: AudioClient,
    capture: AudioCaptureClient,
    bytes: std::collections::VecDeque<u8>,
}
impl AudioInput {
    fn prepare(direction: Direction) -> Result<Self> {
        wasapi::initialize_mta()
            .ok()
            .map_err(|e| anyhow!("初始化音频 COM 失败：{e}"))?;
        let device = DeviceEnumerator::new()?.get_default_device(&direction)?;
        let mut client = device.get_iaudioclient()?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, 48_000, 2, None);
        let (period, _) = client.get_device_period()?;
        client.initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
        )?;
        let capture = client.get_audiocaptureclient()?;
        Ok(Self {
            client,
            capture,
            bytes: std::collections::VecDeque::new(),
        })
    }
    fn start(&mut self) -> Result<()> {
        self.client.start_stream()?;
        Ok(())
    }
    fn drain_bytes(&mut self) -> Result<Vec<u8>> {
        for _ in 0..32 {
            if self.capture.get_next_packet_size()?.unwrap_or(0) == 0 {
                break;
            }
            self.capture.read_from_device_to_deque(&mut self.bytes)?;
        }
        Ok(self.bytes.drain(..).collect())
    }
    fn stop(&mut self) -> Result<()> {
        self.client.stop_stream()?;
        Ok(())
    }
}

const AUDIO_RATE: u64 = 48_000;
const AUDIO_CHANNELS: usize = 2;
const AUDIO_CHUNK_FRAMES: usize = 480;
const MAX_PENDING_AUDIO_FRAMES: usize = 4_800;

fn pcm_level(bytes: &[u8], gain: u16, source_count: usize) -> u8 {
    if gain == 0 || bytes.is_empty() {
        return 0;
    }
    let peak = bytes
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as i32)
        .map(i32::abs)
        .max()
        .unwrap_or(0) as f64;
    if peak == 0.0 {
        return 0;
    }
    let divisor = if source_count == 2 { 200.0 } else { 100.0 };
    let amplitude = (peak * f64::from(gain) / divisor).min(32_768.0);
    let dbfs = 20.0 * (amplitude / 32_768.0).log10();
    ((dbfs + 60.0) * (100.0 / 60.0)).round().clamp(0.0, 100.0) as u8
}

pub fn unpack_levels(packed: u32) -> (u8, u8) {
    ((packed & 0xff) as u8, ((packed >> 8) & 0xff) as u8)
}

struct PcmMixer {
    queues: Vec<std::collections::VecDeque<i16>>,
    sent_frames: u64,
    gains: Vec<u16>,
}
impl PcmMixer {
    fn new(gains: Vec<u16>) -> Self {
        let sources = gains.len();
        assert!((1..=2).contains(&sources));
        Self {
            queues: (0..sources)
                .map(|_| std::collections::VecDeque::new())
                .collect(),
            sent_frames: 0,
            gains,
        }
    }
    fn push(&mut self, source: usize, bytes: &[u8]) -> Result<()> {
        if !bytes
            .len()
            .is_multiple_of(AUDIO_CHANNELS * std::mem::size_of::<i16>())
        {
            bail!("音频设备返回了不完整的 PCM 采样帧");
        }
        let queue = &mut self.queues[source];
        queue.extend(
            bytes
                .chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]])),
        );
        let excess = queue
            .len()
            .saturating_sub(MAX_PENDING_AUDIO_FRAMES * AUDIO_CHANNELS);
        queue.drain(..excess);
        Ok(())
    }
    fn clear_pending(&mut self) {
        for queue in &mut self.queues {
            queue.clear();
        }
    }
    fn next_chunk(&mut self, target_frames: u64) -> Option<Vec<u8>> {
        let remaining = target_frames.saturating_sub(self.sent_frames);
        if remaining == 0 {
            return None;
        }
        let frames = remaining.min(AUDIO_CHUNK_FRAMES as u64) as usize;
        let mut bytes = Vec::with_capacity(frames * AUDIO_CHANNELS * 2);
        for _ in 0..frames * AUDIO_CHANNELS {
            let first = self.queues[0].pop_front().unwrap_or(0) as i32;
            let weighted = if self.queues.len() == 2 {
                let second = self.queues[1].pop_front().unwrap_or(0) as i32;
                (first * self.gains[0] as i32 + second * self.gains[1] as i32) / 200
            } else {
                first * self.gains[0] as i32 / 100
            };
            let mixed = weighted.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            bytes.extend_from_slice(&mixed.to_le_bytes());
        }
        self.sent_frames += frames as u64;
        Some(bytes)
    }
}

fn audio_frames_at(elapsed: Duration) -> u64 {
    (elapsed.as_nanos().saturating_mul(AUDIO_RATE as u128) / 1_000_000_000).min(u64::MAX as u128)
        as u64
}

struct AudioTimeline {
    inputs: Vec<AudioInput>,
    mixer: PcmMixer,
    levels: Arc<AtomicU32>,
    level_slots: Vec<usize>,
    displayed_levels: [u8; 2],
}
impl AudioTimeline {
    fn new(mode: AudioMode, gains: AudioGains, levels: Arc<AtomicU32>) -> Result<Option<Self>> {
        let mut inputs = Vec::new();
        let mut input_gains = Vec::new();
        let mut level_slots = Vec::new();
        if matches!(mode, AudioMode::System | AudioMode::SystemAndMicrophone) {
            inputs.push(AudioInput::prepare(Direction::Render).context("系统播放设备不可用")?);
            input_gains.push(gains.system);
            level_slots.push(0);
        }
        if matches!(mode, AudioMode::Microphone | AudioMode::SystemAndMicrophone) {
            inputs.push(AudioInput::prepare(Direction::Capture).context("默认麦克风不可用")?);
            input_gains.push(gains.microphone);
            level_slots.push(1);
        }
        if inputs.is_empty() {
            return Ok(None);
        }
        for index in 0..inputs.len() {
            if let Err(error) = inputs[index].start() {
                for input in &mut inputs[..index] {
                    let _ = input.stop();
                }
                return Err(error.context("启动音频采集流失败"));
            }
        }
        Ok(Some(Self {
            mixer: PcmMixer::new(input_gains),
            inputs,
            levels,
            level_slots,
            displayed_levels: [0, 0],
        }))
    }
    fn discard_initial(&mut self) -> Result<()> {
        for input in &mut self.inputs {
            let _ = input.drain_bytes()?;
        }
        self.mixer.clear_pending();
        Ok(())
    }
    fn tick(
        &mut self,
        callback: &std::sync::Arc<parking_lot::Mutex<Capture>>,
        elapsed: Option<Duration>,
    ) -> Result<()> {
        let source_count = self.inputs.len();
        for (index, input) in self.inputs.iter_mut().enumerate() {
            let bytes = input.drain_bytes()?;
            if elapsed.is_some() {
                let slot = self.level_slots[index];
                let measured = pcm_level(&bytes, self.mixer.gains[index], source_count);
                self.displayed_levels[slot] =
                    measured.max(self.displayed_levels[slot].saturating_sub(2));
                self.mixer.push(index, &bytes)?;
            }
        }
        let Some(elapsed) = elapsed else {
            self.mixer.clear_pending();
            self.displayed_levels = [0, 0];
            self.levels.store(0, Ordering::Release);
            return Ok(());
        };
        let [system, microphone] = self.displayed_levels;
        self.levels.store(
            u32::from(system) | (u32::from(microphone) << 8),
            Ordering::Release,
        );
        let target_frames = audio_frames_at(elapsed);
        while let Some(bytes) = self.mixer.next_chunk(target_frames) {
            callback
                .lock()
                .encoder
                .as_mut()
                .context("编码器已停止")?
                .send_audio_buffer(&bytes, 0)?;
        }
        Ok(())
    }
    fn stop(&mut self) -> Result<()> {
        let mut first_error = None;
        for input in &mut self.inputs {
            if let Err(error) = input.stop() {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quality_presets_increase_estimated_size_and_audio_adds_capacity() {
        use super::{AudioMode, RecordingQuality};
        let compact = RecordingQuality::Compact.estimated_megabytes(60, AudioMode::None);
        let balanced = RecordingQuality::Balanced.estimated_megabytes(60, AudioMode::None);
        let detailed = RecordingQuality::Detailed.estimated_megabytes(60, AudioMode::None);
        assert!(compact > 25.0 && compact < balanced && balanced < detailed);
        assert!(
            RecordingQuality::Balanced.estimated_megabytes(60, AudioMode::Microphone) > balanced
        );
    }
    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and H.264 encoder"]
    fn quality_presets_produce_real_mp4_files() {
        use super::{AudioGains, AudioMode, Event, RecordingQuality, Region};
        use std::{sync::atomic::Ordering, time::Duration};
        let display = super::primary_display().unwrap();
        for quality in [RecordingQuality::Compact, RecordingQuality::Detailed] {
            let path = std::env::temp_dir().join(format!(
                "zi-quality-{quality:?}-{}.mp4",
                uuid::Uuid::new_v4()
            ));
            let session = super::start_on_display(
                Region {
                    x: 0,
                    y: 0,
                    width: 320,
                    height: 180,
                },
                path.clone(),
                AudioMode::None,
                AudioGains::default(),
                quality,
                display.clone(),
            )
            .unwrap();
            assert!(matches!(
                session
                    .events
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap(),
                Event::Started
            ));
            std::thread::sleep(Duration::from_secs(2));
            session.stop.store(true, Ordering::Release);
            assert!(matches!(
                session
                    .events
                    .recv_timeout(Duration::from_secs(20))
                    .unwrap(),
                Event::Finished(Ok(_))
            ));
            let data = std::fs::read(&path).unwrap();
            assert!(data.len() > 1024 && data.windows(4).any(|part| part == b"moov"));
            eprintln!("{quality:?}: {} bytes", data.len());
            std::fs::remove_file(path).unwrap();
        }
    }
    use super::{AudioMode, Event, Region};
    use std::{
        sync::{Mutex, OnceLock, atomic::Ordering},
        time::Duration,
    };
    #[test]
    fn region_maps_reverse_drag_and_even_dimensions() {
        let r = Region::from_points((600.0, 400.0), (100.0, 100.0), (1920, 1080), (960.0, 540.0))
            .unwrap();
        assert_eq!(
            r,
            Region {
                x: 200,
                y: 200,
                width: 1000,
                height: 600
            }
        );
        assert!(r.fits(1920, 1080));
        assert!(
            Region::from_points((0.0, 0.0), (10.0, 10.0), (1920, 1080), (960.0, 540.0)).is_none()
        );
    }
    #[test]
    fn display_identity_rejects_topology_changes_and_keeps_local_coordinates() {
        let display = super::DisplayInfo {
            device_name: "\\\\.\\DISPLAY2".into(),
            name: "Side display".into(),
            x: -1_920,
            y: -120,
            width: 1_920,
            height: 1_080,
            primary: false,
        };
        assert!(display.label().contains("DISPLAY2"));
        assert!(display.same_topology(&display.clone()));
        let mut moved = display.clone();
        moved.x = 0;
        assert!(!display.same_topology(&moved));
        let mut resized = display.clone();
        resized.width = 1_600;
        assert!(!display.same_topology(&resized));
        let mut primary_changed = display.clone();
        primary_changed.primary = true;
        assert!(!display.same_topology(&primary_changed));
        let region = Region::from_points(
            (0.0, 0.0),
            (960.0, 540.0),
            (display.width, display.height),
            (960.0, 540.0),
        )
        .unwrap();
        assert_eq!((region.x, region.y), (0, 0));
        assert!(region.fits(display.width, display.height));
    }
    #[test]
    fn display_snapshot_is_checked_before_recording() {
        let current = super::primary_display().unwrap();
        eprintln!(
            "primary capture bounds: ({}, {}) {}x{}",
            current.x, current.y, current.width, current.height
        );
        super::validate_display(&current).unwrap();
        let mut stale = current.clone();
        stale.width += 2;
        assert!(super::validate_display(&stale).is_err());
        let region = Region {
            x: 0,
            y: 0,
            width: current.width.min(640) & !1,
            height: current.height.min(360) & !1,
        };
        let path =
            std::env::temp_dir().join(format!("zi-stale-display-{}.mp4", uuid::Uuid::new_v4()));
        assert!(super::validate_request_for_display(region, &path, &stale).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn finishing_never_overwrites_a_newer_destination() {
        let folder = std::env::temp_dir();
        let unique = uuid::Uuid::new_v4();
        let source = folder.join(format!("zi-recording-{unique}-partial.mp4"));
        let target = folder.join(format!("zi-recording-{unique}.mp4"));
        std::fs::write(&source, b"completed capture").unwrap();
        std::fs::write(&target, b"another file").unwrap();
        assert!(super::commit_temp(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"another file");
        assert_eq!(std::fs::read(&source).unwrap(), b"completed capture");
        std::fs::remove_file(source).unwrap();
        std::fs::remove_file(target).unwrap();
    }
    #[test]
    fn pause_clock_excludes_time_without_frame_callbacks() {
        let clock = super::PauseClock::default();
        let started = std::time::Instant::now();
        std::thread::sleep(Duration::from_millis(15));
        clock.set_paused(true);
        assert!(clock.elapsed_since(started).is_none());
        std::thread::sleep(Duration::from_millis(35));
        clock.set_paused(false);
        let elapsed = clock.elapsed_since(started).unwrap();
        assert!(
            elapsed < Duration::from_millis(40),
            "paused interval leaked: {elapsed:?}"
        );
        clock.set_paused(true);
        std::thread::sleep(Duration::from_millis(20));
        let at_stop = clock.duration_at_stop(started);
        assert!(
            at_stop < Duration::from_millis(45),
            "stop while paused leaked time: {at_stop:?}"
        );
    }
    #[test]
    fn audio_clock_fills_silence_and_resumes_without_replaying_paused_samples() {
        let mut mixer = super::PcmMixer::new(vec![100]);
        let samples = [1_000i16, -1_000, 2_000, -2_000];
        let bytes: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        mixer.push(0, &bytes).unwrap();
        let chunk = mixer.next_chunk(4).unwrap();
        let output: Vec<_> = chunk
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]))
            .collect();
        assert_eq!(output, [1_000, -1_000, 2_000, -2_000, 0, 0, 0, 0]);
        assert!(mixer.next_chunk(4).is_none());
        mixer.push(0, &bytes).unwrap();
        mixer.clear_pending();
        assert_eq!(mixer.next_chunk(6).unwrap(), vec![0; 8]);
        assert_eq!(super::audio_frames_at(Duration::from_millis(20)), 960);
    }
    #[test]
    fn audio_mixer_bounds_backlog_and_averages_both_sources() {
        let mut mixer = super::PcmMixer::new(vec![100, 100]);
        let first: Vec<u8> = (0..5_000i16)
            .flat_map(|frame| [frame, frame].into_iter().flat_map(i16::to_le_bytes))
            .collect();
        mixer.push(0, &first).unwrap();
        assert_eq!(mixer.queues[0].len(), super::MAX_PENDING_AUDIO_FRAMES * 2);
        mixer.push(1, &[0xff, 0x7f, 0x00, 0x80]).unwrap();
        let chunk = mixer.next_chunk(1).unwrap();
        let left = i16::from_le_bytes([chunk[0], chunk[1]]);
        let right = i16::from_le_bytes([chunk[2], chunk[3]]);
        assert_eq!(left, ((200 + i16::MAX as i32) / 2) as i16);
        assert_eq!(right, ((200 + i16::MIN as i32) / 2) as i16);
    }
    #[test]
    fn audio_gains_scale_each_source_and_limit_clipping() {
        let stereo = |sample: i16| {
            [sample, sample]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>()
        };
        let left = |bytes: &[u8]| i16::from_le_bytes([bytes[0], bytes[1]]);
        let mut mixed = super::PcmMixer::new(vec![200, 0]);
        mixed.push(0, &stereo(20_000)).unwrap();
        mixed.push(1, &stereo(-20_000)).unwrap();
        assert_eq!(left(&mixed.next_chunk(1).unwrap()), 20_000);
        let mut clipped = super::PcmMixer::new(vec![200, 200]);
        clipped.push(0, &stereo(30_000)).unwrap();
        clipped.push(1, &stereo(30_000)).unwrap();
        assert_eq!(left(&clipped.next_chunk(1).unwrap()), i16::MAX);
        let mut single = super::PcmMixer::new(vec![50]);
        single.push(0, &stereo(-20_000)).unwrap();
        assert_eq!(left(&single.next_chunk(1).unwrap()), -10_000);
    }
    #[test]
    fn audio_meter_maps_pcm_peaks_and_mute_without_overflow() {
        let sample = |value: i16| value.to_le_bytes();
        assert_eq!(super::pcm_level(&sample(0), 100, 1), 0);
        assert_eq!(super::pcm_level(&sample(i16::MIN), 0, 1), 0);
        assert_eq!(super::pcm_level(&sample(i16::MIN), 100, 1), 100);
        assert_eq!(super::pcm_level(&sample(i16::MIN), 200, 2), 100);
        assert!(
            super::pcm_level(&sample(8_000), 100, 2) < super::pcm_level(&sample(8_000), 100, 1)
        );
        assert_eq!(super::unpack_levels(62 | (38 << 8)), (62, 38));
    }
    #[test]
    #[ignore = "needs an unlocked interactive Windows desktop and a working H.264 encoder"]
    fn records_a_playable_mp4_on_the_real_desktop() {
        record_smoke(AudioMode::None, false, 2);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default playback device"]
    fn records_system_audio_with_video() {
        record_smoke(AudioMode::System, false, 2);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default microphone"]
    fn records_microphone_with_video() {
        record_smoke(AudioMode::Microphone, false, 2);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default playback and microphone devices"]
    fn records_system_and_microphone_with_video() {
        record_smoke(AudioMode::SystemAndMicrophone, true, 2);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default microphone; records for 12 seconds"]
    fn records_twelve_seconds_with_synchronized_audio() {
        record_smoke(AudioMode::Microphone, false, 12);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and both audio devices; records for two minutes"]
    fn records_two_minutes_with_dual_audio_and_pause() {
        record_smoke(AudioMode::SystemAndMicrophone, true, 120);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default playback device"]
    fn system_audio_meter_reacts_to_a_local_pcm_tone() {
        use std::{process::Command, sync::atomic::Ordering, time::Instant};
        let sample_rate = 48_000u32;
        let frames = sample_rate * 4;
        let data_len = frames * 2;
        let mut wave = Vec::with_capacity(44 + data_len as usize);
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&(36 + data_len).to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16u32.to_le_bytes());
        wave.extend_from_slice(&1u16.to_le_bytes());
        wave.extend_from_slice(&1u16.to_le_bytes());
        wave.extend_from_slice(&sample_rate.to_le_bytes());
        wave.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wave.extend_from_slice(&2u16.to_le_bytes());
        wave.extend_from_slice(&16u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&data_len.to_le_bytes());
        for index in 0..frames {
            let phase = index as f32 * std::f32::consts::TAU * 440.0 / sample_rate as f32;
            let sample = (phase.sin() * 12_000.0) as i16;
            wave.extend_from_slice(&sample.to_le_bytes());
        }
        let unique = uuid::Uuid::new_v4();
        let tone = std::env::temp_dir().join(format!("zi-recorder-tone-{unique}.wav"));
        let video = std::env::temp_dir().join(format!("zi-recorder-tone-{unique}.mp4"));
        std::fs::write(&tone, wave).unwrap();
        let mut player = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "$player = New-Object System.Media.SoundPlayer $env:ZI_TEST_TONE; $player.PlaySync()",
            ])
            .env("ZI_TEST_TONE", &tone)
            .spawn()
            .unwrap();
        let (width, height) = super::primary_size().unwrap();
        let region = Region {
            x: 0,
            y: 0,
            width: width.min(640) & !1,
            height: height.min(360) & !1,
        };
        let session = super::start(region, video.clone(), AudioMode::System).unwrap();
        assert!(matches!(
            session
                .events
                .recv_timeout(Duration::from_secs(15))
                .unwrap(),
            Event::Started
        ));
        let mut max_level = 0;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            max_level =
                max_level.max(super::unpack_levels(session.levels.load(Ordering::Acquire)).0);
            std::thread::sleep(Duration::from_millis(10));
        }
        session.stop.store(true, Ordering::Release);
        assert!(matches!(
            session
                .events
                .recv_timeout(Duration::from_secs(30))
                .unwrap(),
            Event::Finished(Ok(_))
        ));
        let _ = player.wait();
        std::fs::remove_file(tone).unwrap();
        std::fs::remove_file(video).unwrap();
        eprintln!("system audio meter peak: {max_level}/100");
        assert!(
            max_level > 20,
            "system audio meter stayed silent: {max_level}"
        );
    }
    fn record_smoke(audio: AudioMode, pause_during_capture: bool, capture_seconds: u64) {
        static CAPTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = CAPTURE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (w, h) = super::primary_size().unwrap();
        let region = Region {
            x: 0,
            y: 0,
            width: w.min(640) & !1,
            height: h.min(360) & !1,
        };
        let path = std::env::temp_dir().join(format!(
            "zi-recorder-smoke-{audio:?}-{}.mp4",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let launch_started = std::time::Instant::now();
        let session = super::start_on_display(
            region,
            path.clone(),
            audio,
            super::AudioGains::default(),
            super::RecordingQuality::default(),
            super::primary_display().unwrap(),
        )
        .unwrap();
        let first = session
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap();
        assert!(
            matches!(first, Event::Started),
            "unexpected recorder event: {first:?}"
        );
        eprintln!(
            "{audio:?} capture started after {:?}",
            launch_started.elapsed()
        );
        if pause_during_capture {
            std::thread::sleep(Duration::from_secs(capture_seconds / 2));
            session.pause.set_paused(true);
            std::thread::sleep(Duration::from_secs(3));
            assert_eq!(session.levels.load(Ordering::Acquire), 0);
            session.pause.set_paused(false);
            std::thread::sleep(Duration::from_secs(capture_seconds - capture_seconds / 2));
        } else {
            std::thread::sleep(Duration::from_secs(capture_seconds));
        }
        session.stop.store(true, Ordering::Release);
        match session
            .events
            .recv_timeout(Duration::from_secs(30))
            .unwrap()
        {
            Event::Finished(Ok(actual)) => assert_eq!(actual, path),
            other => panic!("recording failed: {other:?}"),
        }
        let data = std::fs::read(&path).unwrap();
        assert!(data.len() > 1024, "MP4 unexpectedly small");
        assert_eq!(&data[4..8], b"ftyp");
        assert!(
            data.windows(4).any(|v| v == b"moov"),
            "MP4 has no finalized movie atom"
        );
        if audio != AudioMode::None {
            assert!(
                data.windows(4).any(|v| v == b"soun"),
                "MP4 has no audio track"
            );
        }
        let tracks = mp4_track_durations(&data);
        let video_seconds = tracks.iter().find(|(kind, _)| *kind == *b"vide").unwrap().1;
        let expected_seconds = capture_seconds as f64;
        eprintln!("recorded MP4 tracks: {tracks:?}, target={expected_seconds:.3}s");
        assert!(
            (video_seconds - expected_seconds).abs() < 0.75,
            "unexpected video duration: {video_seconds}"
        );
        if audio != AudioMode::None {
            let audio_seconds = tracks.iter().find(|(kind, _)| *kind == *b"soun").unwrap().1;
            assert!(
                (audio_seconds - expected_seconds).abs() < 0.75,
                "unexpected audio duration: {audio_seconds}"
            );
            assert!(
                (audio_seconds - video_seconds).abs() < 0.75,
                "audio/video drift: audio={audio_seconds:.3}s video={video_seconds:.3}s"
            );
        }
        if std::env::var_os("ZI_RECORDER_KEEP_SMOKE").is_some() {
            eprintln!("Recorder smoke file: {}", path.display());
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }

    fn boxes_of<'a>(data: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
        let mut result = Vec::new();
        let mut offset = 0usize;
        while data.len().saturating_sub(offset) >= 8 {
            let declared = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
            let (size, header) = match declared {
                0 => (data.len() - offset, 8),
                1 if data.len().saturating_sub(offset) >= 16 => (
                    u64::from_be_bytes(data[offset + 8..offset + 16].try_into().unwrap()) as usize,
                    16,
                ),
                _ => (declared as usize, 8),
            };
            if size < header || size > data.len() - offset {
                break;
            }
            if &data[offset + 4..offset + 8] == kind {
                result.push(&data[offset + header..offset + size]);
            }
            offset += size;
        }
        result
    }
    fn mp4_track_durations(data: &[u8]) -> Vec<([u8; 4], f64)> {
        let mut result = Vec::new();
        for movie in boxes_of(data, b"moov") {
            for track in boxes_of(movie, b"trak") {
                for media in boxes_of(track, b"mdia") {
                    let Some(handler) = boxes_of(media, b"hdlr").into_iter().next() else {
                        continue;
                    };
                    let Some(header) = boxes_of(media, b"mdhd").into_iter().next() else {
                        continue;
                    };
                    if handler.len() < 12 || header.len() < 20 {
                        continue;
                    }
                    let kind: [u8; 4] = handler[8..12].try_into().unwrap();
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
}
