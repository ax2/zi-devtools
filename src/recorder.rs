//! Local primary-monitor capture. Encoding stays on a worker thread.
use anyhow::{Context as _, Result, anyhow, bail};
use crossbeam_channel::{Receiver, unbounded};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMode {
    #[default]
    None,
    System,
    Microphone,
    SystemAndMicrophone,
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
}
pub struct Session {
    pub stop: Arc<AtomicBool>,
    pub pause: Arc<PauseClock>,
    pub events: Receiver<Event>,
    worker: Option<thread::JoinHandle<()>>,
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
}
type CaptureError = Box<dyn std::error::Error + Send + Sync>;
impl GraphicsCaptureApiHandler for Capture {
    type Flags = (Region, PathBuf, AudioMode, Arc<PauseClock>);
    type Error = CaptureError;
    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        let (region, path, audio, pause) = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(region.width, region.height)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(30)
                .bitrate(8_000_000),
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
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        let Some(elapsed) = self.pause.elapsed_since(self.started) else {
            return Ok(());
        };
        let r = self.region;
        if !r.fits(frame.width(), frame.height()) {
            return Err(anyhow!("显示器尺寸在录制中变化；已停止录制").into());
        }
        let buffer = frame.buffer_crop(r.x, r.y, r.x + r.width, r.y + r.height)?;
        let pixels = buffer.as_nopadding_buffer(&mut self.scratch);
        let stride = r.width as usize * 4;
        self.flipped.resize(pixels.len(), 0);
        for row in 0..r.height as usize {
            self.flipped[row * stride..(row + 1) * stride].copy_from_slice(
                &pixels[(r.height as usize - 1 - row) * stride..(r.height as usize - row) * stride],
            );
        }
        let timestamp = elapsed.as_nanos().saturating_div(100).min(i64::MAX as u128) as i64;
        self.encoder
            .as_mut()
            .ok_or_else(|| anyhow!("视频编码器已停止"))?
            .send_frame_buffer(&self.flipped, timestamp)?;
        Ok(())
    }
}
impl Capture {
    fn extend_to_stop(&mut self) -> Result<()> {
        if self.flipped.is_empty() {
            bail!("没有捕获到画面帧，未创建空视频");
        }
        let timestamp = self
            .pause
            .duration_at_stop(self.started)
            .as_nanos()
            .saturating_div(100)
            .min(i64::MAX as u128) as i64;
        self.encoder
            .as_mut()
            .context("编码器已停止")?
            .send_frame_buffer(&self.flipped, timestamp)?;
        Ok(())
    }
}

pub fn primary_size() -> Result<(u32, u32)> {
    let m = Monitor::primary()?;
    Ok((m.width()?, m.height()?))
}
pub fn start(region: Region, output: PathBuf, audio: AudioMode) -> Result<Session> {
    validate_request(region, &output)?;
    let (tx, events) = unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(PauseClock::default());
    let stop_worker = stop.clone();
    let pause_worker = pause.clone();
    let worker = thread::spawn(move || {
        let result = record(region, &output, audio, pause_worker, &stop_worker, &tx);
        let _ = tx.send(Event::Finished(
            result.map(|_| output).map_err(|e| format!("{e:#}")),
        ));
    });
    Ok(Session {
        stop,
        pause,
        events,
        worker: Some(worker),
    })
}

pub fn validate_request(region: Region, output: &Path) -> Result<()> {
    let (width, height) = primary_size()?;
    if !region.fits(width, height) {
        bail!("选区超出主显示器，或尺寸小于 32 像素");
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
    region: Region,
    output: &Path,
    audio: AudioMode,
    pause: Arc<PauseClock>,
    stop: &AtomicBool,
    tx: &crossbeam_channel::Sender<Event>,
) -> Result<()> {
    let filename = output
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("Zi-Recording");
    let temp = output.with_file_name(format!(
        ".{filename}.{}.recording.mp4",
        uuid::Uuid::new_v4()
    ));
    let result = record_to_temp(region, &temp, audio, pause, stop, tx);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    commit_temp(&temp, output)?;
    Ok(())
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
    region: Region,
    output: &Path,
    audio: AudioMode,
    pause: Arc<PauseClock>,
    stop: &AtomicBool,
    tx: &crossbeam_channel::Sender<Event>,
) -> Result<()> {
    // Device startup may take seconds. Complete it before the video clock begins so that the
    // output never includes an invisible pre-roll while the UI still says "starting".
    let mut audio_timeline = AudioTimeline::new(audio).context("音频设备启动失败")?;
    if let Some(timeline) = &mut audio_timeline
        && let Err(error) = timeline.discard_initial()
    {
        let _ = timeline.stop();
        return Err(error.context("清理音频启动缓冲失败"));
    }
    let settings = Settings::new(
        Monitor::primary()?,
        CursorCaptureSettings::WithCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(33)),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        (region, output.to_path_buf(), audio, pause.clone()),
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
    while !stop.load(Ordering::Acquire) && !control.is_finished() {
        if let Some(timeline) = &mut audio_timeline {
            if let Err(e) = timeline.tick(&callback, pause.elapsed_since(capture_started)) {
                let _ = timeline.stop();
                let _ = control.stop();
                return Err(e.context("音频捕获失败"));
            }
        }
        thread::sleep(Duration::from_millis(if audio_timeline.is_some() {
            10
        } else {
            80
        }));
    }
    let audio_tail = audio_timeline
        .as_mut()
        .map(|timeline| timeline.tick(&callback, Some(pause.duration_at_stop(capture_started))))
        .transpose();
    let audio_stop = audio_timeline.as_mut().map(AudioTimeline::stop).transpose();
    let capture_stop = control.stop().map_err(|e| anyhow!("停止捕获失败：{e}"));
    audio_tail?;
    audio_stop?;
    capture_stop?;
    let encoder = {
        let mut capture = callback.lock();
        capture.extend_to_stop()?;
        capture.encoder.take().context("编码器未创建")?
    };
    encoder
        .finish()
        .map_err(|e| anyhow!("保存 MP4 失败：{e}"))?;
    Ok(())
}

struct AudioInput {
    client: AudioClient,
    capture: AudioCaptureClient,
    bytes: std::collections::VecDeque<u8>,
}
impl AudioInput {
    fn new(direction: Direction) -> Result<Self> {
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
        client.start_stream()?;
        Ok(Self {
            client,
            capture,
            bytes: std::collections::VecDeque::new(),
        })
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

struct PcmMixer {
    queues: Vec<std::collections::VecDeque<i16>>,
    sent_frames: u64,
}
impl PcmMixer {
    fn new(sources: usize) -> Self {
        assert!((1..=2).contains(&sources));
        Self {
            queues: (0..sources)
                .map(|_| std::collections::VecDeque::new())
                .collect(),
            sent_frames: 0,
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
            let mixed = if self.queues.len() == 2 {
                let second = self.queues[1].pop_front().unwrap_or(0) as i32;
                ((first + second) / 2) as i16
            } else {
                first as i16
            };
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
}
impl AudioTimeline {
    fn new(mode: AudioMode) -> Result<Option<Self>> {
        let mut inputs = Vec::new();
        if matches!(mode, AudioMode::System | AudioMode::SystemAndMicrophone) {
            inputs.push(AudioInput::new(Direction::Render).context("系统播放设备不可用")?);
        }
        if matches!(mode, AudioMode::Microphone | AudioMode::SystemAndMicrophone) {
            match AudioInput::new(Direction::Capture).context("默认麦克风不可用") {
                Ok(input) => inputs.push(input),
                Err(error) => {
                    for input in &mut inputs {
                        let _ = input.stop();
                    }
                    return Err(error);
                }
            }
        }
        if inputs.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            mixer: PcmMixer::new(inputs.len()),
            inputs,
        }))
    }
    fn discard_initial(&mut self) -> Result<()> {
        for input in &mut self.inputs {
            for _ in 0..8 {
                if input.drain_bytes()?.is_empty() {
                    break;
                }
            }
        }
        self.mixer.clear_pending();
        Ok(())
    }
    fn tick(
        &mut self,
        callback: &std::sync::Arc<parking_lot::Mutex<Capture>>,
        elapsed: Option<Duration>,
    ) -> Result<()> {
        for (index, input) in self.inputs.iter_mut().enumerate() {
            let bytes = input.drain_bytes()?;
            if elapsed.is_some() {
                self.mixer.push(index, &bytes)?;
            }
        }
        let Some(elapsed) = elapsed else {
            self.mixer.clear_pending();
            return Ok(());
        };
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
        let mut mixer = super::PcmMixer::new(1);
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
        let mut mixer = super::PcmMixer::new(2);
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
    #[ignore = "needs an unlocked interactive Windows desktop and a working H.264 encoder"]
    fn records_a_playable_mp4_on_the_real_desktop() {
        record_smoke(AudioMode::None, false);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default playback device"]
    fn records_system_audio_with_video() {
        record_smoke(AudioMode::System, false);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default microphone"]
    fn records_microphone_with_video() {
        record_smoke(AudioMode::Microphone, false);
    }
    #[test]
    #[ignore = "needs an unlocked interactive desktop and default playback and microphone devices"]
    fn records_system_and_microphone_with_video() {
        record_smoke(AudioMode::SystemAndMicrophone, true);
    }
    fn record_smoke(audio: AudioMode, pause_during_capture: bool) {
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
        let session = super::start(region, path.clone(), audio).unwrap();
        let first = session
            .events
            .recv_timeout(Duration::from_secs(15))
            .unwrap();
        assert!(
            matches!(first, Event::Started),
            "unexpected recorder event: {first:?}"
        );
        if pause_during_capture {
            std::thread::sleep(Duration::from_secs(1));
            session.pause.set_paused(true);
            std::thread::sleep(Duration::from_secs(1));
            session.pause.set_paused(false);
            std::thread::sleep(Duration::from_secs(1));
        } else {
            std::thread::sleep(Duration::from_secs(2));
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
        assert!(
            (1.0..5.0).contains(&video_seconds),
            "unexpected video duration: {video_seconds}"
        );
        if audio != AudioMode::None {
            let audio_seconds = tracks.iter().find(|(kind, _)| *kind == *b"soun").unwrap().1;
            assert!(
                (1.0..5.0).contains(&audio_seconds),
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
