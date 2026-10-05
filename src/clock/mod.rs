mod audio;
#[cfg(feature = "ui-preview")]
pub fn verify_audio_device() -> anyhow::Result<()> {
    audio::verify_device()
}
mod persistence;
mod ui;
use chrono::{DateTime, Days, LocalResult, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use eframe::egui;
use std::time::{Duration, Instant};
#[derive(Default)]
pub struct Stopwatch {
    accumulated: Duration,
    started: Option<Instant>,
    pub laps: Vec<Duration>,
}
impl Stopwatch {
    pub fn elapsed(&self, now: Instant) -> Duration {
        self.accumulated.saturating_add(
            self.started
                .map(|s| now.saturating_duration_since(s))
                .unwrap_or_default(),
        )
    }
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn toggle(&mut self, now: Instant) {
        if self.running() {
            self.accumulated = self.elapsed(now);
            self.started = None;
        } else {
            self.started = Some(now);
        }
    }
    pub fn lap(&mut self, now: Instant) {
        if self.running() && self.laps.len() < 1024 {
            self.laps.push(self.elapsed(now));
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
pub struct Timer {
    pub id: u64,
    pub name: String,
    pub total: Duration,
    pub cycle: Duration,
    remaining: Duration,
    started: Option<Instant>,
    pub finished: bool,
}
impl Timer {
    pub fn new(id: u64, name: String, seconds: u64) -> Self {
        let duration = Duration::from_secs(seconds.clamp(1, 604800));
        Self {
            id,
            name,
            total: duration,
            cycle: duration,
            remaining: duration,
            started: None,
            finished: false,
        }
    }
    pub fn remaining(&self, now: Instant) -> Duration {
        self.remaining.saturating_sub(
            self.started
                .map(|s| now.saturating_duration_since(s))
                .unwrap_or_default(),
        )
    }
    pub fn running(&self) -> bool {
        self.started.is_some()
    }
    pub fn toggle(&mut self, now: Instant) {
        if self.finished {
            return;
        }
        if self.running() {
            self.remaining = self.remaining(now);
            self.started = None;
        } else {
            self.started = Some(now);
        }
    }
    pub fn restart(&mut self) {
        self.remaining = self.total;
        self.cycle = self.total;
        self.started = None;
        self.finished = false;
    }
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.running() && self.remaining(now).is_zero() {
            self.remaining = Duration::ZERO;
            self.started = None;
            self.finished = true;
            true
        } else {
            false
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    Timer(u64),
    Alarm(u64),
    Focus,
}
pub struct Notice {
    pub source: Source,
    pub title: String,
}
pub struct Alarm {
    pub id: u64,
    pub name: String,
    pub zone: Tz,
    pub at: DateTime<Utc>,
    pub daily: bool,
    pub late: bool,
    pub time: NaiveTime,
    pub enabled: bool,
    pub snooze: Option<DateTime<Utc>>,
}
pub fn resolve_local(zone: Tz, local: NaiveDateTime, late: bool) -> Result<DateTime<Utc>, String> {
    match zone.from_local_datetime(&local) {
        LocalResult::None => Err("此时间在夏令时转换中不存在，请选择其他时间".into()),
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(a, b) => {
            Ok(if late { a.max(b) } else { a.min(b) }.with_timezone(&Utc))
        }
    }
}
fn next_daily(now: DateTime<Utc>, zone: Tz, time: NaiveTime, late: bool) -> Option<DateTime<Utc>> {
    let mut date = now.with_timezone(&zone).date_naive();
    for _ in 0..370 {
        if let Ok(at) = resolve_local(zone, date.and_time(time), late)
            && at > now
        {
            return Some(at);
        }
        date = date.checked_add_days(Days::new(1))?;
    }
    None
}
pub struct Focus {
    pub timer: Timer,
    pub work: u64,
    pub short: u64,
    pub long: u64,
    pub completed: u32,
    pub phase: u8,
}
impl Default for Focus {
    fn default() -> Self {
        Self {
            timer: Timer::new(0, "专注".into(), 1500),
            work: 25,
            short: 5,
            long: 15,
            completed: 0,
            phase: 0,
        }
    }
}
impl Focus {
    pub fn label(&self) -> &str {
        match self.phase {
            0 => "专注",
            1 => "短休息",
            _ => "长休息",
        }
    }
    pub fn next(&mut self) {
        if !self.timer.finished {
            return;
        }
        if self.phase == 0 {
            self.completed = self.completed.saturating_add(1);
            self.phase = if self.completed.is_multiple_of(4) {
                2
            } else {
                1
            };
        } else {
            self.phase = 0;
        }
        let minutes = match self.phase {
            0 => self.work,
            1 => self.short,
            _ => self.long,
        };
        self.timer = Timer::new(0, self.label().into(), minutes.saturating_mul(60));
    }
    pub fn reset(&mut self) {
        self.completed = 0;
        self.phase = 0;
        self.timer = Timer::new(0, "专注".into(), self.work.saturating_mul(60));
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    World,
    Stopwatch,
    Timers,
    Alarms,
    Focus,
}
pub struct State {
    audio: audio::State,
    pending_audio: bool,
    storage: Option<persistence::Storage>,
    pub tab: Tab,
    pub zones: Vec<Tz>,
    pub zone_query: String,
    pub analog: bool,
    pub meeting: i32,
    pub stopwatch: Stopwatch,
    pub timers: Vec<Timer>,
    pub alarms: Vec<Alarm>,
    pub notices: Vec<Notice>,
    pub focus: Focus,
    pub timer_name: String,
    pub timer_seconds: u64,
    pub alarm_name: String,
    pub alarm_text: String,
    pub alarm_zone: Tz,
    pub daily: bool,
    pub late: bool,
    pub message: String,
    next_id: u64,
    #[cfg(feature = "ui-preview")]
    pub rects: [egui::Rect; 4],
}
impl Default for State {
    fn default() -> Self {
        let zone = chrono_tz::Asia::Shanghai;
        Self {
            audio: audio::State::default(),
            pending_audio: false,
            storage: None,
            tab: Tab::World,
            zones: vec![
                zone,
                chrono_tz::Europe::London,
                chrono_tz::America::New_York,
                chrono_tz::Asia::Tokyo,
            ],
            zone_query: String::new(),
            analog: false,
            meeting: 0,
            stopwatch: Stopwatch::default(),
            timers: Vec::new(),
            alarms: Vec::new(),
            notices: Vec::new(),
            focus: Focus::default(),
            timer_name: "倒计时".into(),
            timer_seconds: 300,
            alarm_name: "闹钟".into(),
            alarm_text: (Utc::now() + chrono::Duration::minutes(5))
                .with_timezone(&zone)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            alarm_zone: zone,
            daily: false,
            late: false,
            message: String::new(),
            next_id: 1,
            #[cfg(feature = "ui-preview")]
            rects: [egui::Rect::NOTHING; 4],
        }
    }
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_audio_prepare(&mut self) {
        self.audio.preview_fake();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_audio_position(&self, index: usize) -> egui::Pos2 {
        self.audio.rects[index].center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_audio_check(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                assert!(self.audio.settings.enabled);
            }
            1 => {
                if self.audio.busy() {
                    return false;
                }
                assert_eq!(self.audio.preview_attempts(), 1);
                assert!(self.audio.error.is_empty());
            }
            2 => {
                assert!(self.audio.settings.muted);
                self.timer_seconds = 1;
                self.add_timer().unwrap();
                self.add_timer().unwrap();
                let now = Instant::now();
                for t in &mut self.timers {
                    t.toggle(now);
                }
                assert!(self.poll(now + Duration::from_secs(2), Utc::now()));
                assert_eq!(self.notices.len(), 2);
                assert_eq!(self.audio.preview_attempts(), 1);
            }
            3 => {
                assert!(!self.audio.settings.muted);
                self.audio.preview_fail();
                let now = Instant::now();
                self.add_timer().unwrap();
                self.timers[2].toggle(now);
                assert!(self.poll(now + Duration::from_secs(2), Utc::now()));
            }
            4 => {
                if self.audio.busy() {
                    return false;
                }
                assert_eq!(self.notices.len(), 3);
                assert_eq!(self.audio.preview_attempts(), 2);
                assert!(!self.audio.error.is_empty());
                assert!(!self.poll(Instant::now(), Utc::now()));
                assert_eq!(self.audio.preview_attempts(), 2);
                println!(
                    "PASS clock audio UI: enable/preview/mute/unmute, muted simultaneous reminders retained, device failure retained and no retry"
                );
            }
            _ => panic!("unknown audio preview phase"),
        }
        true
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_storage_prepare(&mut self, saving: bool) {
        let storage = self.storage.take();
        *self = Self::default();
        self.storage = storage;
        if saving {
            self.tab = Tab::Timers;
            self.stopwatch.accumulated = Duration::from_millis(1234);
            self.stopwatch.laps = vec![Duration::from_millis(500)];
            self.timer_seconds = 600;
            self.add_timer().unwrap();
            self.timers[0].toggle(Instant::now());
            self.timer_seconds = 300;
            self.add_timer().unwrap();
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_storage_ready(&self, phase: u8) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|s| s.preview_ready(phase, self))
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_storage_position(&self, index: usize) -> egui::Pos2 {
        self.storage.as_ref().unwrap().rects[index].center()
    }
    pub fn new(path: std::path::PathBuf, ctx: egui::Context) -> Self {
        Self {
            audio: audio::State::new(ctx.clone()),
            storage: Some(persistence::Storage::new(path, ctx)),
            ..Self::default()
        }
    }
    pub fn saving(&self) -> bool {
        self.storage.as_ref().is_some_and(|s| s.busy())
    }
    pub fn persistence_tick(&mut self, now: Instant, utc: DateTime<Utc>) {
        if let Some(mut storage) = self.storage.take() {
            storage.tick(self, now, utc);
            self.storage = Some(storage);
        }
    }
    fn changed(&mut self) {
        if let Some(storage) = &mut self.storage {
            storage.dirty = true;
        }
    }
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn add_timer(&mut self) -> Result<(), String> {
        if !(1..=604800).contains(&self.timer_seconds) {
            return Err("计时长度需要1秒至7天".into());
        }
        if self.timer_name.trim().is_empty() || self.timer_name.chars().count() > 80 {
            return Err("名称需要1–80个字符".into());
        }
        if self.timers.len() >= 32 {
            return Err("最多32个独立计时器".into());
        }
        let id = self.id();
        self.timers
            .push(Timer::new(id, self.timer_name.clone(), self.timer_seconds));
        Ok(())
    }
    pub fn add_alarm(&mut self, now: DateTime<Utc>) -> Result<(), String> {
        if self.alarm_name.trim().is_empty() || self.alarm_name.chars().count() > 80 {
            return Err("名称需要1–80个字符".into());
        }
        if self.alarms.len() >= 64 {
            return Err("最多64个闹钟".into());
        }
        let local = NaiveDateTime::parse_from_str(&self.alarm_text, "%Y-%m-%d %H:%M")
            .map_err(|_| "使用 YYYY-MM-DD HH:MM 格式")?;
        let at = resolve_local(self.alarm_zone, local, self.late)?;
        if at <= now {
            return Err("请设置未来的时间".into());
        }
        let id = self.id();
        self.alarms.push(Alarm {
            id,
            name: self.alarm_name.clone(),
            zone: self.alarm_zone,
            at,
            daily: self.daily,
            late: self.late,
            time: local.time(),
            enabled: true,
            snooze: None,
        });
        Ok(())
    }
    pub fn needs_clock(&self) -> bool {
        self.audio.busy()
            || self.stopwatch.running()
            || self.timers.iter().any(Timer::running)
            || self.alarms.iter().any(|a| a.enabled || a.snooze.is_some())
            || self.focus.timer.running()
    }
    pub fn has_work(&self) -> bool {
        self.audio.settings != audio::Settings::default()
            || self.storage.as_ref().is_some_and(|s| s.has_work())
            || !self.timers.is_empty()
            || !self.alarms.is_empty()
            || !self.notices.is_empty()
            || self.stopwatch.elapsed(Instant::now()) > Duration::ZERO
            || self.focus.completed > 0
            || self.focus.timer.running()
            || self.focus.timer.remaining(Instant::now()) < self.focus.timer.total
    }
    fn notify(&mut self, source: Source, title: String) {
        self.notices.retain(|n| n.source != source);
        self.notices.push(Notice { source, title });
    }
    pub fn poll(&mut self, now: Instant, utc: DateTime<Utc>) -> bool {
        self.audio.tick();
        let mut fresh = Vec::new();
        for timer in &mut self.timers {
            if timer.poll(now) {
                fresh.push((Source::Timer(timer.id), format!("{}：计时结束", timer.name)));
            }
        }
        for alarm in &mut self.alarms {
            let snoozed = alarm.snooze.is_some_and(|at| utc >= at);
            let due = alarm.enabled && utc >= alarm.at;
            if snoozed || due {
                fresh.push((
                    Source::Alarm(alarm.id),
                    format!("{} · {}", alarm.name, alarm.zone),
                ));
                if snoozed {
                    alarm.snooze = None;
                }
                if due {
                    if alarm.daily {
                        if let Some(at) = next_daily(utc, alarm.zone, alarm.time, alarm.late) {
                            alarm.at = at;
                        } else {
                            alarm.enabled = false;
                        }
                    } else {
                        alarm.enabled = false;
                    }
                }
            }
        }
        if self.focus.timer.poll(now) {
            fresh.push((
                Source::Focus,
                format!("{}结束，请选择下一阶段", self.focus.label()),
            ));
        }
        let changed = !fresh.is_empty();
        for (source, title) in fresh {
            self.notify(source, title);
        }
        if changed {
            self.changed();
        }
        if changed || self.pending_audio {
            self.pending_audio = false;
            self.audio.alert(utc, false);
        }
        changed
    }
    pub fn snooze(&mut self, source: Source, utc: DateTime<Utc>) {
        match source {
            Source::Alarm(id) => {
                if let Some(a) = self.alarms.iter_mut().find(|a| a.id == id) {
                    a.snooze = Some(utc + chrono::Duration::minutes(5));
                }
            }
            Source::Timer(id) => {
                if let Some(t) = self.timers.iter_mut().find(|t| t.id == id) {
                    t.remaining = Duration::from_secs(300);
                    t.cycle = t.remaining;
                    t.started = Some(Instant::now());
                    t.finished = false;
                }
            }
            Source::Focus => return,
        }
        self.notices.retain(|n| n.source != source);
    }
}
pub fn countdown_text(duration: Duration) -> String {
    duration_text(Duration::from_secs(
        duration
            .as_secs()
            .saturating_add(u64::from(duration.subsec_nanos() > 0)),
    ))
}
pub fn duration_text(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_fixture(&mut self, index: usize) {
        let storage = self.storage.take();
        *self = Self::default();
        self.storage = storage;
        self.tab = match index {
            1 => Tab::Stopwatch,
            2 | 6 => Tab::Timers,
            3 | 7 => Tab::Alarms,
            4 => Tab::Focus,
            _ => Tab::World,
        };
        if index >= 8 {
            self.audio.preview_fake();
            if index >= 9 {
                self.audio.settings.enabled = true;
                self.audio.settings.quiet = true;
                self.audio.settings.quiet_start = 0;
                self.audio.settings.quiet_end = 0;
                self.audio.settings.volume = 23;
                self.audio.settings.tone = audio::Tone::Soft;
            }
            if index == 10 {
                self.audio.error = "合成设备失败示例：声音播放失败，应用内提醒仍保留。".into();
            }
        }
        match self.tab {
            Tab::Stopwatch => {
                self.stopwatch.accumulated = Duration::from_millis(125432);
                self.stopwatch.laps =
                    vec![Duration::from_millis(60111), Duration::from_millis(90222)];
            }
            Tab::Timers => {
                self.timer_name = "泡茶".into();
                self.add_timer().unwrap();
                self.timers[0].remaining = Duration::from_secs(270);
                self.timer_name = "烤箱".into();
                self.timer_seconds = 1800;
                self.add_timer().unwrap();
                self.timers[1].remaining = Duration::from_secs(1450);
            }
            Tab::Alarms => {
                self.alarm_text = (Utc::now() + chrono::Duration::days(1))
                    .with_timezone(&self.alarm_zone)
                    .format("%Y-%m-%d 09:00")
                    .to_string();
                self.alarm_name = "阅读提醒".into();
                self.daily = true;
                self.add_alarm(Utc::now()).unwrap();
            }
            Tab::Focus => {
                self.focus.completed = 2;
                self.focus.timer.remaining = Duration::from_secs(1080);
            }
            _ => {}
        }
    }
    pub fn preview_done(&mut self) -> bool {
        if !self.timers.first().is_some_and(|t| t.finished) {
            return false;
        }
        self.preview_check(4);
        true
    }
    pub fn preview_check(&mut self, phase: u8) {
        match phase {
            0 => {
                self.tab = Tab::Stopwatch;
                self.stopwatch.reset();
            }
            1 => {
                assert!(!self.stopwatch.running());
                assert_eq!(self.stopwatch.laps.len(), 1);
                assert!(self.stopwatch.elapsed(Instant::now()) > Duration::ZERO);
                self.tab = Tab::Timers;
                self.timer_seconds = 1;
                self.timer_name = "合成计时器".into();
            }
            2 => {
                assert_eq!(self.timers.len(), 1);
                assert!(!self.timers[0].running());
            }
            3 => {
                assert!(self.timers[0].running());
                assert_eq!(self.timers[0].total.as_secs(), 1);
            }
            4 => {
                assert!(self.timers[0].finished);
                assert_eq!(self.notices.len(), 1);
                assert!(!self.poll(Instant::now(), Utc::now()));
                println!(
                    "PASS clock native: stopwatch start/lap/pause, timer create/start/finish and single reminder"
                );
            }
            _ => panic!("unknown phase"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_rounds_up_without_showing_zero_before_expiry() {
        assert_eq!(countdown_text(Duration::from_nanos(1)), "00:00:01");
        assert_eq!(
            countdown_text(Duration::from_secs(1) + Duration::from_nanos(1)),
            "00:00:02"
        );
        assert_eq!(countdown_text(Duration::ZERO), "00:00:00");
    }
    #[test]
    fn parallel_timers_bounds_pause_exit_protection_and_focus_completion_gate() {
        let start = Instant::now();
        let mut s = State::default();
        assert!(!s.has_work());
        s.timer_seconds = 0;
        assert!(s.add_timer().is_err());
        assert!(s.timers.is_empty());
        s.timer_seconds = 1;
        s.add_timer().unwrap();
        s.timer_seconds = 5;
        s.add_timer().unwrap();
        s.timers[0].toggle(start);
        s.timers[1].toggle(start);
        assert!(s.poll(start + Duration::from_secs(2), Utc::now()));
        assert!(s.timers[0].finished);
        assert!(s.timers[1].running());
        assert_eq!(s.notices.len(), 1);
        assert!(s.poll(start + Duration::from_secs(5), Utc::now()));
        assert_eq!(s.notices.len(), 2);
        let mut focus = State::default();
        focus.focus.timer.toggle(start);
        focus.focus.timer.toggle(start + Duration::from_secs(2));
        assert!(focus.has_work());
        focus.focus.next();
        assert_eq!(focus.focus.completed, 0);
        assert_eq!(focus.focus.phase, 0);
        for _ in 2..32 {
            s.add_timer().unwrap();
        }
        assert!(s.add_timer().is_err());
    }
    #[test]
    fn monotonic_timers_pause_resume_finish_once_and_stopwatch_keeps_laps() {
        let start = Instant::now();
        let mut t = Timer::new(1, "test".into(), 10);
        t.toggle(start);
        assert_eq!(t.remaining(start + Duration::from_secs(3)).as_secs(), 7);
        t.toggle(start + Duration::from_secs(3));
        assert_eq!(t.remaining(start + Duration::from_secs(99)).as_secs(), 7);
        t.toggle(start + Duration::from_secs(100));
        assert!(t.poll(start + Duration::from_secs(107)));
        assert!(!t.poll(start + Duration::from_secs(200)));
        t.restart();
        assert_eq!(t.remaining(start).as_secs(), 10);
        let mut s = Stopwatch::default();
        s.toggle(start);
        s.lap(start + Duration::from_secs(2));
        s.toggle(start + Duration::from_secs(4));
        s.toggle(start + Duration::from_secs(20));
        assert_eq!(s.elapsed(start + Duration::from_secs(21)).as_secs(), 5);
        assert_eq!(s.laps[0].as_secs(), 2);
    }
    #[test]
    fn dst_missing_and_repeated_local_times_are_explicit() {
        let zone = chrono_tz::America::New_York;
        let parse = |s| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        assert!(resolve_local(zone, parse("2026-03-08 02:30"), false).is_err());
        let a = resolve_local(zone, parse("2026-11-01 01:30"), false).unwrap();
        let b = resolve_local(zone, parse("2026-11-01 01:30"), true).unwrap();
        assert_eq!((b - a).num_hours(), 1);
        let now = resolve_local(zone, parse("2026-03-07 03:00"), false).unwrap();
        let next =
            next_daily(now, zone, NaiveTime::from_hms_opt(2, 30, 0).unwrap(), false).unwrap();
        assert_eq!(
            next.with_timezone(&zone).date_naive().to_string(),
            "2026-03-09"
        );
    }
    #[test]
    fn alarms_forward_jump_catch_up_once_and_backward_jump_never_replays() {
        let mut s = State {
            alarm_zone: chrono_tz::UTC,
            alarm_text: "2026-10-06 09:00".into(),
            daily: true,
            ..Default::default()
        };
        let at = DateTime::parse_from_rfc3339("2026-10-06T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        s.add_alarm(at).unwrap();
        let now = at + chrono::Duration::days(5);
        assert!(s.poll(Instant::now(), now));
        assert_eq!(s.notices.len(), 1);
        assert!(!s.poll(Instant::now(), now));
        assert!(!s.poll(Instant::now(), at));
        assert!(s.alarms[0].at > now);
        s.snooze(Source::Alarm(s.alarms[0].id), now);
        assert!(s.notices.is_empty());
        assert!(s.poll(Instant::now(), now + chrono::Duration::minutes(5)));
    }
    #[test]
    fn focus_four_completed_sessions_choose_long_break() {
        let mut f = Focus::default();
        for _ in 0..3 {
            f.timer.finished = true;
            f.next();
            assert_eq!(f.phase, 1);
            f.timer.finished = true;
            f.next();
            assert_eq!(f.phase, 0);
        }
        f.timer.finished = true;
        f.next();
        assert_eq!(f.phase, 2);
        assert_eq!(f.timer.total.as_secs(), 900);
    }
}
