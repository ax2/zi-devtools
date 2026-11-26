//! The portable clock backup format and its validation, without runtime effects.
use anyhow::{Result, ensure};
pub use chrono::SecondsFormat;
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub const FILE_LIMIT: usize = 512 * 1024;
const MAX_MS: u64 = 3_153_600_000_000; // 100 years; rejects pathological files.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Policy {
    #[default]
    Pause,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Source {
    Timer(u64),
    Alarm(u64),
    Focus,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tone {
    #[default]
    Chime,
    Double,
    Soft,
}
impl Tone {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chime => "清脆提示",
            Self::Double => "双音提醒",
            Self::Soft => "柔和提示",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
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
    pub fn permitted(&self, minute: u16, preview: bool) -> bool {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowTarget {
    World(Tz),
    Stopwatch,
    Timer(u64),
    Focus,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSize {
    target: String,
    pub width: u16,
    pub height: u16,
}
impl SavedSize {
    pub fn new(target: WindowTarget, width: u16, height: u16) -> Self {
        let target = match target {
            WindowTarget::World(zone) => format!("world:{}", zone.name()),
            WindowTarget::Stopwatch => "stopwatch".into(),
            WindowTarget::Timer(id) => format!("timer:{id}"),
            WindowTarget::Focus => "focus".into(),
        };
        Self {
            target,
            width,
            height,
        }
    }
    pub fn validate(&self) -> Result<WindowTarget, String> {
        let target = match self.target.as_str() {
            "stopwatch" => WindowTarget::Stopwatch,
            "focus" => WindowTarget::Focus,
            value if value.starts_with("world:") => value[6..]
                .parse::<Tz>()
                .map(WindowTarget::World)
                .map_err(|_| "无效的小窗时区".to_owned())?,
            value if value.starts_with("timer:") => {
                let id = value[6..]
                    .parse::<u64>()
                    .map_err(|_| "无效的小窗计时器".to_owned())?;
                if id == 0 || id == u64::MAX {
                    return Err("无效的小窗计时器".into());
                }
                WindowTarget::Timer(id)
            }
            _ => return Err("无效的小窗目标".into()),
        };
        if !(320..=1600).contains(&self.width) || !(260..=1200).contains(&self.height) {
            return Err("小窗尺寸越界".into());
        }
        Ok(target)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerData {
    pub id: u64,
    pub name: String,
    pub total: u64,
    pub cycle: u64,
    pub remaining: u64,
    pub running: bool,
    pub finished: bool,
}
impl TimerData {
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
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmData {
    pub id: u64,
    pub name: String,
    pub zone: String,
    pub at: DateTime<Utc>,
    pub daily: bool,
    pub late: bool,
    pub time: NaiveTime,
    pub enabled: bool,
    pub snooze: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeData {
    pub source: Source,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub tool_version: String,
    pub saved_at: DateTime<Utc>,
    pub policy: Policy,
    pub tab: u8,
    #[serde(deserialize_with = "bounded_list::<_, _, 12>")]
    pub zones: Vec<String>,
    pub analog: bool,
    pub meeting: i32,
    pub stopwatch: u64,
    #[serde(deserialize_with = "bounded_list::<_, _, 1024>")]
    pub laps: Vec<u64>,
    #[serde(deserialize_with = "bounded_list::<_, _, 32>")]
    pub timers: Vec<TimerData>,
    #[serde(deserialize_with = "bounded_list::<_, _, 64>")]
    pub alarms: Vec<AlarmData>,
    pub focus: TimerData,
    pub work: u64,
    pub short: u64,
    pub long: u64,
    pub completed: u32,
    pub phase: u8,
    #[serde(deserialize_with = "bounded_list::<_, _, 97>")]
    pub notices: Vec<NoticeData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "bounded_list::<_, _, 48>")]
    pub window_sizes: Vec<SavedSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<Settings>,
}
// The backing str is already valid UTF-8. Every scalar begins with a byte
// other than a 10xxxxxx continuation byte.
// Apply the format's existing list bounds while reading, before allocating
// further entries from a malformed backup.
fn bounded_list<'de, D, T, const LIMIT: usize>(decoder: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    use serde::de::{DeserializeSeed, Error as _, SeqAccess, Visitor};
    use std::{fmt, marker::PhantomData};

    struct Extra;
    impl<'de> DeserializeSeed<'de> for Extra {
        type Value = ();

        fn deserialize<D: serde::Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
            Err(D::Error::custom("时钟记录数量越界"))
        }
    }

    struct Bounded<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> Visitor<'de> for Bounded<T, LIMIT> {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "at most {LIMIT} clock backup records")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
            let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(LIMIT));
            while values.len() < LIMIT {
                let Some(value) = sequence.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            sequence.next_element_seed(Extra)?;
            Ok(values)
        }
    }

    decoder.deserialize_seq(Bounded::<T, LIMIT>(PhantomData))
}

fn scalar_count(value: &str) -> usize {
    const START_BITS: u64 = 0x8080_8080_8080_8080;
    let (chunks, remainder) = value.as_bytes().as_chunks::<8>();
    let continuations: usize = chunks
        .iter()
        .map(|chunk| {
            let word = u64::from_ne_bytes(*chunk);
            (word & !(word << 1) & START_BITS).count_ones() as usize
        })
        .sum();
    value.len()
        - continuations
        - remainder
            .iter()
            .filter(|byte| **byte & 0xc0 == 0x80)
            .count()
}

#[cfg(test)]
#[test]
fn scalar_count_matches_utf8_decoding_across_chunks() {
    for input in [
        "",
        "abcdefgh",
        "é中🕑ø",
        "abcde中🕑abcdef🕑à😀",
        "🕑🕑🕑🕑",
        "中中中中中",
    ] {
        assert_eq!(scalar_count(input), input.chars().count());
    }
}

fn name(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && scalar_count(value) <= 80,
        "名称需要1–80个字符"
    );
    Ok(())
}
impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [1, 2, 3].contains(&self.schema),
            "不支持此时钟文件版本，保留原文件"
        );
        ensure!(
            match self.schema {
                1 => self.audio.is_none(),
                2 | 3 => self.audio.as_ref().is_some_and(Settings::valid),
                _ => false,
            },
            "声音设置与保存格式不一致或越界"
        );
        ensure!(
            self.window_sizes.len() <= 48 && (self.schema == 3 || self.window_sizes.is_empty()),
            "小窗尺寸数量越界或与保存格式不一致"
        );
        let mut window_targets = std::collections::HashSet::new();
        for size in &self.window_sizes {
            ensure!(
                window_targets.insert(size.validate().map_err(anyhow::Error::msg)?),
                "小窗目标重复"
            );
        }
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
        let mut timers = std::collections::HashSet::new();
        for t in &self.timers {
            t.validate()?;
            ensure!(
                t.id > 0 && t.id < u64::MAX && timers.insert(t.id),
                "计时器ID无效或重复"
            );
        }
        let mut alarms = std::collections::HashSet::new();
        for a in &self.alarms {
            name(&a.name)?;
            ensure!(
                a.id > 0
                    && a.id < u64::MAX
                    && !timers.contains(&a.id)
                    && alarms.insert(a.id)
                    && a.zone.parse::<Tz>().is_ok(),
                "闹钟ID或时区无效"
            );
        }
        let mut sources = std::collections::HashSet::new();
        for n in &self.notices {
            ensure!(
                !n.title.is_empty() && scalar_count(&n.title) <= 512 && sources.insert(n.source),
                "提醒无效或重复"
            );
            ensure!(
                match n.source {
                    Source::Timer(id) => timers.contains(&id),
                    Source::Alarm(id) => alarms.contains(&id),
                    Source::Focus => true,
                },
                "提醒来源不存在"
            );
        }
        Ok(())
    }
}
