use super::*;
use egui::{RichText, vec2};

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_reminder_opened(&self) -> bool {
        !self.alarm_open
            && self.calendar
            && self.alarms.first().is_some_and(|(id, at)| {
                self.draft.as_ref().is_some_and(|draft| {
                    &draft.id == id && draft.schedule.as_ref().unwrap().handled.is_none()
                }) && self.selected == at.date()
            })
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_focus_editor(&mut self) {
        self.focus_editor = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_timer(&mut self) {
        self.pending = None;
        self.saving = None;
        self.loaded = true;
        let mut item = Item::new(Some(Local::now().date_naive()));
        item.title = "原生托盘计时提醒验收".into();
        let s = item.schedule.as_mut().unwrap();
        s.start = Local::now().naive_local() + Duration::seconds(3);
        s.minutes = 0;
        self.replace_items(vec![item]);
        self.shown.clear();
        self.alarms.clear();
        self.last_tick = Instant::now() - std::time::Duration::from_secs(2);
        self.preview_delivered
            .store(false, std::sync::atomic::Ordering::Release);
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(if self.calendar {
                "万年历与日程"
            } else {
                "备忘录"
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(format!("待处理提醒 · {}", self.alarms.len()))
                    .clicked()
                {
                    self.alarm_open = true;
                }
                if ui
                    .add_enabled(
                        self.pending.is_none() && !self.has_unsaved(),
                        egui::Button::new("重新加载"),
                    )
                    .clicked()
                {
                    self.draft = None;
                    self.original = None;
                    self.reload();
                }
            });
        });
        ui.label(
            RichText::new("记录留在本机 · 点击保存后持久保留 · 支持搜索、置顶与回收站恢复").weak(),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.calendar, "备忘录").clicked() {
                self.calendar = false;
                self.query.clear();
                self.trash = false;
            }
            if ui
                .selectable_label(self.calendar, "万年历 / 日程")
                .clicked()
            {
                self.calendar = true;
                self.query.clear();
                self.trash = false;
            }
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("搜索标题与正文…")
                    .desired_width(220.0),
            );
            ui.checkbox(&mut self.trash, "回收站");
            if ui
                .add_enabled(
                    self.pending.is_none() && self.loaded,
                    egui::Button::new(if self.calendar {
                        "+ 新建日程"
                    } else {
                        "+ 新建备忘"
                    }),
                )
                .clicked()
            {
                self.new_draft(self.calendar.then_some(self.selected));
            }
        });
        self.file_toolbar(ui);
        if self.trash {
            let count = self.items.iter().filter(|i| i.trash).count();
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("回收站共 {count} 条，包含备忘与日程。"));
                if ui
                    .add_enabled(
                        self.loaded && self.pending.is_none() && !self.has_unsaved() && count > 0,
                        egui::Button::new("清空回收站…"),
                    )
                    .clicked()
                    && let Err(error) = self.request_purge(true)
                {
                    self.message = error.to_string();
                    self.error = true;
                }
                ui.small("清空范围不受当前搜索、日期或类型筛选影响。");
            });
        }
        if self.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("正在读写本地记录…");
            });
        }
        if !self.message.is_empty() {
            let color = if self.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            ui.colored_label(color, &self.message);
        }
        ui.add_space(10.0);
        if self.calendar {
            ui.label(RichText::new("按本机时区安排。运行或缩到托盘时弹出提醒；完全退出不提醒，重启后补显示。休眠期间不唤醒。重复日程合并显示最近一次错过的提醒。").small().weak());
            if !self.trash {
                ui.horizontal_wrapped(|ui| {
                    for (days, label) in [(1, "月历 / 单日"), (7, "7 天日程"), (30, "30 天日程")]
                    {
                        if ui
                            .selectable_label(!self.week_view && self.agenda_days == days, label)
                            .clicked()
                        {
                            self.week_view = false;
                            self.agenda_days = days;
                        }
                    }
                    if ui.selectable_label(self.week_view, "周视图").clicked() {
                        self.week_view = true;
                    }
                    ui.small(if self.week_view {
                        "周一至周日"
                    } else {
                        "以所选日期为起点"
                    });
                });
            }
            ui.add_space(8.0);
        }
        let width = ui.available_width();
        if self.calendar && self.week_view && !self.trash {
            self.week_ui(ui);
            ui.add_space(12.0);
            let editor = egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .show(ui, |ui| self.editor_ui(ui));
            if self.focus_editor {
                editor.response.scroll_to_me(Some(egui::Align::Min));
            }
        } else if width < 860.0 && self.calendar {
            self.calendar_list_ui(ui);
            ui.add_space(12.0);
            let editor = egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .show(ui, |ui| self.editor_ui(ui));
            if self.focus_editor {
                editor.response.scroll_to_me(Some(egui::Align::Min));
            }
        } else {
            ui.columns(2, |cols| {
                cols[0].set_width((width - 12.0) * 0.5);
                cols[1].set_width((width - 12.0) * 0.5);
                if self.calendar {
                    self.calendar_list_ui(&mut cols[0]);
                } else {
                    self.list_ui(&mut cols[0]);
                }
                egui::Frame::group(cols[1].style())
                    .inner_margin(16.0)
                    .show(&mut cols[1], |ui| self.editor_ui(ui));
            });
        }
        self.focus_editor = false;
        ui.add_space(12.0);
        ui.collapsing("本地保存与容量", |ui| {
            ui.label(format!("{} 条 / 2000 条（含回收站）；正文每条 128 KiB，总内容 32 MiB。", self.items.len()));
            ui.label("内容未加密，不会自动上传。内置完整备份无需退出；手工复制数据库文件前请完全退出。回收站清理释放记录容量，数据库空间由 SQLite 复用，不保证立即缩小或安全擦除。");
            ui.label(self.path.display().to_string());
        });
        self.purge_ui(ui.ctx());
        self.export_ui(ui.ctx());
        self.backup_ui(ui.ctx());
        self.ics_ui(ui.ctx());
    }

    fn calendar_list_ui(&mut self, ui: &mut egui::Ui) {
        if self.agenda_days > 1 && !self.trash {
            self.agenda_ui(ui);
        } else {
            self.calendar_ui(ui);
            self.list_ui(ui);
        }
    }

    fn purge_ui(&mut self, ctx: &egui::Context) {
        let Some(reviewed) = self.purge_review.as_ref() else {
            return;
        };
        let mut cancel = false;
        let mut confirm = false;
        #[cfg(feature = "ui-preview")]
        let mut rects = None;
        egui::Modal::new(egui::Id::new("planner-purge")).show(ctx, |ui| {
            ui.set_width(440.0_f32.min((ctx.screen_rect().width() - 48.0).max(180.0)));
            ui.heading(format!("永久删除 {} 条记录？", reviewed.len()));
            ui.label("仅删除下面列出的回收站记录，包含备忘和日程。此操作无法撤销，之后无法从回收站恢复。");
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("purge-items").max_height(180.0).show(ui, |ui| {
                for item in reviewed {
                    ui.label(format!("{} · {}", if item.schedule.is_some() { "日程" } else { "备忘" }, item.title));
                }
            });
            ui.add_space(8.0);
            ui.small("提交时复核记录是否已修改或恢复；有冲突则整批取消。打开此窗口之后新增的回收站记录不会被删除。");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let cancel_response = ui.button("取消，保留记录");
                let confirm_response = ui.button(egui::RichText::new("确认永久删除").color(ui.visuals().error_fg_color));
                cancel = cancel_response.clicked();
                confirm = confirm_response.clicked();
                #[cfg(feature = "ui-preview")]
                { rects = Some([cancel_response.rect, confirm_response.rect]); }
            });
        });
        #[cfg(feature = "ui-preview")]
        {
            self.preview_purge_rects = rects;
        }
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.purge_review = None;
        } else if confirm && let Err(error) = self.confirm_purge() {
            self.purge_review = None;
            self.message = error.to_string();
            self.error = true;
        }
    }

    fn calendar_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.month.year() > MIN_YEAR || self.month.month() > 1,
                    egui::Button::new("‹"),
                )
                .on_hover_text("上个月")
                .clicked()
            {
                self.month = self.month.pred_opt().unwrap().with_day(1).unwrap();
                self.lunar.clear();
            }
            ui.strong(format!(
                "{} 年 {} 月",
                self.month.year(),
                self.month.month()
            ));
            if ui
                .add_enabled(
                    self.month.year() < MAX_YEAR || self.month.month() < 12,
                    egui::Button::new("›"),
                )
                .on_hover_text("下个月")
                .clicked()
            {
                self.month = self
                    .month
                    .checked_add_months(chrono::Months::new(1))
                    .unwrap();
                self.lunar.clear();
            }
            if ui.button("今天").clicked() {
                self.select_date(Local::now().date_naive());
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.jump)
                    .hint_text("YYYY-MM-DD")
                    .desired_width(120.0),
            );
            if ui.button("跳转").clicked() {
                match NaiveDate::parse_from_str(self.jump.trim(), "%Y-%m-%d") {
                    Ok(date) if (MIN_YEAR..=MAX_YEAR).contains(&date.year()) => {
                        self.select_date(date)
                    }
                    _ => {
                        self.message = "请输入 1901–2099 年的有效日期，例如 2026-10-02".into();
                        self.error = true;
                    }
                }
            }
        });
        ui.add_space(8.0);
        let first =
            self.month - Duration::days(i64::from(self.month.weekday().num_days_from_monday()));
        let cell = ((ui.available_width() - 24.0) / 7.0).max(40.0);
        egui::Grid::new("planner-month")
            .spacing(vec2(4.0, 4.0))
            .show(ui, |ui| {
                for label in ["一", "二", "三", "四", "五", "六", "日"] {
                    ui.add_sized([cell, 20.0], egui::Label::new(RichText::new(label).weak()));
                }
                ui.end_row();
                for n in 0..42 {
                    let date = first + Duration::days(n);
                    let valid = (MIN_YEAR..=MAX_YEAR).contains(&date.year());
                    let lunar = if valid {
                        self.lunar
                            .entry(date)
                            .or_insert_with(|| lunar_day(date))
                            .clone()
                    } else {
                        LunarDay {
                            short: String::new(),
                            full: String::new(),
                        }
                    };
                    let count = self
                        .items
                        .iter()
                        .filter(|i| {
                            !i.trash
                                && i.schedule
                                    .as_ref()
                                    .is_some_and(|s| !s.done && s.covering(date).is_some())
                        })
                        .count();
                    let today = date == Local::now().date_naive();
                    let marker = if count > 0 {
                        " •"
                    } else if today {
                        " 今"
                    } else {
                        ""
                    };
                    let text = RichText::new(format!("{}{marker}\n{}", date.day(), lunar.short))
                        .size(12.0);
                    let text = if date.month() != self.month.month() {
                        text.weak()
                    } else {
                        text
                    };
                    let response = ui.add_enabled(
                        valid,
                        egui::Button::new(text)
                            .selected(date == self.selected)
                            .min_size(vec2(cell, 49.0)),
                    );
                    if response
                        .on_hover_text(format!("{date}\n{}\n{count} 条未完成日程", lunar.full))
                        .clicked()
                    {
                        self.selected = date;
                        self.jump = date.to_string();
                    }
                    if n % 7 == 6 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(8.0);
        let day = self
            .lunar
            .entry(self.selected)
            .or_insert_with(|| lunar_day(self.selected));
        ui.strong(self.selected.to_string());
        ui.label(&day.full);
        ui.label(
            RichText::new("1901–2099 · 节日为传统日历信息，不代表官方放假/调休")
                .small()
                .weak(),
        );
        ui.separator();
    }

    pub(super) fn select_date(&mut self, date: NaiveDate) {
        self.selected = date;
        self.month = date.with_day(1).unwrap();
        self.jump = date.to_string();
        self.lunar.clear();
    }

    fn list_ui(&mut self, ui: &mut egui::Ui) {
        if !self.calendar && self.list_sort == listing::ListSort::Updated {
            self.list_sort = listing::ListSort::Default;
        }
        let query = self.query.trim().to_lowercase();
        ui.horizontal_wrapped(|ui| {
            let pin = ui.checkbox(&mut self.list_pinned, "只看置顶");
            #[cfg(feature = "ui-preview")]
            {
                self.preview_list_rects[0] = Some(pin.rect);
            }
            let _ = pin;
            let sort = egui::ComboBox::from_id_salt("planner-list-sort")
                .selected_text(self.list_sort.label(self.calendar))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.list_sort,
                        listing::ListSort::Default,
                        if self.calendar {
                            "时间顺序"
                        } else {
                            "最近更新"
                        },
                    );
                    if self.calendar {
                        ui.selectable_value(
                            &mut self.list_sort,
                            listing::ListSort::Updated,
                            "最近更新",
                        );
                    }
                    let title = ui.selectable_value(
                        &mut self.list_sort,
                        listing::ListSort::Title,
                        "标题顺序",
                    );
                    #[cfg(feature = "ui-preview")]
                    {
                        self.preview_list_rects[2] = Some(title.rect);
                    }
                    let _ = title;
                });
            #[cfg(feature = "ui-preview")]
            {
                self.preview_list_rects[1] = Some(sort.response.rect);
            }
            let _ = sort;
            ui.small("置顶优先");
        });
        let matches = self.listed_indices();
        ui.strong(format!(
            "{} · {} 条",
            if self.trash {
                if self.calendar {
                    "日程回收站 · 当前筛选"
                } else {
                    "备忘回收站 · 当前筛选"
                }
            } else if self.calendar && !query.is_empty() {
                "所有日期的搜索结果"
            } else if self.calendar {
                "当日日程"
            } else {
                "全部备忘"
            },
            matches.len()
        ));
        egui::ScrollArea::vertical()
            .id_salt("planner-list")
            .max_height(if self.calendar { 240.0 } else { 530.0 })
            .show_rows(ui, 54.0, matches.len().max(1), |ui, rows| {
                #[cfg(feature = "ui-preview")]
                {
                    self.preview_list_rows = rows.len();
                    self.preview_list_start = rows.start;
                }
                if matches.is_empty() {
                    ui.add_space(18.0);
                    ui.label(RichText::new("这里还没有记录。新建一条，或更换搜索条件。").weak());
                    return;
                }
                for row in rows {
                    let item = &self.items[matches[row]];
                    let (id, title, pin, updated, schedule) = (
                        item.id.clone(),
                        item.title.clone(),
                        item.pinned,
                        item.updated,
                        item.schedule.clone(),
                    );
                    let detail = if let Some(s) = schedule {
                        let occurrence = if query.is_empty() && !self.trash {
                            s.covering(self.selected).unwrap_or(s.start)
                        } else {
                            s.start
                        };
                        format!(
                            "{}  ·  {}{}",
                            s.range_label(occurrence),
                            s.rule_label(),
                            if s.done { " · 已完成" } else { "" }
                        )
                    } else {
                        Local
                            .timestamp_opt(updated, 0)
                            .single()
                            .map(|d| d.format("更新于 %m-%d %H:%M").to_string())
                            .unwrap_or_default()
                    };
                    let selected = self.draft.as_ref().is_some_and(|i| i.id == id);
                    let row_response = ui
                        .push_id(&id, |ui| {
                            listing::record_row(
                                ui,
                                &format!("{}{title}", if pin { "★ " } else { "" }),
                                &detail,
                                selected,
                            )
                        })
                        .inner;
                    #[cfg(feature = "ui-preview")]
                    if row == 0 {
                        self.preview_list_rects[3] = Some(row_response.rect);
                    }
                    if row_response.clicked()
                        && self.pending.is_none()
                        && self.may_leave()
                        && let Some(item) = self.items.iter().find(|i| i.id == id).cloned()
                    {
                        self.edit(item);
                    }
                }
            });
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut item) = self.draft.clone() else {
            ui.add_space(35.0);
            ui.heading("留住想法，安排下一步");
            ui.label("打开一条记录，或新建一条。备忘录可以复制为日程，调整日期与提醒后保存。");
            ui.add_space(35.0);
            return;
        };
        let dirty = self.has_unsaved();
        let mut trash = false;
        let mut purge = false;
        let mut export = false;
        ui.heading(if item.schedule.is_some() {
            "编辑日程"
        } else {
            "编辑备忘录"
        });
        ui.label(
            RichText::new(if item.revision == 0 {
                "新记录 · 尚未保存"
            } else if dirty {
                "有未保存的修改"
            } else {
                "已保存"
            })
            .weak(),
        );
        ui.add_space(10.0);
        ui.add_enabled_ui(self.pending.is_none() && self.loaded, |ui| {
            ui.label("标题");
            let title_control=ui.add(
                egui::TextEdit::singleline(&mut item.title)
                    .desired_width(f32::INFINITY)
                    .hint_text("写下一个清晰的标题")
                    .char_limit(120),
            );
            #[cfg(feature="ui-preview")] {self.preview_title_rect=Some((title_control.rect,ui.clip_rect()));}
            let _=title_control;
            ui.add_space(8.0);
            if let Some(s) = &mut item.schedule {
                let was_all_day = s.all_day;
                let all_day_control = ui.checkbox(&mut s.all_day, "全天事件");
                #[cfg(feature = "ui-preview")]
                { self.preview_interval_rect = Some((all_day_control.rect, ui.clip_rect())); }
                let _ = all_day_control;
                if s.all_day != was_all_day {
                    if s.all_day {
                        if s.end.is_none() {
                            self.end_date_text = self.date_text.clone();
                        } else if self.end_time_text == "00:00" {
                            if let Ok(day) = NaiveDate::parse_from_str(&self.end_date_text, "%Y-%m-%d") {
                                self.end_date_text = day.pred_opt().unwrap_or(day).to_string();
                            }
                        }
                        s.end = Some(s.start);
                    } else {
                        self.end_time_text = "23:59".into();
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("开始日期");
                    ui.add(egui::TextEdit::singleline(&mut self.date_text).desired_width(106.0));
                    if !s.all_day {
                        ui.label("时间");
                        ui.add(egui::TextEdit::singleline(&mut self.time_text).desired_width(52.0));
                    }
                });
                if !s.all_day {
                    let mut has_end = s.end.is_some();
                    if !has_end && let Ok(start) = NaiveDateTime::parse_from_str(&format!("{} {}", self.date_text.trim(), self.time_text.trim()), "%Y-%m-%d %H:%M") {
                        let end = start + Duration::hours(1);
                        self.end_date_text = end.date().to_string();
                        self.end_time_text = end.format("%H:%M").to_string();
                    }
                    if ui.checkbox(&mut has_end, "设置结束时间").changed() {
                        s.end = has_end.then_some(s.start);
                    }
                }
                if s.end.is_some() || s.all_day {
                    ui.horizontal(|ui| {
                        ui.label(if s.all_day { "结束日期" } else { "结束日期 / 时间" });
                        ui.add(egui::TextEdit::singleline(&mut self.end_date_text).desired_width(106.0));
                        if s.all_day { ui.small("包含当天"); }
                        else { ui.add(egui::TextEdit::singleline(&mut self.end_time_text).desired_width(52.0)); }
                    });
                }
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("planner-repeat")
                        .selected_text(s.repeat.label())
                        .show_ui(ui, |ui| {
                            for repeat in [Repeat::Once, Repeat::Daily, Repeat::Weekly, Repeat::Monthly, Repeat::Yearly] {
                                ui.selectable_value(&mut s.repeat, repeat, repeat.label());
                            }
                        });
                    ui.checkbox(
                        &mut s.done,
                        if s.repeat == Repeat::Once {
                            "已完成"
                        } else {
                            "结束整个重复日程"
                        },
                    );
                });
                if matches!(s.repeat, Repeat::Monthly | Repeat::Yearly) {
                    let policy = ui.checkbox(&mut s.clamp_missing_day, "没有对应日期时，改用当月最后一天");
                    #[cfg(feature = "ui-preview")]
                    { self.preview_recurrence_rects[0] = Some((policy.rect, ui.clip_rect())); }
                    #[cfg(not(feature = "ui-preview"))]
                    let _ = policy;
                    ui.small(if s.clamp_missing_day {
                        "从原始日号计算：1 月 31 日 → 2 月末 → 3 月 31 日；闰日遇平年用 2 月 28 日。"
                    } else {
                        "没有对应日期则跳过：31 日跳过短月；2 月 29 日仅在闰年安排。"
                    });
                    ui.small("公历规则；不等于农历生日。更改规则后提醒处理状态会重置，需保存才生效。");
                } else {
                    s.clamp_missing_day = false;
                }
                if s.repeat != Repeat::Once {
                    let mut limited = s.repeat_until.is_some();
                    let control = ui.checkbox(&mut limited, "设置重复截止日期");
                    #[cfg(feature = "ui-preview")]
                    { self.preview_cutoff_rects[0] = Some((control.rect, ui.clip_rect())); }
                    if control.changed() {
                        if limited {
                            let start = NaiveDate::parse_from_str(self.date_text.trim(), "%Y-%m-%d").unwrap_or(s.start.date());
                            let day = start.checked_add_signed(Duration::days(30)).unwrap_or(start).min(NaiveDate::from_ymd_opt(MAX_YEAR, 12, 31).unwrap());
                            s.repeat_until = Some(day);
                            self.repeat_until_text = day.to_string();
                        } else { s.repeat_until = None; }
                    }
                    if limited {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("重复至");
                            let input = ui.add(egui::TextEdit::singleline(&mut self.repeat_until_text).desired_width(106.0).hint_text("YYYY-MM-DD"));
                            #[cfg(feature = "ui-preview")]
                            { self.preview_cutoff_rects[1] = Some((input.rect, ui.clip_rect())); }
                            let _ = input;
                            ui.small("包含当天开始的安排");
                        });
                        ui.small("只限制开始日期，最后一次可跨过截止日。修改截止不会重置已知晓提醒。");
                    }
                } else { s.repeat_until = None; }
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.remind, "弹出提醒");
                    ui.add_enabled(
                        s.remind,
                        egui::DragValue::new(&mut s.minutes)
                            .range(0..=10080)
                            .prefix("提前 ")
                            .suffix(" 分钟"),
                    );
                });
                ui.label(
                    RichText::new("0 分钟表示准时提醒；不重复日程也可设为一次性闹钟。")
                        .small()
                        .weak(),
                );
                if s.all_day {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("提醒钟点");
                        ui.add_enabled(s.remind, egui::TextEdit::singleline(&mut self.time_text).desired_width(52.0));
                        ui.small("以开始当天为准，再减去提前分钟");
                    });
                }
                ui.add_space(8.0);
            }
            ui.label(if item.schedule.is_some() {
                "详情 / 地点 / 准备事项"
            } else {
                "正文"
            });
            egui::ScrollArea::vertical()
                .id_salt("planner-body")
                .max_height(if item.schedule.is_some() {
                    215.0
                } else {
                    340.0
                })
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut item.body)
                            .desired_width(f32::INFINITY)
                            .desired_rows(12)
                            .char_limit(MAX_BODY)
                            .hint_text("支持多行文字；保存前可随时修改…"),
                    );
                });
            ui.horizontal(|ui| {
                ui.checkbox(&mut item.pinned, "置顶");
                ui.label(
                    RichText::new(format!("{} 字符", item.body.chars().count()))
                        .small()
                        .weak(),
                );
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("复制全文").clicked() {
                    ui.ctx()
                        .copy_text(format!("{}\n\n{}", item.title, item.body));
                }
                export = ui
                    .add_enabled(!item.trash, egui::Button::new("导出正文…"))
                    .clicked();
            });
            ui.add_space(10.0);
            trash = ui
                .add_enabled(
                    item.revision > 0 && !dirty,
                    egui::Button::new(if item.trash {
                        "从回收站恢复"
                    } else {
                        "移入回收站"
                    }),
                )
                .clicked();
            if item.trash {
                ui.label("回收站中的日程不会提醒。恢复后，未处理的过期提醒会再次显示。");
                purge = ui
                    .add_enabled(
                        item.revision > 0 && !dirty,
                        egui::Button::new("永久删除这条记录…"),
                    )
                    .clicked();
            }
        });
        self.draft = Some(item.clone());
        if trash {
            item.trash = !item.trash;
            self.launch(Some(item));
        } else if purge {
            if let Err(error) = self.request_purge(false) {
                self.message = error.to_string();
                self.error = true;
            }
        } else if export {
            if let Err(error) = self.review_export() {
                self.message = error.to_string();
                self.error = true;
            }
        }
    }

    pub fn reminder_ui(&mut self, ctx: &egui::Context) -> bool {
        if !self.alarm_open {
            return false;
        }
        let mut open = true;
        let mut snooze_minutes = self.snooze_minutes;
        #[cfg(feature = "ui-preview")]
        let mut snooze_rects = self.preview_snooze_rects;
        let mut action = None;
        let mut selected = None;
        #[cfg(feature = "ui-preview")]
        let mut open_rect = None;
        egui::Window::new("日程提醒")
            .id(egui::Id::new("planner-alarms"))
            .open(&mut open)
            .collapsible(false)
            .default_width(440.0)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label("提醒会保留到你处理。关闭此窗口可稍后从日程页继续查看。");
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        if self.alarms.is_empty() {
                            ui.label("没有待处理提醒。");
                        }
                        for (id, at) in &self.alarms {
                            let Some(item) = self.items.iter().find(|i| &i.id == id) else {
                                continue;
                            };
                            ui.push_id(id, |ui| {
                                ui.separator();
                                ui.strong(&item.title);
                                if let Some(s) = &item.schedule {
                                    ui.label(s.range_label(*at));
                                    if s.all_day {
                                        ui.small(format!(
                                            "提醒基准：{} · 本机时间",
                                            s.reminder_at(*at).format("%Y-%m-%d %H:%M")
                                        ));
                                    }
                                }
                                let open_response = ui.add_enabled(
                                    self.pending.is_none(),
                                    egui::Button::new("打开日程 →"),
                                );
                                #[cfg(feature = "ui-preview")]
                                {
                                    open_rect = Some(open_response.rect);
                                }
                                if open_response.clicked() {
                                    selected = Some((id.clone(), *at));
                                }
                                let editing = self.draft.as_ref().is_some_and(|i| &i.id == id)
                                    && self.has_unsaved();
                                ui.add_enabled_ui(self.pending.is_none() && !editing, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("已知晓").clicked() {
                                            action = Some((id.clone(), *at, reminder_actions::ReminderAction::Acknowledge));
                                        }
                                        let combo = egui::ComboBox::from_id_salt("snooze-duration")
                                            .selected_text(match snooze_minutes { 60=>"1 小时".into(),120=>"2 小时".into(),1440=>"1 天".into(),_=>format!("{snooze_minutes} 分钟") })
                                            .show_ui(ui, |ui| {
                                                for (minutes, label) in [(5,"5 分钟"),(10,"10 分钟"),(30,"30 分钟"),(60,"1 小时"),(120,"2 小时"),(1440,"1 天")] {
                                                    let option = ui.selectable_value(&mut snooze_minutes, minutes, label);
                                                    #[cfg(feature="ui-preview")] if minutes == 30 { snooze_rects[1] = Some(option.rect); }
                                                    let _ = option;
                                                }
                                            });
                                        #[cfg(feature="ui-preview")] {snooze_rects[0] = Some(combo.response.rect);}
                                        let _ = combo;
                                        let snooze = ui.button("稍后提醒").on_hover_text("从点击时刻开始延后，只处理这次提醒；其他重复次数仍按原规则提醒。");
                                        #[cfg(feature="ui-preview")] {snooze_rects[2] = Some(snooze.rect);}
                                        if snooze.clicked() {
                                            action = Some((id.clone(), *at, reminder_actions::ReminderAction::Snooze(snooze_minutes)));
                                        }
                                    });
                                });
                                if editing {
                                    ui.label("此日程正在编辑，请先保存或放弃编辑。");
                                }
                            });
                        }
                    });
                if self.error {
                    ui.colored_label(ui.visuals().error_fg_color, &self.message);
                }
            });
        self.alarm_open = open;
        self.snooze_minutes = snooze_minutes;
        #[cfg(feature = "ui-preview")]
        {
            self.preview_snooze_rects = snooze_rects;
        }
        #[cfg(feature = "ui-preview")]
        {
            self.preview_open_reminder_rect = open_rect;
        }
        if let Some((id, at)) = selected {
            match self.open_event(&id, at) {
                Ok(()) => return true,
                Err(error) => {
                    self.message = error.to_string();
                    self.error = true;
                }
            }
        }
        if let Some((id, at, action)) = action
            && let Err(error) = self.respond_reminder(&id, at, action, Local::now())
        {
            self.message = error.to_string();
            self.error = true;
        }
        false
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_trash(&mut self, confirm: bool) {
        self.preview(false, false);
        self.items[0].trash = true;
        self.items[2].trash = true;
        self.edit(self.items[0].clone());
        self.trash = true;
        self.query.clear();
        if confirm {
            self.request_purge(true).unwrap();
        }
        self.focus_editor = false;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_purge_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview_trash(false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-purge-ui-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                for mut item in self.items.clone() {
                    item.revision = 0;
                    store::save(&self.path, item).unwrap();
                }
                self.replace_items(store::load(&self.path).unwrap());
                self.edit(self.items.iter().find(|i| i.trash).unwrap().clone());
                self.request_purge(true).unwrap();
            }
            1 => {
                assert!(self.purge_review.is_none() && self.pending.is_none());
                assert_eq!(store::load(&self.path).unwrap().len(), 3);
                self.request_purge(true).unwrap();
            }
            2 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && self.purge_review.is_none());
                assert_eq!(self.items.len(), 1);
                assert!(!self.items[0].trash && self.draft.is_none());
                assert_eq!(store::load(&self.path).unwrap(), self.items);
                std::fs::remove_file(&self.path).unwrap();
                std::fs::remove_dir(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS trash UI: actual cancel retains all records; actual confirm deletes only reviewed trash, clears selection and preserves live record"
                );
            }
            _ => unreachable!(),
        }
        true
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview(&mut self, calendar: bool, alarm: bool) {
        // The capture app uses an isolated config directory; never writes user records.
        self.pending = None;
        self.saving = None;
        self.deleting = None;
        self.purge_review = None;
        self.file_operation = false;
        self.export_review = None;
        self.backup_review = None;
        self.list_pinned = false;
        self.list_sort = listing::ListSort::default();
        self.agenda_days = 1;
        self.agenda_done = false;
        self.agenda_cache = Default::default();
        self.trash = false;
        self.loaded = true;
        self.list_cache.invalidate();
        self.items.clear();
        self.calendar = calendar;
        self.select_date(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap());
        let mut memo = Item::new(None);
        memo.title = "网站图片优化清单".into();
        memo.body = "所有落地页图片统一使用 WebP。\n\n□ 检查手机与桌面两种尺寸\n□ 兼顾文字清晰度与图片体积\n□ 发布前核对首页返回链接\n\n完成检查后，把这条备忘转成下次复查的日程。".into();
        memo.pinned = true;
        memo.revision = 1;
        memo.updated = Local::now().timestamp();
        self.items.push(memo.clone());
        let mut another = memo.clone();
        another.id = uuid::Uuid::new_v4().to_string();
        another.title = "下周工具开发想法".into();
        another.pinned = false;
        another.body = "备忘、日历、文件互通的下一步。".into();
        self.items.push(another);
        let mut event = Item::new(Some(self.selected));
        event.title = "整理本周项目与备忘".into();
        event.body = "检查未完成事项，整理本周笔记。\n准备下周开发安排。".into();
        event.revision = 1;
        event.schedule.as_mut().unwrap().repeat = Repeat::Weekly;
        self.items.push(event.clone());
        self.edit(if calendar { event.clone() } else { memo });
        self.message.clear();
        self.alarms.clear();
        self.alarm_open = alarm;
        if alarm {
            self.alarms.push((event.id, event.schedule.unwrap().start));
        }
        self.last_tick = Instant::now() + std::time::Duration::from_secs(120);
        self.focus_editor = false;
    }
}
