use chrono::{DateTime, Local, Timelike, Utc};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const RATE: usize = 48_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(super) enum Tone {
    #[default]
    Chime,
    Double,
    Soft,
}
impl Tone {
    fn label(self) -> &'static str {
        match self {
            Self::Chime => "清脆提示",
            Self::Double => "双音提醒",
            Self::Soft => "柔和提示",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    pub enabled: bool,
    pub muted: bool,
    pub volume: u8,
    pub tone: Tone,
    pub quiet: bool,
    pub quiet_start: u16,
    pub quiet_end: u16,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            muted: false,
            volume: 50,
            tone: Tone::Chime,
            quiet: false,
            quiet_start: 1320,
            quiet_end: 420,
        }
    }
}
impl Settings {
    pub fn valid(&self) -> bool {
        self.volume <= 100 && self.quiet_start < 1440 && self.quiet_end < 1440
    }
    fn permitted(&self, minute: u16, preview: bool) -> bool {
        if !self.valid() || self.muted || self.volume == 0 || (!preview && !self.enabled) {
            return false;
        }
        if preview || !self.quiet {
            return true;
        }
        let quiet = if self.quiet_start == self.quiet_end {
            true
        } else if self.quiet_start < self.quiet_end {
            (self.quiet_start..self.quiet_end).contains(&minute)
        } else {
            minute >= self.quiet_start || minute < self.quiet_end
        };
        !quiet
    }
}
// Bounded stereo f32 frames. Each note fades in/out to avoid abrupt clicks.
fn samples(settings: &Settings) -> Vec<u8> {
    let notes: &[(f64, usize, usize)] = match settings.tone {
        Tone::Chime => &[(880.0, 0, 300), (1320.0, 330, 450)],
        Tone::Double => &[(660.0, 0, 250), (660.0, 400, 250)],
        Tone::Soft => &[(440.0, 0, 700)],
    };
    let duration = notes
        .iter()
        .map(|(_, start, length)| start + length)
        .max()
        .unwrap_or(0);
    let frames = duration * RATE / 1000;
    let mut bytes = Vec::with_capacity(frames * 8);
    let gain = f64::from(settings.volume.min(100)) / 100.0 * 0.25;
    for frame in 0..frames {
        let ms = frame as f64 * 1000.0 / RATE as f64;
        let mut value = 0.0;
        for (frequency, start, length) in notes {
            let elapsed = ms - *start as f64;
            if elapsed >= 0.0 && elapsed < *length as f64 {
                let envelope =
                    (elapsed / 12.0).min(1.0) * ((*length as f64 - elapsed) / 80.0).min(1.0);
                value +=
                    (elapsed / 1000.0 * frequency * std::f64::consts::TAU).sin() * envelope * gain;
            }
        }
        let sample = (value.clamp(-0.25, 0.25) as f32).to_le_bytes();
        bytes.extend_from_slice(&sample);
        bytes.extend_from_slice(&sample);
    }
    bytes
}
type Backend = fn(&[u8], &AtomicBool) -> Result<(), String>;
#[cfg(windows)]
fn play(bytes: &[u8], cancel: &AtomicBool) -> Result<(), String> {
    use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
    let operation = (|| -> anyhow::Result<()> {
        wasapi::initialize_mta().ok()?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                wasapi::deinitialize();
            }
        }
        let _com = Com;
        let device = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
        let mut client = device.get_iaudioclient()?;
        let format = WaveFormat::new(32, 32, &SampleType::Float, RATE, 2, None);
        let (period, _) = client.get_device_period()?;
        client.initialize_client(
            &format,
            &Direction::Render,
            &StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
        )?;
        let render = client.get_audiorenderclient()?;
        let capacity = client.get_buffer_size()? as usize;
        anyhow::ensure!(capacity > 0 && capacity <= RATE * 2, "invalid audio buffer");
        let total = bytes.len() / 8;
        let initial = total.min(capacity);
        render.write_to_device(initial, &bytes[..initial * 8], None)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        client.start_stream()?;
        struct Stream(AudioClient);
        impl Drop for Stream {
            fn drop(&mut self) {
                let _ = self.0.stop_stream();
            }
        }
        let stream = Stream(client);
        let mut position = initial;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "audio output stalled");
            if position == total && stream.0.get_current_padding()? == 0 {
                break;
            }
            let available = stream.0.get_available_space_in_frames()? as usize;
            let count = available.min(total.saturating_sub(position));
            if count > 0 {
                render.write_to_device(
                    count,
                    &bytes[position * 8..(position + count) * 8],
                    None,
                )?;
                position += count;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    operation.map_err(|_| "声音播放失败；请检查默认输出设备。提醒仍保留，可重新试听。".into())
}
#[cfg(not(windows))]
fn play(_: &[u8], _: &AtomicBool) -> Result<(), String> {
    Err("当前声音后端仅支持Windows；应用内提醒仍保留".into())
}

pub(super) struct State {
    pub settings: Settings,
    ctx: egui::Context,
    receiver: Option<crossbeam_channel::Receiver<Result<(), String>>>,
    cancel: Arc<AtomicBool>,
    backend: Backend,
    pub error: String,
    status: &'static str,
    attempts: u64,
    #[cfg(feature = "ui-preview")]
    pub rects: [egui::Rect; 4],
    #[cfg(feature = "ui-preview")]
    pub preview_open: bool,
}
impl Default for State {
    fn default() -> Self {
        Self::new(egui::Context::default())
    }
}
impl State {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            settings: Settings::default(),
            ctx,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            backend: play,
            error: String::new(),
            status: "尚未播放",
            attempts: 0,
            #[cfg(feature = "ui-preview")]
            rects: [egui::Rect::NOTHING; 4],
            #[cfg(feature = "ui-preview")]
            preview_open: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn tick(&mut self) {
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.receiver = None;
                    if self.cancel.load(Ordering::Acquire) {
                        self.status = "播放已停止";
                    } else {
                        match result {
                            Ok(()) => self.status = "短音播放已结束",
                            Err(error) => {
                                self.error = error;
                                self.status = "声音失败，提醒保留";
                            }
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.error = "声音后台任务中断，提醒仍保留".into();
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if self.busy() {
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn minute(utc: DateTime<Utc>) -> u16 {
        let local = utc.with_timezone(&Local);
        (local.hour() * 60 + local.minute()) as u16
    }
    pub fn alert(&mut self, utc: DateTime<Utc>, preview: bool) -> bool {
        if !self.settings.permitted(Self::minute(utc), preview) {
            self.status = "当前声音关闭、静音或处于免打扰";
            return false;
        }
        if self.busy() {
            self.status = "已有短音播放，本次声音已合并";
            return false;
        }
        self.error.clear();
        self.status = "正在播放短音";
        self.attempts = self.attempts.saturating_add(1);
        let bytes = samples(&self.settings);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let backend = self.backend;
        let ctx = self.ctx.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.receiver = Some(rx);
        std::thread::spawn(move || {
            let result = backend(&bytes, &cancel);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        true
    }
    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn restore(&mut self, settings: Settings) {
        self.stop();
        self.settings = settings;
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.ctx = ui.ctx().clone();
        let before = self.settings.clone();
        let summary = if self.settings.muted || self.settings.volume == 0 {
            "静音"
        } else if !self.settings.enabled {
            "关闭"
        } else if !self.settings.permitted(Self::minute(Utc::now()), false) {
            "免打扰"
        } else {
            "开启"
        };
        let header = egui::CollapsingHeader::new(format!("声音：{summary} · 设置与试听"))
            .id_salt("clock-audio-settings")
            .default_open(false);
        #[cfg(feature = "ui-preview")]
        let header = header.open(self.preview_open.then_some(true));
        header.show(ui,|ui| {
            ui.horizontal_wrapped(|ui| {
                let _enable=ui.checkbox(&mut self.settings.enabled,"开启声音提醒");
                #[cfg(feature="ui-preview")]{self.rects[0]=_enable.rect;}
                let _mute=ui.checkbox(&mut self.settings.muted,"静音");
                #[cfg(feature="ui-preview")]{self.rects[3]=_mute.rect;}
                ui.add(egui::Slider::new(&mut self.settings.volume,0..=100).text("音量"));
                egui::ComboBox::from_id_salt("clock-audio-tone").selected_text(self.settings.tone.label()).show_ui(ui,|ui|{for tone in [Tone::Chime,Tone::Double,Tone::Soft]{ui.selectable_value(&mut self.settings.tone,tone,tone.label());}});
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.settings.quiet,"定时免打扰（本机当地时间）");
                for (label,minute) in [("开始",&mut self.settings.quiet_start),("结束",&mut self.settings.quiet_end)] {
                    ui.label(label);ui.add(egui::DragValue::new(minute).range(0..=1439).speed(15).custom_formatter(|v,_|format!("{:02}:{:02}",v as u16/60,v as u16%60)).custom_parser(parse_minute));
                }
            });
            ui.small("开始=结束表示全天免打扰；静音和音量0也禁止试听。提醒不因静音而丢失，不更改系统音量。");
            ui.horizontal_wrapped(|ui| {
                let preview=ui.add_enabled(!self.busy() && !self.settings.muted && self.settings.volume>0,egui::Button::new("试听（忽略定时免打扰）"));
                #[cfg(feature="ui-preview")]{self.rects[1]=preview.rect;}
                if preview.clicked(){self.alert(Utc::now(),true);}
                let stop=ui.add_enabled(self.busy(),egui::Button::new("停止播放"));
                #[cfg(feature="ui-preview")]{self.rects[2]=stop.rect;}
                if stop.clicked(){self.stop();}
                ui.small(self.status);
            });
            ui.small("三种内置短音，每次不到1秒。多项同时到期只播一次，播放中再到期合并声音，全部提醒仍可逐项查看。");
        });
        if before != self.settings && !self.settings.permitted(Self::minute(Utc::now()), false) {
            self.stop();
        }
        if !self.error.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.error);
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fake(&mut self) {
        self.backend = |_, _| Ok(());
        self.preview_open = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fail(&mut self) {
        self.backend = |_, _| Err("合成音频失败；提醒仍保留".into());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_attempts(&self) -> u64 {
        self.attempts
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.stop();
    }
}
fn parse_minute(text: &str) -> Option<f64> {
    let (h, m) = text.trim().split_once(':')?;
    let h = h.parse::<u16>().ok()?;
    let m = m.parse::<u16>().ok()?;
    (h < 24 && m < 60).then_some(f64::from(h * 60 + m))
}

#[cfg(feature = "ui-preview")]
pub(super) fn verify_device() -> anyhow::Result<()> {
    let settings = Settings {
        volume: 12,
        ..Settings::default()
    };
    play(&samples(&settings), &AtomicBool::new(false)).map_err(anyhow::Error::msg)
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
