use super::*;

fn start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

fn adjacent(day: NaiveDate, direction: i64) -> Option<NaiveDate> {
    let lower = NaiveDate::from_ymd_opt(MIN_YEAR, 1, 1).unwrap();
    let upper = NaiveDate::from_ymd_opt(MAX_YEAR, 12, 31).unwrap();
    let target = day
        .checked_add_signed(Duration::days(direction * 7))?
        .clamp(lower, upper);
    (start(target) != start(day)).then_some(target)
}
fn overlaps(schedule: &Schedule, at: NaiveDateTime, day: NaiveDate) -> bool {
    if !(MIN_YEAR..=MAX_YEAR).contains(&day.year()) {
        return false;
    }
    let midnight = day.and_time(NaiveTime::MIN);
    let next = midnight + Duration::days(1);
    schedule
        .end_at(at)
        .map_or(at.date() == day, |end| at < next && end > midnight)
}

#[derive(Default)]
pub(super) struct Cache {
    agenda: agenda::Cache,
    columns: [Vec<agenda::Row>; 7],
}
impl Cache {
    fn refresh(&mut self, items: &[Item], selected: NaiveDate, query: &str, done: bool) -> bool {
        let monday = start(selected);
        if !self.agenda.refresh(items, monday, 7, query, done) {
            return false;
        }
        for (offset, column) in self.columns.iter_mut().enumerate() {
            let day = monday + Duration::days(offset as i64);
            *column = self
                .agenda
                .rows
                .iter()
                .copied()
                .filter(|row| overlaps(items[row.index].schedule.as_ref().unwrap(), row.at, day))
                .collect();
            column.sort_by_key(|row| !items[row.index].schedule.as_ref().unwrap().all_day);
        }
        true
    }
}

impl State {
    fn open_week_event(&mut self, id: &str, at: NaiveDateTime, day: NaiveDate) -> Result<()> {
        let query = self.query.clone();
        self.open_event(id, at)?;
        self.week_view = true;
        self.select_date(day);
        self.query = query;
        Ok(())
    }
    pub(super) fn week_ui(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        ui.horizontal_wrapped(|ui| {
            for (index, direction, label) in [(0, -1, "‹ 上周"), (1, 1, "下周 ›")] {
                let target = adjacent(self.selected, direction);
                let response = ui.add_enabled(target.is_some(), egui::Button::new(label));
                #[cfg(feature = "ui-preview")]
                {
                    self.preview_week_rects[index] = Some((response.rect, ui.clip_rect()));
                }
                #[cfg(not(feature = "ui-preview"))]
                let _ = index;
                if response.clicked() {
                    select = target;
                }
            }
            if ui.button("本周 / 今天").clicked() {
                select = Some(Local::now().date_naive());
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.jump)
                    .desired_width(110.0)
                    .hint_text("YYYY-MM-DD"),
            );
            if ui.button("跳转到周").clicked() {
                match NaiveDate::parse_from_str(self.jump.trim(), "%Y-%m-%d") {
                    Ok(day) if (MIN_YEAR..=MAX_YEAR).contains(&day.year()) => select = Some(day),
                    _ => {
                        self.error = true;
                        self.message = "日期需要是1901–2099年的YYYY-MM-DD格式".into();
                    }
                }
            }
            ui.checkbox(&mut self.agenda_done, "包含已完成");
        });
        if let Some(day) = select {
            self.select_date(day);
        }
        let monday = start(self.selected);
        let changed =
            self.week_cache
                .refresh(&self.items, self.selected, &self.query, self.agenda_done);
        ui.strong(format!(
            "{} 至 {} · {} 次安排",
            monday,
            monday + Duration::days(6),
            self.week_cache.agenda.rows.len()
        ));
        ui.small("搜索仅筛选本周。全天优先，跨天安排在覆盖的每天显示；点击打开原日程，重复日程编辑整个系列。选择日期后可在顶部新建日程。");
        // Reserve frame strokes and rounding as well as the actual column gaps.
        let spacing = 6.0 * ui.spacing().item_spacing.x + 28.0;
        let cell_width = ((ui.available_width() - spacing) / 7.0).max(140.0);
        if cell_width * 7.0 + spacing > ui.available_width() {
            ui.small("横向滚动查看其他日期，每列也可上下滚动。");
        }
        let mut open = None;
        let mut day_selection = None;
        #[cfg(feature = "ui-preview")]
        {
            self.preview_week_rects[2] = None;
        }
        egui::ScrollArea::horizontal()
            .id_salt("planner-week-horizontal")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for offset in 0..7 {
                        let day = monday + Duration::days(offset as i64);
                        let valid = (MIN_YEAR..=MAX_YEAR).contains(&day.year());
                        egui::Frame::group(ui.style())
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_width(cell_width - 16.0);
                                    let label = format!(
                                        "{} · {}",
                                        ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                                            [offset],
                                        day.format("%m-%d")
                                    );
                                    let date = ui.add_enabled(
                                        valid,
                                        egui::Button::new(label).selected(self.selected == day),
                                    );
                                    if date.clicked() {
                                        day_selection = Some(day);
                                    }
                                    ui.small(if !valid {
                                        "超出支持范围".into()
                                    } else if day == Local::now().date_naive() {
                                        "今天".into()
                                    } else {
                                        format!("{} 次安排", self.week_cache.columns[offset].len())
                                    });
                                    ui.separator();
                                    let column = &self.week_cache.columns[offset];
                                    let mut area = egui::ScrollArea::vertical()
                                        .id_salt(("planner-week-column", offset))
                                        .max_height(290.0)
                                        .auto_shrink([false, false]);
                                    if changed {
                                        area = area.vertical_scroll_offset(0.0);
                                    }
                                    area.show_rows(ui, 54.0, column.len().max(1), |ui, range| {
                                        if column.is_empty() {
                                            ui.small(if valid {
                                                "暂无安排"
                                            } else {
                                                "不可选择"
                                            });
                                            return;
                                        }
                                        for index in range {
                                            let row = column[index];
                                            let item = &self.items[row.index];
                                            let schedule = item.schedule.as_ref().unwrap();
                                            let detail = if schedule.all_day {
                                                "全天".into()
                                            } else if row.at.date() != day {
                                                "跨天延续".into()
                                            } else {
                                                row.at.format("%H:%M").to_string()
                                            };
                                            let response = listing::record_row(
                                                ui,
                                                &format!(
                                                    "{}{}",
                                                    if item.pinned { "★ " } else { "" },
                                                    item.title
                                                ),
                                                &detail,
                                                self.draft
                                                    .as_ref()
                                                    .is_some_and(|draft| draft.id == item.id),
                                            )
                                            .on_hover_text(format!(
                                                "{}\n{}\n{}{}",
                                                item.title,
                                                schedule.range_label(row.at),
                                                schedule.rule_label(),
                                                if schedule.done { " · 已完成" } else { "" }
                                            ));
                                            #[cfg(feature = "ui-preview")]
                                            if offset == 0 && index == 0 {
                                                self.preview_week_rects[2] =
                                                    Some((response.rect, ui.clip_rect()));
                                            }
                                            if response.clicked() {
                                                open = Some((item.id.clone(), row.at, day));
                                            }
                                        }
                                    });
                                });
                            });
                    }
                });
            });
        if let Some(day) = day_selection {
            self.select_date(day);
        }
        if let Some((id, at, day)) = open
            && let Err(error) = self.open_week_event(&id, at, day)
        {
            self.message = error.to_string();
            self.error = true;
        }
    }
}

#[cfg(feature = "ui-preview")]
impl State {
    pub fn preview_week(&mut self) {
        self.preview_agenda(false);
        let mut carry = self.items[2].clone();
        carry.id = uuid::Uuid::new_v4().to_string();
        carry.title = "资料整理与团队复盘 · 跨天".into();
        let s = carry.schedule.as_mut().unwrap();
        s.start = NaiveDate::from_ymd_opt(2026, 9, 27)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        s.end = Some(s.start + Duration::days(2));
        s.all_day = true;
        s.reminder_time = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        s.repeat = Repeat::Once;
        s.remind = false;
        let mut items = self.items.clone();
        items.push(carry);
        self.replace_items(items);
        self.week_view = true;
        self.focus_editor = false;
    }
    pub fn preview_week_smoke(&mut self, phase: u8) {
        if phase == 0 {
            self.preview_week();
            self.path = std::env::temp_dir()
                .join(format!("zi-week-{}", uuid::Uuid::new_v4()))
                .join("planner.sqlite3");
            for mut item in self.items.clone() {
                item.revision = 0;
                store::save(&self.path, item).unwrap();
            }
            self.replace_items(store::load(&self.path).unwrap());
        } else {
            assert!(self.week_view && self.calendar);
            assert_eq!(self.selected, NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
            let item = self.draft.as_ref().unwrap();
            assert_eq!(item.title, "资料整理与团队复盘 · 跨天");
            assert_eq!(
                item.schedule.as_ref().unwrap().start.date(),
                NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
            );
            assert!(!self.has_unsaved());
            assert_eq!(store::load(&self.path).unwrap(), self.items);
            std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
            println!(
                "PASS week UI: next/previous week then actual carry-in card click, selected visible day retained, original schedule and database unchanged"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::tests::{date, event, fixture, wait_state};

    #[test]
    fn week_navigation_clamps_supported_years_without_empty_extra_weeks() {
        for (day, monday) in [
            ("2026-10-02 00:00", "2026-09-28 00:00"),
            ("2026-10-04 00:00", "2026-09-28 00:00"),
            ("1901-01-01 00:00", "1900-12-31 00:00"),
        ] {
            assert_eq!(start(date(day).date()), date(monday).date());
        }
        assert!(adjacent(date("1901-01-01 00:00").date(), -1).is_none());
        assert!(adjacent(date("2099-12-31 00:00").date(), 1).is_none());
        let day = date("2026-10-02 00:00").date();
        assert_eq!(adjacent(adjacent(day, 1).unwrap(), -1), Some(day));
    }
    #[test]
    fn columns_include_carry_in_with_exclusive_end_and_filter_repeating_series() {
        let mut item = event();
        item.title = "跨天合成安排".into();
        let s = item.schedule.as_mut().unwrap();
        s.start = date("2026-09-27 21:00");
        s.end = Some(date("2026-09-29 00:00"));
        s.repeat = Repeat::Weekly;
        let mut daily = event();
        daily.title = "每日合成".into();
        daily.schedule.as_mut().unwrap().start = date("2026-09-28 09:00");
        daily.schedule.as_mut().unwrap().repeat = Repeat::Daily;
        let mut hidden = daily.clone();
        hidden.id = uuid::Uuid::new_v4().to_string();
        hidden.trash = true;
        let mut completed = daily.clone();
        completed.id = uuid::Uuid::new_v4().to_string();
        completed.schedule.as_mut().unwrap().done = true;
        let items = vec![item, daily, hidden, completed];
        let day = date("2026-10-02 00:00").date();
        let mut cache = Cache::default();
        cache.refresh(&items, day, "", false);
        assert_eq!(
            cache.columns[0].iter().filter(|row| row.index == 0).count(),
            1
        );
        assert!(!cache.columns[1].iter().any(|row| row.index == 0));
        assert!(cache.columns[6].iter().any(|row| row.index == 0));
        assert!(
            cache
                .columns
                .iter()
                .all(|col| col.iter().any(|row| row.index == 1))
        );
        assert!(
            cache
                .columns
                .iter()
                .all(|col| col.iter().all(|row| row.index < 2))
        );
        cache.refresh(&items, day, "跨天", false);
        assert_eq!(cache.agenda.rows.len(), 2);
        cache.refresh(&items, day, "每日", true);
        assert!(cache.columns.iter().all(|col| col.len() == 2));
    }
    #[test]
    fn year_edges_and_same_revision_reload_refresh_columns() {
        let mut item = event();
        item.schedule.as_mut().unwrap().start = date("2099-12-31 09:00");
        let mut state = State::new(fixture());
        state.replace_items(vec![item.clone()]);
        state
            .week_cache
            .refresh(&state.items, date("2099-12-31 00:00").date(), "", false);
        assert!(state.week_cache.columns.iter().skip(4).all(Vec::is_empty));
        assert!(state.week_cache.columns[3].iter().any(|r| r.index == 0));
        item.schedule.as_mut().unwrap().start = date("2099-12-30 09:00");
        state.replace_items(vec![item]);
        state
            .week_cache
            .refresh(&state.items, date("2099-12-31 00:00").date(), "", false);
        assert!(state.week_cache.columns[3].is_empty());
        assert_eq!(state.week_cache.columns[2].len(), 1);
    }
    #[test]
    fn week_click_preserves_series_date_query_and_dirty_draft_guards() {
        let path = fixture();
        let mut state = State::new(path.clone());
        wait_state(&mut state);
        let mut item = event();
        item.schedule.as_mut().unwrap().start = date("2026-09-27 21:00");
        state.edit(item);
        state.save_draft();
        wait_state(&mut state);
        let saved = store::load(&path).unwrap();
        let original = saved[0].clone();
        state.query = "合成".into();
        state
            .open_week_event(
                &original.id,
                date("2026-09-27 21:00"),
                date("2026-09-28 00:00").date(),
            )
            .unwrap();
        assert!(state.week_view);
        assert_eq!(state.query, "合成");
        assert_eq!(state.selected, date("2026-09-28 00:00").date());
        assert_eq!(state.draft, Some(original.clone()));
        assert!(!state.has_unsaved());
        assert_eq!(store::load(&path).unwrap(), saved);
        state.draft.as_mut().unwrap().body = "未保存".into();
        state
            .open_week_event(
                &original.id,
                original.schedule.as_ref().unwrap().start,
                state.selected,
            )
            .unwrap();
        assert_eq!(state.draft.as_ref().unwrap().body, "未保存");
        let other = event();
        state.items.push(other.clone());
        assert!(
            state
                .open_week_event(&other.id, other.schedule.unwrap().start, state.selected)
                .is_err()
        );
        assert_eq!(state.draft.as_ref().unwrap().body, "未保存");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
