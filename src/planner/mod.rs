//! Local, explicitly saved notes and calendar events. No network or OS scheduler.
mod store;
#[cfg(test)]
mod tests;
mod ui;

use anyhow::{Result, ensure};
use chrono::{DateTime, Datelike, Days, Duration, Local, NaiveDate, NaiveDateTime, TimeZone};
use eframe::egui;
use lunar_rust::{
    lunar::LunarRefHelper,
    solar::{self, SolarRefHelper},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc,
    time::Instant,
};

const MIN_YEAR: i32 = 1901;
const MAX_YEAR: i32 = 2099;
const MAX_ITEMS: usize = 2000;
const MAX_BODY: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Repeat {
    Once,
    Daily,
    Weekly,
}
impl Repeat {
    fn label(self) -> &'static str {
        match self {
            Self::Once => "不重复",
            Self::Daily => "每天",
            Self::Weekly => "每周",
        }
    }
    fn days(self) -> Option<i64> {
        match self {
            Self::Once => None,
            Self::Daily => Some(1),
            Self::Weekly => Some(7),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Schedule {
    start: NaiveDateTime,
    minutes: u32,
    remind: bool,
    repeat: Repeat,
    done: bool,
    handled: Option<NaiveDateTime>,
    snooze: Option<(NaiveDateTime, i64)>,
}
impl Schedule {
    fn on_day(&self, date: NaiveDate) -> bool {
        let delta = date.signed_duration_since(self.start.date()).num_days();
        delta >= 0 && self.repeat.days().map_or(delta == 0, |n| delta % n == 0)
    }

    // Missed repetitions coalesce to the latest occurrence, rather than a flood.
    fn due<T: TimeZone>(&self, now: DateTime<T>) -> Option<NaiveDateTime> {
        if !self.remind || self.done {
            return None;
        }
        let limit = now
            .naive_local()
            .checked_add_signed(Duration::minutes(i64::from(self.minutes)))?;
        if limit < self.start {
            return None;
        }
        let occurrence = if let Some(days) = self.repeat.days() {
            let mut cycles = limit
                .date()
                .signed_duration_since(self.start.date())
                .num_days()
                / days;
            let candidate = self
                .start
                .checked_add_days(Days::new((cycles * days) as u64))?;
            if candidate > limit {
                cycles -= 1;
            }
            if cycles < 0 {
                return None;
            }
            self.start
                .checked_add_days(Days::new((cycles * days) as u64))?
        } else {
            self.start
        };
        if self.handled.is_some_and(|handled| handled >= occurrence) {
            return None;
        }
        if self
            .snooze
            .is_some_and(|(at, until)| at == occurrence && now.timestamp() < until)
        {
            return None;
        }
        // A DST gap is not silently shifted; an ambiguous time uses its first occurrence.
        let at = now.timezone().from_local_datetime(&occurrence).earliest()?;
        (now.timestamp() >= at.timestamp() - i64::from(self.minutes) * 60).then_some(occurrence)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Item {
    id: String,
    revision: i64,
    title: String,
    body: String,
    pinned: bool,
    trash: bool,
    updated: i64,
    schedule: Option<Schedule>,
}
impl Item {
    fn new(date: Option<NaiveDate>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            title: String::new(),
            body: String::new(),
            pinned: false,
            trash: false,
            updated: 0,
            schedule: date.map(|date| Schedule {
                start: date.and_hms_opt(9, 0, 0).unwrap(),
                minutes: 10,
                remind: true,
                repeat: Repeat::Once,
                done: false,
                handled: None,
                snooze: None,
            }),
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            uuid::Uuid::parse_str(&self.id)?.to_string() == self.id,
            "记录标识无效"
        );
        ensure!(self.revision >= 0, "记录版本无效");
        ensure!(
            !self.title.trim().is_empty()
                && self.title.chars().count() <= 120
                && !self.title.chars().any(char::is_control),
            "标题需要 1–120 个字符，不能包含换行"
        );
        ensure!(self.body.len() <= MAX_BODY, "正文最多 128 KiB");
        if let Some(s) = &self.schedule {
            ensure!(
                (MIN_YEAR..=MAX_YEAR).contains(&s.start.year()),
                "日程支持 1901–2099 年"
            );
            ensure!(s.minutes <= 10080, "提醒最多提前 7 天");
            ensure!(
                s.repeat
                    .days()
                    .is_none_or(|days| i64::from(s.minutes) < days * 1440),
                "重复日程的提前时间需要短于重复周期"
            );
        }
        Ok(())
    }
}

#[derive(Clone)]
struct LunarDay {
    full: String,
    short: String,
}
fn lunar_day(date: NaiveDate) -> LunarDay {
    let solar = solar::from_ymd(
        i64::from(date.year()),
        i64::from(date.month()),
        i64::from(date.day()),
    );
    let lunar = solar.get_lunar();
    let term = lunar.get_jie_qi();
    let mut festivals = solar.get_festivals();
    festivals.extend(lunar.get_festivals());
    let festivals = festivals.join(" · ");
    let day = lunar.get_day_in_chinese();
    let month = format!("{}月", lunar.get_month_in_chinese());
    LunarDay {
        full: format!(
            "农历 {}年{month}{day}  {}  {}",
            lunar.get_year_in_chinese(),
            term,
            festivals
        ),
        short: if !term.is_empty() {
            term
        } else if !festivals.is_empty() {
            festivals.chars().take(4).collect()
        } else if day == "初一" {
            month
        } else {
            day
        },
    }
}

type LoadResult = std::result::Result<Vec<Item>, String>;
pub struct State {
    path: PathBuf,
    items: Vec<Item>,
    pending: Option<mpsc::Receiver<LoadResult>>,
    saving: Option<String>,
    loaded: bool,
    draft: Option<Item>,
    original: Option<Item>,
    date_text: String,
    time_text: String,
    query: String,
    trash: bool,
    pub calendar: bool,
    selected: NaiveDate,
    month: NaiveDate,
    jump: String,
    lunar: HashMap<NaiveDate, LunarDay>,
    message: String,
    error: bool,
    last_tick: Instant,
    shown: HashSet<(String, NaiveDateTime)>,
    alarms: Vec<(String, NaiveDateTime)>,
    alarm_open: bool,
    focus_editor: bool,
    #[cfg(feature = "ui-preview")]
    pub preview_delivered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "ui-preview")]
    pub preview_open_reminder_rect: Option<egui::Rect>,
}
impl State {
    /// Receive a snapshot in memory. Persistence still requires the Save action.
    pub fn receive_text(&mut self, source: &str, text: &str) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "备忘录正在加载或保存，请稍后重试"
        );
        ensure!(
            !self.has_unsaved(),
            "请先保存或放弃备忘 / 日程的当前编辑，再接收结果"
        );
        ensure!(
            !text.is_empty() && text.len() <= MAX_BODY,
            "备忘正文需要 1 字节至 128 KiB，请先缩小结果范围"
        );
        let mut item = Item::new(None);
        item.title = format!("来自 {source}")
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        item.body = text.to_owned();
        item.validate()?;
        self.edit(item);
        self.calendar = false;
        self.trash = false;
        self.query.clear();
        self.message = "结果已填入新备忘草稿；点击保存后才会保留到本机。".into();
        self.error = false;
        Ok(())
    }
    pub fn transfer_text(&self) -> Option<(String, &str)> {
        self.draft.as_ref().filter(|item| !item.trash).map(|item| {
            (
                format!(
                    "{}正文 · {}",
                    if item.schedule.is_some() {
                        "日程"
                    } else {
                        "备忘录"
                    },
                    item.title
                ),
                item.body.as_str(),
            )
        })
    }
    fn open_event(&mut self, id: &str, at: NaiveDateTime) -> Result<()> {
        ensure!(self.pending.is_none(), "正在读写本地记录，请稍后打开日程");
        let item = self
            .items
            .iter()
            .find(|i| i.id == id && !i.trash && i.schedule.is_some())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("日程已不可用，请重新加载"))?;
        if self.draft.as_ref().is_none_or(|draft| draft.id != id) {
            ensure!(
                !self.has_unsaved(),
                "请先保存或放弃当前备忘 / 日程编辑，再打开提醒对应的日程"
            );
            self.edit(item);
        }
        self.calendar = true;
        self.trash = false;
        self.query.clear();
        self.select_date(at.date());
        self.focus_editor = true;
        self.alarm_open = false;
        Ok(())
    }
    pub fn saving(&self) -> bool {
        self.pending.is_some() && self.saving.is_some()
    }
    pub fn needs_clock(&self) -> bool {
        self.pending.is_some()
            || self.items.iter().any(|i| {
                !i.trash
                    && i.schedule.as_ref().is_some_and(|s| {
                        s.remind
                            && !s.done
                            && (s.repeat != Repeat::Once || s.handled != Some(s.start))
                    })
            })
    }
    pub fn new(path: PathBuf) -> Self {
        let today = Local::now().date_naive();
        let mut state = Self {
            #[cfg(feature = "ui-preview")]
            preview_open_reminder_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_delivered: Default::default(),
            path,
            items: Vec::new(),
            pending: None,
            saving: None,
            loaded: false,
            draft: None,
            original: None,
            date_text: String::new(),
            time_text: String::new(),
            query: String::new(),
            trash: false,
            calendar: false,
            selected: today,
            month: today.with_day(1).unwrap(),
            jump: today.to_string(),
            lunar: HashMap::new(),
            message: String::new(),
            error: false,
            last_tick: Instant::now() - std::time::Duration::from_secs(2),
            shown: HashSet::new(),
            alarms: Vec::new(),
            alarm_open: false,
            focus_editor: false,
        };
        state.reload();
        state
    }
    fn launch(&mut self, item: Option<Item>) {
        if self.pending.is_some() {
            return;
        }
        self.saving = item.as_ref().map(|i| i.id.clone());
        let path = self.path.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                if let Some(item) = item {
                    store::save(&path, item)?;
                }
                store::load(&path)
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }
    fn reload(&mut self) {
        self.launch(None);
    }
    pub fn has_unsaved(&self) -> bool {
        if self.pending.is_some() && self.saving.is_some() {
            return true;
        }
        self.draft.as_ref().is_some_and(|item| {
            item.revision == 0 && (!item.title.is_empty() || !item.body.is_empty())
        }) || self.draft != self.original
            || self
                .draft
                .as_ref()
                .and_then(|i| i.schedule.as_ref())
                .is_some_and(|s| {
                    self.date_text != s.start.date().to_string()
                        || self.time_text != s.start.format("%H:%M").to_string()
                })
    }
    fn edit(&mut self, item: Item) {
        self.focus_editor = true;
        self.date_text = item
            .schedule
            .as_ref()
            .map(|s| s.start.date().to_string())
            .unwrap_or_default();
        self.time_text = item
            .schedule
            .as_ref()
            .map(|s| s.start.format("%H:%M").to_string())
            .unwrap_or_default();
        self.original = Some(item.clone());
        self.draft = Some(item);
    }
    fn discard(&mut self) {
        if let Some(original) = self.original.clone() {
            if original.revision == 0 {
                self.draft = None;
                self.original = None;
                self.date_text.clear();
                self.time_text.clear();
            } else {
                self.edit(original);
            }
        }
    }
    fn may_leave(&mut self) -> bool {
        if self.has_unsaved() {
            self.message = "请先保存或放弃当前编辑，再打开另一条记录。".into();
            self.error = true;
            false
        } else {
            true
        }
    }
    fn new_draft(&mut self, date: Option<NaiveDate>) {
        if self.may_leave() {
            self.edit(Item::new(date));
        }
    }
    fn save_draft(&mut self) {
        let Some(mut item) = self.draft.clone() else {
            return;
        };
        let result = (|| -> Result<()> {
            if let Some(s) = &mut item.schedule {
                let start = NaiveDateTime::parse_from_str(
                    &format!("{} {}", self.date_text.trim(), self.time_text.trim()),
                    "%Y-%m-%d %H:%M",
                )?;
                ensure!(
                    Local.from_local_datetime(&start).single().is_some(),
                    "该本地时间不存在或存在夏令时歧义，请选择其他时间"
                );
                s.start = start;
                if self
                    .original
                    .as_ref()
                    .and_then(|i| i.schedule.as_ref())
                    .is_some_and(|old| {
                        old.start != s.start
                            || old.repeat != s.repeat
                            || old.minutes != s.minutes
                            || old.remind != s.remind
                    })
                {
                    s.handled = None;
                    s.snooze = None;
                }
            }
            item.title = item.title.trim().to_string();
            item.validate()?;
            Ok(())
        })();
        match result {
            Ok(()) => self.launch(Some(item)),
            Err(e) => {
                self.message = e.to_string();
                self.error = true;
            }
        }
    }
    /// Runs before hidden-window early returns. Returns true only for newly due alarms.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        if let Some(rx) = &self.pending {
            let reply = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("本地存储工作线程中断，请重新加载".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(reply) = reply {
                self.pending = None;
                match reply {
                    Ok(items) => {
                        self.items = items;
                        self.loaded = true;
                        self.error = false;
                        self.message = if self.saving.is_some() {
                            "已保存到本机"
                        } else {
                            "已加载本地记录"
                        }
                        .into();
                        if let Some(id) = self.saving.take() {
                            if self.draft.as_ref().is_some_and(|i| i.id == id)
                                && let Some(item) = self.items.iter().find(|i| i.id == id).cloned()
                            {
                                self.edit(item);
                            }
                            self.shown.retain(|(saved, _)| saved != &id);
                        }
                        self.last_tick = Instant::now() - std::time::Duration::from_secs(2);
                    }
                    Err(error) => {
                        self.message = error;
                        self.error = true;
                        self.saving = None;
                    }
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(
            if self.pending.is_some() { 100 } else { 1000 },
        ));
        if self.last_tick.elapsed().as_secs() < 1 {
            return false;
        }
        self.last_tick = Instant::now();
        let now = Local::now();
        self.alarms = self
            .items
            .iter()
            .filter(|i| !i.trash)
            .filter_map(|i| i.schedule.as_ref()?.due(now).map(|at| (i.id.clone(), at)))
            .collect();
        self.alarms.sort_by_key(|(_, at)| *at);
        let due: HashSet<_> = self.alarms.iter().cloned().collect();
        self.shown.retain(|key| due.contains(key));
        let mut fresh = false;
        for key in &self.alarms {
            if self.shown.insert(key.clone()) {
                fresh = true;
            }
        }
        if fresh {
            self.alarm_open = true;
            #[cfg(feature = "ui-preview")]
            self.preview_delivered
                .store(true, std::sync::atomic::Ordering::Release);
        }
        fresh
    }
}
