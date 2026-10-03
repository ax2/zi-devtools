use super::*;

impl State {
    pub fn preview_cutoff(&mut self) {
        self.preview(true, false);
        let mut item = self.items[2].clone();
        item.title = "每周资料整理 · 本月结束".into();
        item.body = "检查本周资料和记录。\n最后一次可以跨天完成，截止后不产生新安排。".into();
        let s = item.schedule.as_mut().unwrap();
        s.start = NaiveDate::from_ymd_opt(2026, 10, 2)
            .unwrap()
            .and_hms_opt(21, 0, 0)
            .unwrap();
        s.end = Some(s.start + Duration::hours(36));
        s.repeat = Repeat::Weekly;
        s.remind = false;
        s.repeat_until = Some(NaiveDate::from_ymd_opt(2026, 10, 9).unwrap());
        self.replace_items(vec![item.clone()]);
        self.edit(item);
        self.focus_editor = false;
        self.select_date(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap());
    }
    pub fn preview_cutoff_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_cutoff();
                self.path = std::env::temp_dir()
                    .join(format!("zi-cutoff-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[0].clone();
                item.revision = 0;
                item.schedule.as_mut().unwrap().repeat_until = None;
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
                        .repeat_until
                        .is_some()
                );
                assert!(
                    store::load(&self.path).unwrap()[0]
                        .schedule
                        .as_ref()
                        .unwrap()
                        .repeat_until
                        .is_none()
                );
            }
            2 => {
                assert_eq!(self.repeat_until_text, "2026-10-09");
                assert!(self.has_unsaved());
            }
            3 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && !self.has_unsaved());
                let items = store::load(&self.path).unwrap();
                let s = items[0].schedule.as_ref().unwrap();
                let day = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
                assert_eq!(s.repeat_until, Some(day));
                assert!(!s.on_day(day + Days::new(7)));
                assert!(s.covering(day + Days::new(2)).is_some());
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS cutoff native toggle, typed date, draft guard, save/reload, final carry-in and no later occurrences"
                );
            }
            _ => unreachable!(),
        }
        true
    }
}

impl State {
    pub fn preview_cutoff_interop(&self, output: &std::path::Path) {
        let mut items = Vec::new();
        let mut expected = Vec::new();
        for (repeat, clamp) in [
            (Repeat::Daily, false),
            (Repeat::Weekly, false),
            (Repeat::Monthly, false),
            (Repeat::Monthly, true),
            (Repeat::Yearly, false),
            (Repeat::Yearly, true),
        ] {
            for all_day in [false, true] {
                let day = NaiveDate::from_ymd_opt(
                    2024,
                    if repeat == Repeat::Yearly { 2 } else { 1 },
                    if repeat == Repeat::Yearly { 29 } else { 31 },
                )
                .unwrap();
                let mut item = Item::new(Some(day));
                item.title = format!("{repeat:?}-{clamp}-{all_day}");
                let s = item.schedule.as_mut().unwrap();
                s.repeat = repeat;
                s.clamp_missing_day = clamp;
                s.remind = false;
                s.repeat_until = Some(NaiveDate::from_ymd_opt(2027, 3, 1).unwrap());
                if all_day {
                    s.start = day.and_time(NaiveTime::MIN);
                    s.all_day = true;
                    s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                    s.end = Some(s.start + Duration::days(1));
                }
                let dates: Vec<_> = (0..2200)
                    .map(|n| day + Days::new(n))
                    .filter(|d| s.on_day(*d))
                    .map(|d| {
                        d.and_time(s.start.time())
                            .format("%Y%m%dT%H%M%S")
                            .to_string()
                    })
                    .collect();
                expected.push(serde_json::json!({"title":item.title,"dates":dates}));
                items.push(item);
            }
        }
        std::fs::write(
            output.join("finite.ics"),
            ics::export(&items, false).unwrap(),
        )
        .unwrap();
        std::fs::write(
            output.join("expected.json"),
            serde_json::to_vec_pretty(&expected).unwrap(),
        )
        .unwrap();
        if output.join("foreign.ics").exists() {
            let incoming = ics::read(&output.join("foreign.ics")).unwrap();
            let actual: Vec<_> = incoming
                .records
                .iter()
                .map(|item| {
                    let s = item.schedule.as_ref().unwrap();
                    let dates: Vec<_> = (0..2200)
                        .map(|n| s.start.date() + Days::new(n))
                        .filter(|d| s.on_day(*d))
                        .map(|d| {
                            d.and_time(s.start.time())
                                .format("%Y%m%dT%H%M%S")
                                .to_string()
                        })
                        .collect();
                    serde_json::json!({"title":item.title,"dates":dates,"remind":s.remind})
                })
                .collect();
            std::fs::write(
                output.join("foreign-actual.json"),
                serde_json::to_vec_pretty(&actual).unwrap(),
            )
            .unwrap();
        }
        println!(
            "PASS generated finite recurrence export and independent producer import fixtures"
        );
    }
}
