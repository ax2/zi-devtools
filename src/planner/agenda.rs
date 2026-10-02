use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub index: usize,
    pub at: NaiveDateTime,
}

pub(super) fn rows(
    items: &[Item],
    start: NaiveDate,
    days: u32,
    query: &str,
    done: bool,
) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(schedule) = item.schedule.as_ref() else {
            continue;
        };
        if item.trash
            || (!done && schedule.done)
            || (!query.is_empty()
                && !format!("{} {}", item.title, item.body)
                    .to_lowercase()
                    .contains(&query))
        {
            continue;
        }
        // Include a carry-in once, even when it began before the visible range.
        let midnight = start.and_time(NaiveTime::MIN);
        if days > 0
            && (MIN_YEAR..=MAX_YEAR).contains(&start.year())
            && let Some(at) = midnight
                .checked_sub_signed(Duration::nanoseconds(1))
                .and_then(|before| schedule.latest(before))
            && schedule.end_at(at).is_some_and(|end| end > midnight)
        {
            rows.push(Row { index, at });
        }
        for offset in 0..days.min(30) {
            let Some(day) = start.checked_add_days(Days::new(u64::from(offset))) else {
                break;
            };
            if (MIN_YEAR..=MAX_YEAR).contains(&day.year()) && schedule.on_day(day) {
                rows.push(Row {
                    index,
                    at: day.and_time(schedule.start.time()),
                });
            }
        }
    }
    rows.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then_with(|| items[a.index].title.cmp(&items[b.index].title))
            .then_with(|| items[a.index].id.cmp(&items[b.index].id))
    });
    rows
}

#[derive(PartialEq, Eq)]
struct Key {
    start: NaiveDate,
    days: u32,
    query: String,
    done: bool,
    // Saved records change their revision for every mutation. Preserve ordering
    // here because cached rows refer to indices, not copied record bodies.
    versions: Vec<(String, i64)>,
}
#[derive(Default)]
pub(super) struct Cache {
    key: Option<Key>,
    pub rows: Vec<Row>,
}
impl Cache {
    pub fn refresh(
        &mut self,
        items: &[Item],
        start: NaiveDate,
        days: u32,
        query: &str,
        done: bool,
    ) -> bool {
        let key = Key {
            start,
            days,
            query: query.trim().to_lowercase(),
            done,
            versions: items.iter().map(|i| (i.id.clone(), i.revision)).collect(),
        };
        if self.key.as_ref() == Some(&key) {
            return false;
        }
        self.rows = rows(items, start, days, &key.query, done);
        self.key = Some(key);
        true
    }
}

impl State {
    pub(super) fn agenda_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("今天起").clicked() {
                self.select_date(Local::now().date_naive());
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.jump)
                    .desired_width(110.0)
                    .hint_text("YYYY-MM-DD"),
            );
            if ui.button("设为起点").clicked() {
                match NaiveDate::parse_from_str(self.jump.trim(), "%Y-%m-%d") {
                    Ok(day) if (MIN_YEAR..=MAX_YEAR).contains(&day.year()) => self.select_date(day),
                    _ => {
                        self.message = "起点需要是 1901–2099 年的 YYYY-MM-DD 日期".into();
                        self.error = true;
                    }
                }
            }
            ui.checkbox(&mut self.agenda_done, "包含已完成");
        });
        let end = self
            .selected
            .checked_add_days(Days::new(u64::from(self.agenda_days - 1)))
            .unwrap_or(self.selected)
            .min(NaiveDate::from_ymd_opt(MAX_YEAR, 12, 31).unwrap());
        let changed = self.agenda_cache.refresh(
            &self.items,
            self.selected,
            self.agenda_days,
            &self.query,
            self.agenda_done,
        );
        ui.strong(format!(
            "{} 至 {} · {} 次安排",
            self.selected,
            end,
            self.agenda_cache.rows.len()
        ));
        ui.small("包含范围前开始但尚未结束的安排，每次只列一次。按开始时间排序，搜索筛选此范围；点击进入开始日，编辑重复日程影响整个系列。");
        ui.add_space(6.0);
        if self.agenda_cache.rows.is_empty() {
            ui.label("此范围没有符合条件的日程。可更换日期、关键词或包含已完成记录。");
        }
        let mut open = None;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("planner-agenda")
            .max_height(440.0);
        if changed {
            area = area.vertical_scroll_offset(0.0);
        }
        #[cfg(feature = "ui-preview")]
        {
            self.preview_agenda_rect = None;
        }
        area.show_rows(ui, 72.0, self.agenda_cache.rows.len(), |ui, range| {
            for row_index in range {
                let row = self.agenda_cache.rows[row_index];
                let item = &self.items[row.index];
                let schedule = item.schedule.as_ref().unwrap();
                let response = ui
                    .add_sized(
                        [ui.available_width(), 72.0],
                        egui::Button::new(format!(
                            "{}{}{}\n{}\n{} · {}{}",
                            if item.pinned { "★ " } else { "" },
                            item.title.chars().take(30).collect::<String>(),
                            if item.title.chars().count() > 30 {
                                "…"
                            } else {
                                ""
                            },
                            schedule.range_label(row.at),
                            schedule.rule_label(),
                            if schedule.remind {
                                "提醒开启"
                            } else {
                                "提醒关闭"
                            },
                            if schedule.done { " · 已完成" } else { "" }
                        ))
                        .selected(self.draft.as_ref().is_some_and(|d| d.id == item.id)),
                    )
                    .on_hover_text(format!(
                        "{}\n{}\n系列起点：{}",
                        item.title,
                        schedule.range_label(row.at),
                        schedule.start
                    ));
                #[cfg(feature = "ui-preview")]
                if row_index == 0 {
                    self.preview_agenda_rect = Some(response.rect);
                }
                if response.clicked() {
                    open = Some((item.id.clone(), row.at));
                }
            }
        });
        if let Some((id, at)) = open
            && let Err(error) = self.open_event(&id, at)
        {
            self.message = error.to_string();
            self.error = true;
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_agenda(&mut self, month: bool) {
        self.preview(true, false);
        self.agenda_days = if month { 30 } else { 7 };
        let mut daily = self.items[2].clone();
        daily.id = uuid::Uuid::new_v4().to_string();
        daily.title = "每日整理收件箱与备忘".into();
        let s = daily.schedule.as_mut().unwrap();
        s.start = (self.selected - Duration::days(2))
            .and_hms_opt(8, 30, 0)
            .unwrap();
        s.repeat = Repeat::Daily;
        s.remind = false;
        self.items.push(daily);
        self.draft = None;
        self.original = None;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_agenda_assert(&self) {
        assert_eq!(self.agenda_days, 1);
        assert_eq!(self.selected, NaiveDate::from_ymd_opt(2026, 10, 2).unwrap());
        assert_eq!(self.draft.as_ref().unwrap().title, "每日整理收件箱与备忘");
        assert!(!self.has_unsaved());
        println!(
            "PASS agenda UI: actual first occurrence click opens exact saved series and calendar date without mutation"
        );
    }
}
