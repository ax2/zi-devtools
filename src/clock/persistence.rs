use super::*;
use anyhow::{Context, Result, ensure};
use crossbeam_channel::{Receiver, TryRecvError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 524_288;
const MAX_MS: u64 = 3_153_600_000_000; // 100 years; rejects pathological files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(super) enum Policy {
    #[default]
    Pause,
    Continue,
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimerData {
    id: u64,
    name: String,
    total: u64,
    cycle: u64,
    remaining: u64,
    running: bool,
    finished: bool,
}
impl TimerData {
    fn capture(t: &Timer, now: Instant) -> Self {
        Self {
            id: t.id,
            name: t.name.clone(),
            total: millis(t.total),
            cycle: millis(t.cycle),
            remaining: millis(t.remaining(now)),
            running: t.running(),
            finished: t.finished,
        }
    }
    fn validate(&self) -> Result<()> {
        name(&self.name)?;
        ensure!(
            (1_000..=604_800_000).contains(&self.total)
                && (1_000..=604_800_000).contains(&self.cycle),
            "计时时长越界"
        );
        ensure!(self.remaining <= self.cycle, "剩余时长超过当前周期");
        ensure!(
            !self.finished || (!self.running && self.remaining == 0),
            "结束状态不一致"
        );
        Ok(())
    }
    fn restore(&self, now: Instant, elapsed: u64, policy: Policy) -> Timer {
        let remaining = if self.running && policy == Policy::Continue {
            self.remaining.saturating_sub(elapsed)
        } else {
            self.remaining
        };
        Timer {
            id: self.id,
            name: self.name.clone(),
            total: Duration::from_millis(self.total),
            cycle: Duration::from_millis(self.cycle),
            remaining: Duration::from_millis(remaining),
            started: (self.running && policy == Policy::Continue && remaining > 0).then_some(now),
            finished: self.finished || (self.running && remaining == 0),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AlarmData {
    id: u64,
    name: String,
    zone: String,
    at: DateTime<Utc>,
    daily: bool,
    late: bool,
    time: NaiveTime,
    enabled: bool,
    snooze: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeData {
    source: Source,
    title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    schema: u32,
    tool_version: String,
    saved_at: DateTime<Utc>,
    policy: Policy,
    tab: u8,
    zones: Vec<String>,
    analog: bool,
    meeting: i32,
    stopwatch: u64,
    laps: Vec<u64>,
    timers: Vec<TimerData>,
    alarms: Vec<AlarmData>,
    focus: TimerData,
    work: u64,
    short: u64,
    long: u64,
    completed: u32,
    phase: u8,
    notices: Vec<NoticeData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<audio::Settings>,
}
fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}
fn name(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= 80,
        "名称需要1–80个字符"
    );
    Ok(())
}
impl Snapshot {
    pub(super) fn capture(s: &State, now: Instant, utc: DateTime<Utc>, policy: Policy) -> Self {
        Self {
            schema: 2,
            tool_version: "0.4.0".into(),
            audio: Some(s.audio.settings.clone()),
            saved_at: utc,
            policy,
            tab: match s.tab {
                Tab::World => 0,
                Tab::Stopwatch => 1,
                Tab::Timers => 2,
                Tab::Alarms => 3,
                Tab::Focus => 4,
            },
            zones: s.zones.iter().map(|z| z.name().into()).collect(),
            analog: s.analog,
            meeting: s.meeting,
            stopwatch: millis(s.stopwatch.elapsed(now)),
            laps: s.stopwatch.laps.iter().map(|d| millis(*d)).collect(),
            timers: s
                .timers
                .iter()
                .map(|t| TimerData::capture(t, now))
                .collect(),
            alarms: s
                .alarms
                .iter()
                .map(|a| AlarmData {
                    id: a.id,
                    name: a.name.clone(),
                    zone: a.zone.name().into(),
                    at: a.at,
                    daily: a.daily,
                    late: a.late,
                    time: a.time,
                    enabled: a.enabled,
                    snooze: a.snooze,
                })
                .collect(),
            focus: TimerData::capture(&s.focus.timer, now),
            work: s.focus.work,
            short: s.focus.short,
            long: s.focus.long,
            completed: s.focus.completed,
            phase: s.focus.phase,
            notices: s
                .notices
                .iter()
                .map(|n| NoticeData {
                    source: n.source,
                    title: n.title.clone(),
                })
                .collect(),
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            [1, 2].contains(&self.schema),
            "不支持此时钟文件版本，保留原文件"
        );
        ensure!(
            match self.schema {
                1 => self.audio.is_none(),
                2 => self.audio.as_ref().is_some_and(audio::Settings::valid),
                _ => false,
            },
            "声音设置与保存格式不一致或越界"
        );
        ensure!(
            !self.tool_version.is_empty()
                && self.tool_version.len() <= 64
                && self.tab <= 4
                && (-720..=1440).contains(&self.meeting),
            "时钟设置越界"
        );
        ensure!(
            self.zones.len() <= 12
                && self.laps.len() <= 1024
                && self.timers.len() <= 32
                && self.alarms.len() <= 64
                && self.notices.len() <= 97,
            "时钟记录数量越界"
        );
        let mut zones = std::collections::HashSet::new();
        for z in &self.zones {
            ensure!(z.parse::<Tz>().is_ok() && zones.insert(z), "无效或重复时区");
        }
        ensure!(
            self.stopwatch <= MAX_MS
                && self.laps.windows(2).all(|p| p[0] <= p[1])
                && self.laps.last().is_none_or(|last| *last <= self.stopwatch),
            "秒表分段不一致"
        );
        ensure!(
            (1..=180).contains(&self.work)
                && (1..=60).contains(&self.short)
                && (1..=120).contains(&self.long)
                && self.phase <= 2
                && self.focus.id == 0,
            "专注设置越界"
        );
        self.focus.validate()?;
        let mut ids = std::collections::HashSet::new();
        for t in &self.timers {
            t.validate()?;
            ensure!(
                t.id > 0 && t.id < u64::MAX && ids.insert(t.id),
                "计时器ID无效或重复"
            );
        }
        for a in &self.alarms {
            name(&a.name)?;
            ensure!(
                a.id > 0 && a.id < u64::MAX && ids.insert(a.id) && a.zone.parse::<Tz>().is_ok(),
                "闹钟ID或时区无效"
            );
        }
        let mut sources = std::collections::HashSet::new();
        for n in &self.notices {
            ensure!(
                !n.title.is_empty()
                    && n.title.chars().count() <= 512
                    && sources.insert(format!("{:?}", n.source)),
                "提醒无效或重复"
            );
            ensure!(
                match n.source {
                    Source::Timer(id) => self.timers.iter().any(|t| t.id == id),
                    Source::Alarm(id) => self.alarms.iter().any(|a| a.id == id),
                    Source::Focus => true,
                },
                "提醒来源不存在"
            );
        }
        Ok(())
    }
    pub(super) fn restore(
        &self,
        s: &mut State,
        now: Instant,
        utc: DateTime<Utc>,
        policy: Policy,
    ) -> Result<()> {
        self.validate()?;
        s.windows.clear();
        s.audio.restore(self.audio.clone().unwrap_or_default());
        s.pending_audio = false;
        let elapsed = utc
            .signed_duration_since(self.saved_at)
            .num_milliseconds()
            .max(0) as u64;
        s.tab = match self.tab {
            1 => Tab::Stopwatch,
            2 => Tab::Timers,
            3 => Tab::Alarms,
            4 => Tab::Focus,
            _ => Tab::World,
        };
        s.zones = self
            .zones
            .iter()
            .map(|z| z.parse().expect("validated zone"))
            .collect();
        s.analog = self.analog;
        s.meeting = self.meeting;
        s.stopwatch = Stopwatch {
            accumulated: Duration::from_millis(self.stopwatch),
            started: None,
            laps: self
                .laps
                .iter()
                .map(|v| Duration::from_millis(*v))
                .collect(),
        };
        s.timers = self
            .timers
            .iter()
            .map(|t| t.restore(now, elapsed, policy))
            .collect();
        s.alarms = self
            .alarms
            .iter()
            .map(|a| Alarm {
                id: a.id,
                name: a.name.clone(),
                zone: a.zone.parse().expect("validated zone"),
                at: a.at,
                daily: a.daily,
                late: a.late,
                time: a.time,
                enabled: a.enabled,
                snooze: a.snooze,
            })
            .collect();
        s.focus = Focus {
            timer: self.focus.restore(now, elapsed, policy),
            work: self.work,
            short: self.short,
            long: self.long,
            completed: self.completed,
            phase: self.phase,
        };
        s.notices = self
            .notices
            .iter()
            .map(|n| Notice {
                source: n.source,
                title: n.title.clone(),
            })
            .collect();
        for t in &s.timers {
            if t.finished
                && !self
                    .timers
                    .iter()
                    .find(|d| d.id == t.id)
                    .expect("restored timer")
                    .finished
                && !s.notices.iter().any(|n| n.source == Source::Timer(t.id))
            {
                s.notices.push(Notice {
                    source: Source::Timer(t.id),
                    title: format!("{}：离线期间计时结束", t.name),
                });
                s.pending_audio = true;
            }
        }
        if s.focus.timer.finished
            && !self.focus.finished
            && !s.notices.iter().any(|n| n.source == Source::Focus)
        {
            s.notices.push(Notice {
                source: Source::Focus,
                title: "专注阶段在离线期间结束，请选择下一阶段".into(),
            });
            s.pending_audio = true;
        }
        s.next_id = self
            .timers
            .iter()
            .map(|t| t.id)
            .chain(self.alarms.iter().map(|a| a.id))
            .max()
            .unwrap_or(0)
            + 1;
        // Due calendar alarms are polled normally; restored acknowledged alarms are not rearmed.
        Ok(())
    }
}
type Hash = [u8; 32];
#[cfg(windows)]
struct WriteLock(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WriteLock {
    fn acquire(path: &Path) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{CreateFileW, FILE_FLAG_DELETE_ON_CLOSE, OPEN_ALWAYS},
        };
        let lock = path.with_extension("lock");
        let name: Vec<u16> = lock.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_ALWAYS,
                FILE_FLAG_DELETE_ON_CLOSE,
                std::ptr::null_mut(),
            )
        };
        ensure!(
            handle != INVALID_HANDLE_VALUE,
            "另一个进程正在保存时钟，或无法创建写锁"
        );
        Ok(Self(handle))
    }
}
#[cfg(windows)]
impl Drop for WriteLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
struct WriteLock(PathBuf, fs::File);
#[cfg(not(windows))]
impl WriteLock {
    fn acquire(path: &Path) -> Result<Self> {
        let lock = path.with_extension("lock");
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)?;
        Ok(Self(lock, file))
    }
}
#[cfg(not(windows))]
impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.1.sync_all();
        let _ = fs::remove_file(&self.0);
    }
}
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= LIMIT,
        "时钟文件不是普通文件或超过512KiB"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "时钟文件超过512KiB");
    Ok(Some(bytes))
}
fn read(path: &Path) -> Result<Option<(Snapshot, Hash)>> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    let snapshot: Snapshot =
        serde_json::from_slice(&bytes).context("无法解析时钟文件，保留原文件")?;
    snapshot.validate()?;
    Ok(Some((snapshot, Sha256::digest(&bytes).into())))
}
fn backup_schema1(path: &Path, hash: Hash) -> Result<()> {
    let bytes = read_bytes(path)?.context("旧版保存记录已不存在")?;
    let actual: Hash = Sha256::digest(&bytes).into();
    ensure!(actual == hash, "旧版记录在迁移前被修改，请重新读取");
    let digest = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let backup = path.with_file_name(format!(
        "{}.schema1-{digest}.backup.json",
        path.file_name()
            .context("时钟路径无文件名")?
            .to_string_lossy()
    ));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
    {
        Ok(mut file) => {
            let result = file.write_all(&bytes).and_then(|()| file.sync_all());
            drop(file);
            if result.is_err() {
                let _ = fs::remove_file(&backup);
            }
            result.context("无法保留旧版快照，原记录未替换")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                read_bytes(&backup)?.as_ref() == Some(&bytes),
                "已有旧版备份不一致，原记录未替换"
            );
        }
        Err(error) => return Err(error).context("无法保留旧版快照，原记录未替换"),
    }
    Ok(())
}
fn write(path: &Path, snapshot: &Snapshot, expected: Option<Hash>) -> Result<Hash> {
    snapshot.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let _lock = WriteLock::acquire(path)?;
    let current = read(path)?;
    ensure!(
        current.as_ref().map(|(_, h)| *h) == expected,
        "保存文件被其他进程修改，请重新读取并确认"
    );
    if let Some((old, hash)) = &current
        && old.schema == 1
        && snapshot.schema == 2
    {
        backup_schema1(path, *hash)?;
    }
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    ensure!(bytes.len() as u64 <= LIMIT, "时钟内容超过512KiB");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Never delete the previous file first. A failed commit leaves it readable.
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.context("时钟保存失败，旧文件与当前会话保留")?;
    Ok(Sha256::digest(&bytes).into())
}
enum Reply {
    Loaded(Result<Option<(Box<Snapshot>, Hash)>>),
    Saved(Result<(Hash, DateTime<Utc>)>),
}
pub(super) struct Storage {
    path: PathBuf,
    ctx: egui::Context,
    receiver: Option<Receiver<Reply>>,
    expected: Option<Hash>,
    loaded: bool,
    pending: Option<Snapshot>,
    enabled: bool,
    pub dirty: bool,
    last_checkpoint: Instant,
    saved: Option<DateTime<Utc>>,
    error: String,
    policy: Policy,
    confirm_replace: bool,
    #[cfg(feature = "ui-preview")]
    pub rects: [egui::Rect; 3],
}
impl Storage {
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_ready(&self, phase: u8, state: &State) -> bool {
        if self.busy() || !self.loaded {
            return false;
        }
        assert!(
            self.error.is_empty(),
            "storage preview error: {}",
            self.error
        );
        match phase {
            0 => {
                assert!(self.pending.is_none());
                true
            }
            1 => {
                if !self.enabled || self.saved.is_none() || self.dirty {
                    return false;
                }
                let data = read(&self.path).unwrap().unwrap().0;
                assert_eq!(data.timers.len(), 2);
                assert!(data.timers[0].running);
                assert!(!data.timers[1].running);
                assert_eq!(data.stopwatch, 1234);
                assert_eq!(data.laps, [500]);
                println!(
                    "PASS clock native save: explicit enable, async atomic write, running/paused timers and stopwatch snapshot"
                );
                true
            }
            2 => {
                assert!(self.pending.is_some());
                assert!(!self.enabled);
                assert!(state.timers.is_empty());
                true
            }
            3 => {
                if self.pending.is_some() || !self.enabled || self.dirty {
                    return false;
                }
                assert_eq!(state.timers.len(), 2);
                assert!(!state.timers[0].running());
                assert!(!state.timers[1].running());
                assert_eq!(
                    state.timers[1].remaining(Instant::now()),
                    Duration::from_secs(300)
                );
                assert_eq!(
                    state.stopwatch.elapsed(Instant::now()),
                    Duration::from_millis(1234)
                );
                assert_eq!(state.stopwatch.laps, [Duration::from_millis(500)]);
                println!(
                    "PASS clock native restart: load preview without mutation, explicit restore, paused timers and stopwatch, re-save"
                );
                true
            }
            _ => panic!("unknown storage preview phase"),
        }
    }
    pub fn new(path: PathBuf, ctx: egui::Context) -> Self {
        let mut s = Self {
            path,
            ctx,
            receiver: None,
            expected: None,
            loaded: false,
            pending: None,
            enabled: false,
            dirty: false,
            last_checkpoint: Instant::now(),
            saved: None,
            error: String::new(),
            policy: Policy::Pause,
            confirm_replace: false,
            #[cfg(feature = "ui-preview")]
            rects: [egui::Rect::NOTHING; 3],
        };
        s.load();
        s
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub(super) fn has_work(&self) -> bool {
        self.busy() || (self.enabled && self.dirty)
    }
    fn load(&mut self) {
        if self.busy() {
            return;
        }
        self.enabled = false;
        self.loaded = false;
        self.pending = None;
        self.error.clear();
        self.confirm_replace = false;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.receiver = Some(rx);
        let path = self.path.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let reply = Reply::Loaded(
                read(&path).map(|data| data.map(|(snapshot, hash)| (Box::new(snapshot), hash))),
            );
            let _ = tx.send(reply);
            ctx.request_repaint();
        });
    }
    fn save(&mut self, s: &State, now: Instant, utc: DateTime<Utc>) {
        if self.busy() || !self.loaded || self.pending.is_some() {
            return;
        }
        let snapshot = Snapshot::capture(s, now, utc, self.policy);
        let path = self.path.clone();
        let expected = self.expected;
        let ctx = self.ctx.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.receiver = Some(rx);
        self.last_checkpoint = now;
        self.dirty = false;
        self.error.clear();
        std::thread::spawn(move || {
            let result = write(&path, &snapshot, expected).map(|h| (h, utc));
            let _ = tx.send(Reply::Saved(result));
            ctx.request_repaint();
        });
    }
    pub(super) fn tick(&mut self, s: &State, now: Instant, utc: DateTime<Utc>) {
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(reply) => {
                    self.receiver = None;
                    match reply {
                        Reply::Loaded(Ok(data)) => {
                            self.loaded = true;
                            if let Some((snapshot, hash)) = data {
                                self.expected = Some(hash);
                                self.saved = Some(snapshot.saved_at);
                                self.policy = snapshot.policy;
                                self.pending = Some(*snapshot);
                            } else {
                                self.expected = None;
                                self.saved = None;
                            }
                        }
                        Reply::Saved(Ok((hash, at))) => {
                            self.expected = Some(hash);
                            self.saved = Some(at);
                        }
                        Reply::Loaded(Err(e)) | Reply::Saved(Err(e)) => {
                            self.error = format!("{e:#}");
                            self.dirty = true;
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.receiver = None;
                    self.error = "时钟后台读写中断，当前会话保留".into();
                    self.dirty = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.enabled
            && self.error.is_empty()
            && !self.busy()
            && (self.dirty
                || (s.needs_clock()
                    && now.saturating_duration_since(self.last_checkpoint)
                        >= Duration::from_secs(30)))
        {
            self.save(s, now, utc);
        }
        if self.busy() {
            self.ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub(super) fn ui(&mut self, s: &mut State, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if self.pending.is_none() {
                let enable = ui.add_enabled(
                    self.loaded && !self.busy() && self.error.is_empty(),
                    egui::Button::new(if self.enabled {
                        "停止自动保存"
                    } else {
                        "启用本机保存"
                    }),
                );
                #[cfg(feature = "ui-preview")]
                {
                    self.rects[0] = enable.rect;
                }
                if enable.clicked() {
                    self.enabled = !self.enabled;
                    if self.enabled {
                        self.dirty = true;
                    }
                }
                let save = ui.add_enabled(
                    self.enabled && !self.busy(),
                    egui::Button::new("立即保存 / 重试"),
                );
                #[cfg(feature = "ui-preview")]
                {
                    self.rects[2] = save.rect;
                }
                if save.clicked() {
                    self.save(s, Instant::now(), Utc::now());
                }
            } else {
                ui.label("已有保存记录，等待确认恢复");
            }
            if self.busy() {
                ui.small("后台读写中");
            } else if self.enabled && !self.dirty && self.error.is_empty() {
                ui.small("已保存 · 运行每30秒检查点");
            } else if self.enabled {
                ui.small("修改待保存");
            } else if self.pending.is_none() {
                ui.small("当前仅运行会话；停止保存会保留已有文件");
            }
        });
        if !self.error.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &self.error);
        }
        egui::CollapsingHeader::new("保存设置 / 读取与恢复预览").default_open(false).open(self.pending.is_some().then_some(true)).show(ui,|ui| {
            ui.small("主动开启后保存到下列文件：操作后自动保存，运行计时每30秒检查点。故障退出可能丢失上次成功保存后的进度；完全退出后不弹提醒。");
            ui.label(self.path.display().to_string());
            let mut policy=self.policy;
            ui.horizontal_wrapped(|ui| {ui.label("重启倒计时：");ui.selectable_value(&mut policy,Policy::Pause,"按剩余时间暂停恢复");ui.selectable_value(&mut policy,Policy::Continue,"按原到期时间继续（含退出期间）");});
            if policy!=self.policy {self.policy=policy;self.dirty=true;}
            ui.small("秒表始终暂停恢复；闹钟按日历时间补一次提醒。系统时间回拨时，继续模式不会增加保存的剩余时长。");
            if let Some(snapshot)=self.pending.clone() {
                ui.label(format!("待恢复：{} · {}计时器 / {}闹钟 / {}分段 · {}提醒",snapshot.saved_at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S"),snapshot.timers.len(),snapshot.alarms.len(),snapshot.laps.len(),snapshot.notices.len()));
                ui.small("恢复会替换当前时钟会话并关闭已有时钟小窗，请先确认；不会修改备忘、日程或系统时间。");
                if snapshot.schema == 1 {ui.small("确认后保存将升级格式，并在同目录保留旧文件的校验备份；旧版程序需要从该备份恢复。");}
                let restore=ui.add_enabled(!self.busy(),egui::Button::new("恢复此记录并启用保存"));
                #[cfg(feature="ui-preview")]{self.rects[1]=restore.rect;}
                if restore.clicked() {
                    match snapshot.restore(s,Instant::now(),Utc::now(),self.policy) {
                        Ok(())=>{self.pending=None;self.enabled=true;self.dirty=true;},Err(e)=>self.error=format!("{e:#}")
                    }
                }
                ui.checkbox(&mut self.confirm_replace,"确认用当前时钟会话覆盖已保存记录");
                if ui.add_enabled(!self.busy() && self.confirm_replace,egui::Button::new("保留当前会话并替换保存记录")).clicked(){self.pending=None;self.enabled=true;self.dirty=true;self.confirm_replace=false;}
            }
            if ui.add_enabled(!self.busy(),egui::Button::new("重新读取保存记录（先预览，不恢复）")).clicked(){self.load();}
            if self.busy(){ui.label("后台读写中，请等待完成后退出。");}
            if let Some(at)=self.saved{ui.small(format!("最近保存：{}",at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")));}
            if self.enabled && !self.dirty && !self.busy() && self.error.is_empty(){ui.small("自动保存已开启；当前修改已保存。运行计时每30秒保存检查点。");}
        });
    }
}
