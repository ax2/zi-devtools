use super::*;
use std::path::Path;

impl State {
    pub fn preview_ics(&mut self, importing: bool) {
        self.preview_interval(true);
        self.items[0].title = "秋日旅行 · 本机安排".into();
        self.edit(self.items[0].clone());
        self.focus_editor = false;
        if importing {
            let incoming = self.ics_fixture();
            let mut review = ics_ui::ImportReview::new(incoming, self.items.clone());
            review.update = true;
            review.rebuild();
            self.ics_review = Some(ics_ui::Review::Import(review));
        } else {
            let mut review = ics_ui::ExportReview {
                records: self.items.clone(),
                selected_id: Some(self.items[0].id.clone()),
                selected_only: true,
                completed: false,
                reminders: false,
                text: Ok(String::new()),
                count: 0,
            };
            review.rebuild();
            self.ics_review = Some(ics_ui::Review::Export(review));
        }
    }
    fn ics_fixture(&self) -> ics::Import {
        let mut changed = self.items[0].clone();
        changed.title = "秋日旅行 · 文件中的新安排".into();
        changed.body = "第一天整理路线；第二天拍摄并整理照片。".into();
        let mut extra = changed.clone();
        extra.id = uuid::Uuid::new_v4().to_string();
        extra.title = "旅途后的资料归档".into();
        extra.schedule.as_mut().unwrap().start = NaiveDate::from_ymd_opt(2026, 10, 6)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        extra.schedule.as_mut().unwrap().end = Some(
            NaiveDate::from_ymd_opt(2026, 10, 7)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
        ics::parse(&ics::export(&[changed, extra], false).unwrap()).unwrap()
    }
    pub fn preview_ics_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_ics(false);
                self.ics_review = None;
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-ics-ui-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                let mut item = self.items[0].clone();
                item.revision = 0;
                store::save(&self.path, item).unwrap();
                let mut note = Item::new(None);
                note.title = "不受日历导入影响的备忘".into();
                store::save(&self.path, note).unwrap();
                // Keep the known calendar first for fixture construction.
                let loaded = store::load(&self.path).unwrap();
                self.list_cache.invalidate();
                self.items = loaded
                    .iter()
                    .filter(|i| i.schedule.is_some())
                    .chain(loaded.iter().filter(|i| i.schedule.is_none()))
                    .cloned()
                    .collect();
                let incoming = self.ics_fixture();
                ics::write(
                    &self.path.with_extension("ics"),
                    &ics::export(&incoming.records, false).unwrap(),
                )
                .unwrap();
                self.draft = None;
                self.original = None;
                self.read_ics(self.path.with_extension("ics")).unwrap();
            }
            1 => {
                assert!(self.ics_review.is_none() && self.pending.is_none());
                assert!(
                    store::load(&self.path)
                        .unwrap()
                        .iter()
                        .any(|i| i.title == "秋日旅行 · 本机安排")
                );
                self.read_ics(self.path.with_extension("ics")).unwrap();
            }
            2 => {
                assert!(
                    self.import_ics().is_err(),
                    "updating an existing calendar requires explicit acknowledgment"
                );
            }
            3 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error, "{}", self.message);
                assert!(self.ics_review.is_none());
                let loaded = store::load(&self.path).unwrap();
                assert_eq!(loaded.len(), 3);
                assert!(
                    loaded
                        .iter()
                        .any(|i| i.title == "秋日旅行 · 文件中的新安排")
                );
                assert!(loaded.iter().any(|i| i.title == "旅途后的资料归档"));
                assert!(loaded.iter().any(|i| i.title == "不受日历导入影响的备忘"));
                assert!(
                    loaded
                        .iter()
                        .filter_map(|i| i.schedule.as_ref())
                        .all(|s| !s.remind)
                );
                let incoming = ics::read(&self.path.with_extension("ics")).unwrap();
                let plan = ics_ui::plan(&loaded, &incoming.records, true).unwrap();
                assert_eq!((plan.added, plan.updated, plan.unchanged), (0, 0, 2));
                std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS ICS UI: actual cancel, update selection, overwrite acknowledgment and import; note retained, alarms disabled, UID reimport idempotent"
                );
                return true;
            }
            _ => unreachable!(),
        }
        false
    }
    pub fn preview_ics_interop(&self, output: &Path) {
        std::fs::create_dir_all(output).unwrap();
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
            let date = NaiveDate::from_ymd_opt(
                2024,
                if repeat == Repeat::Yearly { 2 } else { 1 },
                if repeat == Repeat::Yearly { 29 } else { 31 },
            )
            .unwrap();
            let mut item = Item::new(Some(date));
            item.title = format!("{repeat:?}-{clamp}");
            item.body = "中文, 转义; \\ 与换行\n末尾空格  ".into();
            let s = item.schedule.as_mut().unwrap();
            s.repeat = repeat;
            s.clamp_missing_day = clamp;
            s.end = Some(s.start + Duration::hours(1));
            let days: Vec<_> = (0..2200)
                .map(|n| date + Days::new(n))
                .filter(|d| s.on_day(*d))
                .map(|d| {
                    d.and_time(s.start.time())
                        .format("%Y%m%dT%H%M%S")
                        .to_string()
                })
                .collect();
            expected.push(serde_json::json!({"title":item.title,"dates":days,"body":item.body}));
            items.push(item);
        }
        let mut all_day = Item::new(Some(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap()));
        all_day.title = "多日全天".into();
        let s = all_day.schedule.as_mut().unwrap();
        s.start = s.start.date().and_time(NaiveTime::MIN);
        s.all_day = true;
        s.end = Some(s.start + Duration::days(3));
        s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        items.push(all_day);
        std::fs::write(
            output.join("calendar.ics"),
            ics::export(&items, true).unwrap(),
        )
        .unwrap();
        std::fs::write(
            output.join("expected.json"),
            serde_json::to_vec_pretty(&expected).unwrap(),
        )
        .unwrap();
        let foreign = output.join("foreign.ics");
        if foreign.exists() {
            let incoming = ics::read(&foreign).unwrap();
            assert_eq!(incoming.records.len(), 2);
            let meeting = incoming
                .records
                .iter()
                .find(|i| i.title == "来自外部日历的会议")
                .unwrap();
            let expected = NaiveDate::from_ymd_opt(2026, 10, 2)
                .unwrap()
                .and_hms_opt(1, 0, 0)
                .unwrap()
                .and_utc()
                .with_timezone(&Local)
                .naive_local();
            assert_eq!(meeting.schedule.as_ref().unwrap().start, expected);
            assert!(meeting.body.contains("尾部空格  \n\n地点：线上"));
            assert_eq!(
                meeting.calendar_uid.as_deref(),
                Some("foreign-a@calendar.test")
            );
            let monthly = incoming
                .records
                .iter()
                .find(|i| i.title == "每月资料归档")
                .unwrap()
                .schedule
                .as_ref()
                .unwrap();
            assert!(monthly.all_day && monthly.repeat == Repeat::Monthly);
            assert!(!monthly.on_day(NaiveDate::from_ymd_opt(2027, 2, 28).unwrap()));
            assert!(monthly.on_day(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()));
            assert!(
                incoming
                    .records
                    .iter()
                    .all(|i| !i.schedule.as_ref().unwrap().remind)
            );
            std::fs::write(
                output.join("foreign-parsed.json"),
                serde_json::to_vec_pretty(&incoming.records).unwrap(),
            )
            .unwrap();
            println!(
                "PASS independent producer import: UTC conversion, all-day monthly dates, escaped text, original UID and disabled alarms"
            );
        }
        println!(
            "PASS generated independent ICS fixtures: six recurrence rules plus multi-day all-day event"
        );
    }
}
