use super::*;
#[cfg(feature = "ui-preview")]
use chrono::Timelike;

impl Schedule {
    pub(super) fn validate_interval(&self) -> Result<()> {
        if self.all_day {
            ensure!(
                self.start.time() == NaiveTime::MIN,
                "全天日程从当天零点开始"
            );
            ensure!(
                self.end.is_some() && self.reminder_time.is_some(),
                "全天日程需要结束日期和提醒钟点"
            );
        } else {
            ensure!(self.reminder_time.is_none(), "普通日程使用开始时间提醒");
        }
        if let Some(end) = self.end {
            let ceiling = NaiveDate::from_ymd_opt(MAX_YEAR + 1, 1, 1)
                .unwrap()
                .and_time(NaiveTime::MIN);
            ensure!(
                end > self.start && end <= ceiling,
                "结束时间必须晚于开始时间，且不能超过 2099 年末"
            );
            ensure!(
                !self.all_day || end.time() == NaiveTime::MIN,
                "全天日程结束边界必须为零点"
            );
            // Non-overlapping occurrences keep each day and range unambiguous.
            let minimum_days = match self.repeat {
                Repeat::Once => None,
                Repeat::Daily => Some(1),
                Repeat::Weekly => Some(7),
                Repeat::Monthly => Some(28),
                Repeat::Yearly => Some(365),
            };
            ensure!(
                minimum_days.is_none_or(|days| end - self.start <= Duration::days(days)),
                "重复日程的时长不能超过周期：每天 1 天、每周 7 天、每月 28 天、每年 365 天"
            );
        }
        Ok(())
    }
    pub(super) fn end_at(&self, at: NaiveDateTime) -> Option<NaiveDateTime> {
        at.checked_add_signed(self.end? - self.start)
    }
    pub(super) fn reminder_at(&self, at: NaiveDateTime) -> NaiveDateTime {
        self.reminder_time
            .map_or(at, |clock| at.date().and_time(clock))
    }
    /// Latest occurrence intersecting the calendar day. End boundaries are exclusive.
    pub(super) fn covering(&self, day: NaiveDate) -> Option<NaiveDateTime> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&day.year()) {
            return None;
        }
        let midnight = day.and_time(NaiveTime::MIN);
        let at = self.latest(day.and_hms_opt(23, 59, 59).unwrap())?;
        (at.date() == day || self.end_at(at).is_some_and(|end| end > midnight)).then_some(at)
    }
    pub(super) fn display_end(&self) -> Option<NaiveDateTime> {
        self.end.and_then(|end| {
            if self.all_day {
                end.checked_sub_signed(Duration::days(1))
            } else {
                Some(end)
            }
        })
    }
    pub(super) fn range_label(&self, at: NaiveDateTime) -> String {
        if self.all_day {
            let last = self
                .end_at(at)
                .and_then(|end| end.date().pred_opt())
                .unwrap_or(at.date());
            if last == at.date() {
                format!("{} · 全天", at.format("%Y-%m-%d"))
            } else {
                format!("{} 至 {last} · 全天", at.format("%Y-%m-%d"))
            }
        } else if let Some(end) = self.end_at(at) {
            if end.date() == at.date() {
                format!("{}–{}", at.format("%Y-%m-%d %H:%M"), end.format("%H:%M"))
            } else {
                format!(
                    "{} 至 {}",
                    at.format("%Y-%m-%d %H:%M"),
                    end.format(if end.year() == at.year() {
                        "%m-%d %H:%M"
                    } else {
                        "%Y-%m-%d %H:%M"
                    })
                )
            }
        } else {
            at.format("%Y-%m-%d %H:%M").to_string()
        }
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_interval_agenda(&mut self) {
        self.agenda_days = 7;
    }
    pub fn preview_interval_focus(&mut self) {
        self.focus_editor = true;
    }
    pub fn preview_interval(&mut self, all_day: bool) {
        self.preview(true, false);
        let mut item = self.items[2].clone();
        item.title = if all_day {
            "秋日旅行 · 整理行程与照片"
        } else {
            "跨天资料整理"
        }
        .into();
        item.body = "准备资料与清单。\n开始前检查安排；结束后整理结果。".into();
        let s = item.schedule.as_mut().unwrap();
        s.start = NaiveDate::from_ymd_opt(2026, 10, 2)
            .unwrap()
            .and_hms_opt(if all_day { 0 } else { 21 }, 0, 0)
            .unwrap();
        s.end = Some(
            NaiveDate::from_ymd_opt(2026, 10, if all_day { 5 } else { 3 })
                .unwrap()
                .and_hms_opt(if all_day { 0 } else { 10 }, 0, 0)
                .unwrap(),
        );
        s.all_day = all_day;
        s.reminder_time = all_day.then(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        s.repeat = Repeat::Once;
        s.remind = false;
        self.replace_items(vec![item.clone()]);
        self.edit(item);
        self.select_date(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap());
        self.focus_editor = false;
    }
    pub fn preview_interval_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_interval(false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-interval-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[0].clone();
                item.revision = 0;
                store::save(&self.path, item).unwrap();
                self.replace_items(store::load(&self.path).unwrap());
                self.edit(self.items[0].clone());
                self.focus_editor = false;
            }
            1 => {
                assert!(self.has_unsaved());
                assert!(
                    self.draft
                        .as_ref()
                        .unwrap()
                        .schedule
                        .as_ref()
                        .unwrap()
                        .all_day
                );
                assert!(
                    !store::load(&self.path).unwrap()[0]
                        .schedule
                        .as_ref()
                        .unwrap()
                        .all_day
                );
                assert_eq!(self.end_date_text, "2026-10-03");
            }
            2 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error, "{}", self.message);
                let saved = store::load(&self.path).unwrap();
                let s = saved[0].schedule.as_ref().unwrap();
                assert!(s.all_day);
                assert_eq!(s.start.hour(), 0);
                assert_eq!(
                    s.end.unwrap().date(),
                    NaiveDate::from_ymd_opt(2026, 10, 4).unwrap()
                );
                assert_eq!(s.reminder_time.unwrap().hour(), 21);
                assert!(
                    s.covering(NaiveDate::from_ymd_opt(2026, 10, 3).unwrap())
                        .is_some()
                );
                assert!(!self.has_unsaved());
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS interval native all-day checkbox, draft-only protection, save, reload, inclusive end and reminder clock"
                );
                return true;
            }
            _ => unreachable!(),
        }
        false
    }
}
