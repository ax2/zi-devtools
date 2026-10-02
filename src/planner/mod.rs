//! Local, explicitly saved notes and calendar events. No network or OS scheduler.
mod agenda;
mod backup;
mod backup_ui;
#[cfg(feature = "ui-preview")]
mod cutoff_preview;
#[cfg(test)]
mod cutoff_tests;
mod files;
mod ics;
#[cfg(feature = "ui-preview")]
mod ics_preview;
#[cfg(test)]
mod ics_tests;
mod ics_ui;
mod interval;
#[cfg(test)]
mod interval_tests;
mod recurrence;
mod store;
#[cfg(test)]
mod tests;
mod ui;

use anyhow::{Result, ensure};
use chrono::{
    DateTime, Datelike, Days, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
};
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
    Monthly,
    Yearly,
}
impl Repeat {
    fn label(self) -> &'static str {
        match self {
            Self::Once => "不重复",
            Self::Daily => "每天",
            Self::Weekly => "每周",
            Self::Monthly => "每月",
            Self::Yearly => "每年",
        }
    }
    fn days(self) -> Option<i64> {
        match self {
            Self::Once | Self::Monthly | Self::Yearly => None,
            Self::Daily => Some(1),
            Self::Weekly => Some(7),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Schedule {
    start: NaiveDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<NaiveDateTime>,
    #[serde(default, skip_serializing_if = "is_false")]
    all_day: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reminder_time: Option<NaiveTime>,
    minutes: u32,
    remind: bool,
    repeat: Repeat,
    #[serde(default, skip_serializing_if = "is_false")]
    clamp_missing_day: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repeat_until: Option<NaiveDate>,
    done: bool,
    handled: Option<NaiveDateTime>,
    snooze: Option<(NaiveDateTime, i64)>,
}
fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: String,
    revision: i64,
    title: String,
    body: String,
    pinned: bool,
    trash: bool,
    updated: i64,
    schedule: Option<Schedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calendar_uid: Option<String>,
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
            calendar_uid: None,
            schedule: date.map(|date| Schedule {
                start: date.and_hms_opt(9, 0, 0).unwrap(),
                end: None,
                all_day: false,
                reminder_time: None,
                minutes: 10,
                remind: true,
                repeat: Repeat::Once,
                clamp_missing_day: false,
                repeat_until: None,
                done: false,
                handled: None,
                snooze: None,
            }),
        }
    }
    fn validate(&self) -> Result<()> {
        if let Some(uid) = &self.calendar_uid {
            ensure!(
                self.schedule.is_some()
                    && !uid.is_empty()
                    && uid.len() <= 1024
                    && !uid.chars().any(char::is_control),
                "日历 UID 无效"
            );
        }
        ensure!(
            uuid::Uuid::parse_str(&self.id)?.to_string() == self.id,
            "记录标识无效"
        );
        ensure!(self.revision >= 0, "记录版本无效");
        ensure!(valid_timestamp(self.updated), "记录更新时间无效");
        ensure!(
            !self.title.trim().is_empty()
                && self.title.chars().count() <= 120
                && !self.title.chars().any(char::is_control),
            "标题需要 1–120 个字符，不能包含换行"
        );
        ensure!(self.body.len() <= MAX_BODY, "正文最多 128 KiB");
        if let Some(s) = &self.schedule {
            s.validate_interval()?;
            ensure!(
                s.repeat_until.is_none_or(|day| s.repeat != Repeat::Once
                    && day >= s.start.date()
                    && (MIN_YEAR..=MAX_YEAR).contains(&day.year())),
                "重复截止日期需不早于开始日期，且在 1901–2099 年内；单次日程不能设置重复截止"
            );
            ensure!(
                !s.clamp_missing_day || matches!(s.repeat, Repeat::Monthly | Repeat::Yearly),
                "仅每月 / 每年重复可设置月底替代"
            );
            ensure!(
                s.snooze.is_none_or(|(_, until)| valid_timestamp(until)),
                "稍后提醒时间无效"
            );
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

fn valid_timestamp(value: i64) -> bool {
    DateTime::from_timestamp(value, 0).is_some_and(|time| (1..=9999).contains(&time.year()))
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

enum Reply {
    IcsReady(Box<ics_ui::Review>),
    IcsSaved(PathBuf, usize),
    IcsImported(Vec<Item>, usize),
    Loaded(Vec<Item>),
    Imported(Item),
    Exported(PathBuf, usize),
    BackupReady(Box<backup::Review>),
    BackupSaved(PathBuf, usize),
    Restored(Vec<Item>),
}
type LoadResult = std::result::Result<Reply, String>;
pub struct State {
    path: PathBuf,
    items: Vec<Item>,
    pending: Option<mpsc::Receiver<LoadResult>>,
    saving: Option<String>,
    deleting: Option<Vec<String>>,
    purge_review: Option<Vec<Item>>,
    file_operation: bool,
    export_review: Option<files::Export>,
    backup_review: Option<backup::Review>,
    ics_review: Option<ics_ui::Review>,
    loaded: bool,
    draft: Option<Item>,
    original: Option<Item>,
    date_text: String,
    time_text: String,
    end_date_text: String,
    end_time_text: String,
    repeat_until_text: String,
    query: String,
    trash: bool,
    pub calendar: bool,
    agenda_days: u32,
    agenda_done: bool,
    agenda_cache: agenda::Cache,
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
    #[cfg(feature = "ui-preview")]
    pub preview_purge_rects: Option<[egui::Rect; 2]>,
    #[cfg(feature = "ui-preview")]
    pub preview_export_cancel_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    pub preview_backup_rects: [Option<egui::Rect>; 4],
    #[cfg(feature = "ui-preview")]
    pub preview_agenda_rect: Option<egui::Rect>,
    #[cfg(feature = "ui-preview")]
    pub preview_recurrence_rects: [Option<(egui::Rect, egui::Rect)>; 2],
    #[cfg(feature = "ui-preview")]
    pub preview_interval_rect: Option<(egui::Rect, egui::Rect)>,
    #[cfg(feature = "ui-preview")]
    pub preview_ics_rects: [Option<egui::Rect>; 4],
    #[cfg(feature = "ui-preview")]
    pub preview_cutoff_rects: [Option<(egui::Rect, egui::Rect)>; 2],
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
                "请先保存或放弃当前备忘 / 日程编辑，再打开其他日程"
            );
            self.edit(item);
        }
        self.calendar = true;
        self.agenda_days = 1;
        self.trash = false;
        self.query.clear();
        self.select_date(at.date());
        self.focus_editor = true;
        self.alarm_open = false;
        Ok(())
    }
    pub fn saving(&self) -> bool {
        self.pending.is_some()
            && (self.saving.is_some() || self.deleting.is_some() || self.file_operation)
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
            preview_purge_rects: None,
            #[cfg(feature = "ui-preview")]
            preview_export_cancel_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_backup_rects: [None; 4],
            #[cfg(feature = "ui-preview")]
            preview_agenda_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_recurrence_rects: [None; 2],
            #[cfg(feature = "ui-preview")]
            preview_interval_rect: None,
            #[cfg(feature = "ui-preview")]
            preview_ics_rects: [None; 4],
            #[cfg(feature = "ui-preview")]
            preview_cutoff_rects: [None; 2],
            #[cfg(feature = "ui-preview")]
            preview_delivered: Default::default(),
            path,
            items: Vec::new(),
            pending: None,
            saving: None,
            deleting: None,
            purge_review: None,
            file_operation: false,
            export_review: None,
            backup_review: None,
            ics_review: None,
            loaded: false,
            draft: None,
            original: None,
            date_text: String::new(),
            time_text: String::new(),
            end_date_text: String::new(),
            end_time_text: String::new(),
            repeat_until_text: String::new(),
            query: String::new(),
            trash: false,
            calendar: false,
            agenda_days: 1,
            agenda_done: false,
            agenda_cache: Default::default(),
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
                store::load(&path).map(Reply::Loaded)
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }
    fn reload(&mut self) {
        self.launch(None);
    }
    fn request_purge(&mut self, all: bool) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(!self.has_unsaved(), "请先保存或放弃当前编辑，再清理回收站");
        let selected = self.draft.as_ref().map(|i| i.id.as_str());
        let reviewed: Vec<_> = self
            .items
            .iter()
            .filter(|i| i.trash && (all || Some(i.id.as_str()) == selected))
            .cloned()
            .collect();
        ensure!(!reviewed.is_empty(), "没有可永久删除的回收站记录");
        self.purge_review = Some(reviewed);
        Ok(())
    }
    fn confirm_purge(&mut self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none() && !self.has_unsaved(),
            "记录正在读写或有未保存编辑，请稍后重试"
        );
        let reviewed = self
            .purge_review
            .take()
            .ok_or_else(|| anyhow::anyhow!("请先预览要删除的记录"))?;
        self.deleting = Some(reviewed.iter().map(|i| i.id.clone()).collect());
        let path = self.path.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(
                store::purge(&path, &reviewed)
                    .map(Reply::Loaded)
                    .map_err(|e| e.to_string()),
            );
        });
        Ok(())
    }
    pub fn has_unsaved(&self) -> bool {
        if self.saving() {
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
                        || self.time_text != s.reminder_at(s.start).format("%H:%M").to_string()
                        || s.repeat_until
                            .is_some_and(|day| self.repeat_until_text != day.to_string())
                        || s.display_end().is_some_and(|end| {
                            self.end_date_text != end.date().to_string()
                                || (!s.all_day
                                    && self.end_time_text != end.format("%H:%M").to_string())
                        })
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
            .map(|s| s.reminder_at(s.start).format("%H:%M").to_string())
            .unwrap_or_default();
        let end = item.schedule.as_ref().and_then(|s| {
            s.display_end()
                .or_else(|| s.start.checked_add_signed(Duration::hours(1)))
        });
        self.end_date_text = end.map(|end| end.date().to_string()).unwrap_or_default();
        self.end_time_text = end
            .map(|end| end.format("%H:%M").to_string())
            .unwrap_or_default();
        self.repeat_until_text = item
            .schedule
            .as_ref()
            .and_then(|s| s.repeat_until)
            .map(|day| day.to_string())
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
                    &format!(
                        "{} {}",
                        self.date_text.trim(),
                        if s.all_day {
                            "00:00"
                        } else {
                            self.time_text.trim()
                        }
                    ),
                    "%Y-%m-%d %H:%M",
                )?;
                ensure!(
                    s.all_day || Local.from_local_datetime(&start).single().is_some(),
                    "该本地时间不存在或存在夏令时歧义，请选择其他时间"
                );
                s.start = start;
                s.repeat_until = if s.repeat == Repeat::Once || s.repeat_until.is_none() {
                    None
                } else {
                    Some(
                        NaiveDate::parse_from_str(self.repeat_until_text.trim(), "%Y-%m-%d")
                            .map_err(|_| anyhow::anyhow!("重复截止日期格式应为 YYYY-MM-DD"))?,
                    )
                };
                s.reminder_time = if s.all_day {
                    Some(NaiveTime::parse_from_str(self.time_text.trim(), "%H:%M")?)
                } else {
                    None
                };
                if s.end.is_some() || s.all_day {
                    let end = NaiveDateTime::parse_from_str(
                        &format!(
                            "{} {}",
                            self.end_date_text.trim(),
                            if s.all_day {
                                "00:00"
                            } else {
                                self.end_time_text.trim()
                            }
                        ),
                        "%Y-%m-%d %H:%M",
                    )?;
                    s.end = Some(if s.all_day {
                        end.checked_add_signed(Duration::days(1))
                            .ok_or_else(|| anyhow::anyhow!("结束日期无效"))?
                    } else {
                        end
                    });
                    ensure!(
                        s.all_day || Local.from_local_datetime(&end).single().is_some(),
                        "结束时间不存在或存在夏令时歧义，请选择其他时间"
                    );
                }
                ensure!(
                    !s.remind
                        || Local
                            .from_local_datetime(&s.reminder_at(start))
                            .single()
                            .is_some(),
                    "提醒时间不存在或存在夏令时歧义，请选择其他时间"
                );
                if self
                    .original
                    .as_ref()
                    .and_then(|i| i.schedule.as_ref())
                    .is_some_and(|old| {
                        old.start != s.start
                            || old.all_day != s.all_day
                            || old.reminder_time != s.reminder_time
                            || old.repeat != s.repeat
                            || old.clamp_missing_day != s.clamp_missing_day
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
                self.file_operation = false;
                match reply {
                    Ok(Reply::IcsReady(review)) => {
                        self.ics_review = Some(*review);
                        self.message.clear();
                        self.error = false;
                    }
                    Ok(Reply::IcsSaved(path, count)) => {
                        self.message = format!("已导出 {count} 条日程 · {}", path.display());
                        self.error = false;
                    }
                    Ok(Reply::IcsImported(items, count)) => {
                        self.items = items;
                        self.loaded = true;
                        self.draft = None;
                        self.original = None;
                        self.date_text.clear();
                        self.time_text.clear();
                        self.end_date_text.clear();
                        self.end_time_text.clear();
                        self.query.clear();
                        self.calendar = true;
                        self.trash = false;
                        self.message = format!(
                            "已导入 / 更新 {count} 条日程；这些条目的提醒已关闭。可打开日程设置提醒后保存。"
                        );
                        self.error = false;
                        self.last_tick = Instant::now() - std::time::Duration::from_secs(2);
                    }
                    Ok(Reply::BackupReady(review)) => {
                        self.backup_review = Some(*review);
                        self.message = "已读取已保存记录的快照，请核对范围后继续。".into();
                        self.error = false;
                    }
                    Ok(Reply::BackupSaved(path, count)) => {
                        self.message = format!("完整备份已保存 · {count} 条 · {}", path.display());
                        self.error = false;
                    }
                    Ok(Reply::Restored(items)) => {
                        self.items = items;
                        self.draft = None;
                        self.original = None;
                        self.date_text.clear();
                        self.time_text.clear();
                        self.loaded = true;
                        self.query.clear();
                        self.message =
                            "备份恢复已完成。日程按本机时区安排，已启用的到期提醒可能立即显示。"
                                .into();
                        self.error = false;
                        self.last_tick = Instant::now() - std::time::Duration::from_secs(2);
                    }
                    Ok(Reply::Imported(item)) => {
                        self.finish_import(item);
                    }
                    Ok(Reply::Exported(path, bytes)) => {
                        self.message = format!(
                            "已导出正文 · {bytes} 字节 · {}；不改变当前编辑的保存状态。",
                            path.display()
                        );
                        self.error = false;
                    }
                    Ok(Reply::Loaded(items)) => {
                        self.items = items;
                        self.loaded = true;
                        self.error = false;
                        self.message = if self.saving.is_some() {
                            "已保存到本机"
                        } else {
                            "已加载本地记录"
                        }
                        .into();
                        if let Some(ids) = self.deleting.take() {
                            self.message = format!(
                                "已永久删除 {} 条回收站记录，记录容量可再次使用。",
                                ids.len()
                            );
                            if self.draft.as_ref().is_some_and(|i| ids.contains(&i.id)) {
                                self.draft = None;
                                self.original = None;
                                self.date_text.clear();
                                self.time_text.clear();
                            }
                            self.shown.retain(|(id, _)| !ids.contains(id));
                        }
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
                        self.deleting = None;
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
