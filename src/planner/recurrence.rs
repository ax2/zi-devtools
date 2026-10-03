use super::*;

impl Schedule {
    // Always anchor to the original day, not the last clamped occurrence.
    fn date_in_month(&self, year: i32, month: u32) -> Option<NaiveDate> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return None;
        }
        NaiveDate::from_ymd_opt(year, month, self.start.day()).or_else(|| {
            self.clamp_missing_day
                .then(|| {
                    NaiveDate::from_ymd_opt(year, month, 1)?
                        .checked_add_months(chrono::Months::new(1))?
                        .pred_opt()
                })
                .flatten()
        })
    }
    pub(super) fn rule_label(&self) -> String {
        let rule = match (self.repeat, self.clamp_missing_day) {
            (Repeat::Monthly, false) => "每月 · 缺日跳过",
            (Repeat::Monthly, true) => "每月 · 缺日用月底",
            (Repeat::Yearly, false) => "每年 · 缺日跳过",
            (Repeat::Yearly, true) => "每年 · 缺日用月底",
            _ => self.repeat.label(),
        };
        self.repeat_until
            .map_or_else(|| rule.to_owned(), |day| format!("{rule} · 截至 {day}"))
    }
    pub(super) fn last_repeat_day(&self) -> NaiveDate {
        self.repeat_until
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(MAX_YEAR, 12, 31).unwrap())
    }
    pub(super) fn on_day(&self, date: NaiveDate) -> bool {
        if date < self.start.date()
            || date > self.last_repeat_day()
            || !(MIN_YEAR..=MAX_YEAR).contains(&date.year())
        {
            return false;
        }
        match self.repeat {
            Repeat::Monthly => self.date_in_month(date.year(), date.month()) == Some(date),
            Repeat::Yearly => {
                date.month() == self.start.month()
                    && self.date_in_month(date.year(), self.start.month()) == Some(date)
            }
            _ => {
                let delta = date.signed_duration_since(self.start.date()).num_days();
                self.repeat.days().map_or(delta == 0, |n| delta % n == 0)
            }
        }
    }
    pub(super) fn latest(&self, limit: NaiveDateTime) -> Option<NaiveDateTime> {
        let limit = limit.min(self.last_repeat_day().and_hms_opt(23, 59, 59)?);
        if limit < self.start {
            return None;
        }
        let valid = |day: Option<NaiveDate>| {
            day.map(|d| d.and_time(self.start.time()))
                .filter(|at| *at >= self.start && *at <= limit)
        };
        match self.repeat {
            Repeat::Once => Some(self.start),
            Repeat::Daily | Repeat::Weekly => {
                let days = self.repeat.days()?;
                let mut cycles = limit
                    .date()
                    .signed_duration_since(self.start.date())
                    .num_days()
                    / days;
                if self
                    .start
                    .checked_add_days(Days::new((cycles * days) as u64))?
                    > limit
                {
                    cycles -= 1;
                }
                if cycles < 0 {
                    return None;
                }
                self.start
                    .checked_add_days(Days::new((cycles * days) as u64))
            }
            // A 31st can miss the current and previous month; a Feb 29 can
            // miss seven years around a non-leap century. Work stays bounded.
            Repeat::Monthly => (0..=2).find_map(|back| {
                let month = limit
                    .date()
                    .with_day(1)?
                    .checked_sub_months(chrono::Months::new(back))?;
                valid(self.date_in_month(month.year(), month.month()))
            }),
            Repeat::Yearly => (0..=8).find_map(|back| {
                valid(self.date_in_month(limit.year() - back, self.start.month()))
            }),
        }
    }
    // Missed repetitions coalesce to the latest occurrence, rather than a flood.
    pub(super) fn due<T: TimeZone>(&self, now: DateTime<T>) -> Option<NaiveDateTime> {
        if !self.remind || self.done {
            return None;
        }
        let mut limit = now
            .naive_local()
            .checked_add_signed(Duration::minutes(i64::from(self.minutes)))?;
        if let Some(clock) = self.reminder_time {
            limit = limit.checked_sub_signed(clock.signed_duration_since(NaiveTime::MIN))?;
        }
        let occurrence = self.latest(limit)?;
        if self.handled.is_some_and(|handled| handled >= occurrence)
            || self
                .snooze
                .is_some_and(|(at, until)| at == occurrence && now.timestamp() < until)
        {
            return None;
        }
        // A DST gap is not shifted; an ambiguous time uses its first occurrence.
        let at = now
            .timezone()
            .from_local_datetime(&self.reminder_at(occurrence))
            .earliest()?;
        (now.timestamp() >= at.timestamp() - i64::from(self.minutes) * 60).then_some(occurrence)
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_recurrence(&mut self, yearly: bool) {
        self.preview(true, false);
        let mut item = self.items[2].clone();
        item.title = if yearly {
            "闰年生日安排"
        } else {
            "每月整理账单与资料"
        }
        .into();
        item.body = "检查重复规则，再保存到本机。\n日期始终以原始起点计算。".into();
        let s = item.schedule.as_mut().unwrap();
        s.start = NaiveDate::from_ymd_opt(
            if yearly { 2024 } else { 2026 },
            if yearly { 2 } else { 1 },
            if yearly { 29 } else { 31 },
        )
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
        s.repeat = if yearly {
            Repeat::Yearly
        } else {
            Repeat::Monthly
        };
        s.clamp_missing_day = !yearly;
        s.remind = false;
        self.replace_items(vec![item.clone()]);
        self.edit(item);
        self.select_date(
            NaiveDate::from_ymd_opt(
                if yearly { 2028 } else { 2027 },
                2,
                if yearly { 29 } else { 28 },
            )
            .unwrap(),
        );
        self.focus_editor = false;
    }
    pub fn preview_recurrence_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_recurrence(false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-repeat-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[0].clone();
                item.revision = 0;
                item.schedule.as_mut().unwrap().clamp_missing_day = false;
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
                        .clamp_missing_day
                );
                assert!(
                    !store::load(&self.path).unwrap()[0]
                        .schedule
                        .as_ref()
                        .unwrap()
                        .clamp_missing_day
                );
            }
            2 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.has_unsaved() && !self.error);
                let saved = store::load(&self.path).unwrap();
                let s = saved[0].schedule.as_ref().unwrap();
                assert!(s.clamp_missing_day && !s.remind && s.repeat == Repeat::Monthly);
                assert!(s.on_day(NaiveDate::from_ymd_opt(2027, 2, 28).unwrap()));
                assert!(s.on_day(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()));
                std::fs::remove_file(&self.path).unwrap();
                std::fs::remove_dir(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS recurrence UI: actual policy checkbox changes only draft; actual save persists month-end fallback without reminder opt-in or day drift"
                );
            }
            _ => unreachable!(),
        }
        true
    }
}
