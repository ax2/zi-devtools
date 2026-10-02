use super::*;

pub(super) struct ImportReview {
    pub incoming: ics::Import,
    pub current: Vec<Item>,
    pub update: bool,
    pub confirmed: bool,
    pub plan: std::result::Result<Plan, String>,
}
pub(super) struct Plan {
    pub records: Vec<Item>,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub unchanged: usize,
}
impl ImportReview {
    pub(super) fn new(incoming: ics::Import, current: Vec<Item>) -> Self {
        let plan = plan(&current, &incoming.records, false).map_err(|e| e.to_string());
        Self {
            incoming,
            current,
            update: false,
            confirmed: false,
            plan,
        }
    }
    pub(super) fn rebuild(&mut self) {
        self.confirmed = false;
        self.plan =
            plan(&self.current, &self.incoming.records, self.update).map_err(|e| e.to_string());
    }
}
fn same_event(a: &Item, b: &Item) -> bool {
    let (Some(a_s), Some(b_s)) = (&a.schedule, &b.schedule) else {
        return false;
    };
    a.title == b.title
        && a.body == b.body
        && a_s.start == b_s.start
        && a_s.end == b_s.end
        && a_s.all_day == b_s.all_day
        && a_s.repeat == b_s.repeat
        && a_s.clamp_missing_day == b_s.clamp_missing_day
        && a_s.last_repeat_day() == b_s.last_repeat_day()
}
pub(super) fn plan(current: &[Item], incoming: &[Item], update: bool) -> Result<Plan> {
    backup::validate_records(incoming)?;
    let mut result = Plan {
        records: current.to_vec(),
        added: 0,
        updated: 0,
        skipped: 0,
        unchanged: 0,
    };
    let positions: HashMap<_, _> = current
        .iter()
        .enumerate()
        .map(|(index, i)| (i.id.as_str(), index))
        .collect();
    for item in incoming {
        if let Some(&index) = positions.get(item.id.as_str()) {
            let old = &current[index];
            ensure!(
                old.schedule.is_some(),
                "导入 UID 与现有备忘冲突，未写入任何记录"
            );
            ensure!(
                old.calendar_uid
                    .as_ref()
                    .is_none_or(|uid| Some(uid) == item.calendar_uid.as_ref()),
                "导入标识与本机日历 UID 不一致，未写入任何记录"
            );
            if same_event(old, item) {
                result.unchanged += 1;
            } else if update {
                let mut replacement = item.clone();
                replacement.pinned = old.pinned;
                replacement.trash = old.trash;
                replacement.schedule.as_mut().unwrap().done = old.schedule.as_ref().unwrap().done;
                result.records[index] = replacement;
                result.updated += 1;
            } else {
                result.skipped += 1;
            }
        } else {
            result.records.push(item.clone());
            result.added += 1;
        }
    }
    backup::validate_records(&result.records)?;
    Ok(result)
}
pub(super) struct ExportReview {
    pub records: Vec<Item>,
    pub selected_id: Option<String>,
    pub selected_only: bool,
    pub completed: bool,
    pub reminders: bool,
    pub text: std::result::Result<String, String>,
    pub count: usize,
}
impl ExportReview {
    fn selected(&self) -> Vec<Item> {
        self.records
            .iter()
            .filter(|i| !self.selected_only || self.selected_id.as_ref() == Some(&i.id))
            .filter(|i| self.completed || !i.schedule.as_ref().unwrap().done)
            .cloned()
            .collect()
    }
    pub(super) fn rebuild(&mut self) {
        let selected = self.selected();
        self.count = selected.len();
        self.text = ics::export(&selected, self.reminders).map_err(|e| e.to_string());
    }
}
pub(super) enum Review {
    Import(ImportReview),
    Export(ExportReview),
}

impl State {
    fn ics_available(&self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none() && !self.has_unsaved(),
            "请先保存或放弃当前编辑，并等待读写完成"
        );
        ensure!(
            self.ics_review.is_none()
                && self.backup_review.is_none()
                && self.export_review.is_none()
                && self.purge_review.is_none(),
            "请先关闭其他确认窗口"
        );
        Ok(())
    }
    pub(super) fn read_ics(&mut self, path: PathBuf) -> Result<()> {
        self.ics_available()?;
        let database = self.path.clone();
        self.start_file(move || {
            let incoming = ics::read(&path)?;
            let current = store::load(&database)?;
            Ok(Reply::IcsReady(Box::new(Review::Import(
                ImportReview::new(incoming, current),
            ))))
        });
        Ok(())
    }
    pub(super) fn import_ics(&mut self) -> Result<()> {
        ensure!(
            self.pending.is_none() && !self.has_unsaved(),
            "请先保存编辑，并等待读写完成"
        );
        let Some(Review::Import(review)) = self.ics_review.as_ref() else {
            anyhow::bail!("请先预览 ICS 导入");
        };
        let plan = review
            .plan
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?;
        let count = plan.added + plan.updated;
        ensure!(count > 0, "没有需要导入的变化");
        ensure!(
            plan.updated == 0 || review.confirmed,
            "请明确确认更新同 UID 日程，更新条目的提醒将关闭"
        );
        let expected = review.current.clone();
        let records = plan.records.clone();
        let path = self.path.clone();
        self.ics_review = None;
        self.start_file(move || {
            store::restore(&path, &expected, &records).map(|items| Reply::IcsImported(items, count))
        });
        Ok(())
    }
    pub(super) fn prepare_ics(&mut self) -> Result<()> {
        self.ics_available()?;
        let path = self.path.clone();
        let selected_id = self
            .draft
            .as_ref()
            .filter(|i| i.schedule.is_some() && !i.trash)
            .map(|i| i.id.clone());
        self.start_file(move || {
            let records = store::load(&path)?
                .into_iter()
                .filter(|i| i.schedule.is_some() && !i.trash)
                .collect();
            let mut review = ExportReview {
                records,
                selected_only: selected_id.is_some(),
                selected_id,
                completed: false,
                reminders: false,
                text: Ok(String::new()),
                count: 0,
            };
            review.rebuild();
            Ok(Reply::IcsReady(Box::new(Review::Export(review))))
        });
        Ok(())
    }
    pub(super) fn save_ics(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            self.pending.is_none() && !self.has_unsaved(),
            "请先保存编辑，并等待读写完成"
        );
        let Some(Review::Export(review)) = self.ics_review.as_ref() else {
            anyhow::bail!("请先预览 ICS 导出");
        };
        let text = review
            .text
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?
            .clone();
        let count = review.count;
        self.ics_review = None;
        self.start_file(move || {
            ics::write(&path, &text)?;
            Ok(Reply::IcsSaved(path, count))
        });
        Ok(())
    }
    pub(super) fn ics_buttons(&mut self, ui: &mut egui::Ui) {
        if !self.calendar {
            return;
        }
        let enabled = self.loaded && self.pending.is_none() && !self.has_unsaved();
        ui.horizontal_wrapped(|ui| {
            #[cfg(windows)]
            if ui
                .add_enabled(enabled, egui::Button::new("导入 ICS 日历…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("ICS 日历", &["ics"])
                    .pick_file()
                && let Err(e) = self.read_ics(path)
            {
                self.message = format!("{e:#}");
                self.error = true;
            }
            if ui
                .add_enabled(enabled, egui::Button::new("导出日程 ICS…"))
                .clicked()
                && let Err(e) = self.prepare_ics()
            {
                self.message = format!("{e:#}");
                self.error = true;
            }
            ui.small("与其他日历交换安排 · 先预览再确认");
        });
    }
    pub(super) fn ics_ui(&mut self, ctx: &egui::Context) {
        let Some(review) = self.ics_review.as_mut() else {
            return;
        };
        let mut cancel = false;
        let mut import = false;
        let mut save = false;
        #[cfg(feature = "ui-preview")]
        let mut rects = [None; 4];
        egui::Modal::new(egui::Id::new("planner-ics")).show(ctx,|ui| {
            ui.set_width(600.0_f32.min((ctx.screen_rect().width()-48.0).max(180.0)));
            match review {
                Review::Import(review)=>{
                    ui.heading("预览 ICS 日历导入");
                    ui.label(format!("文件包含 {} 条日程；确认前不会写入本机。",review.incoming.records.len()));
                    ui.label("新增 / 更新条目的提醒关闭。更新保留本机置顶、完成和回收站状态；知晓及延后状态重置。其他备忘与日程保留。");
                    let update_control=ui.checkbox(&mut review.update,"更新同 UID 日程（默认只添加新日程）");
                    if update_control.changed() {review.rebuild();}
                    #[cfg(feature="ui-preview")] {rects[1]=Some(update_control.rect);}
                    match &review.plan {
                        Ok(plan)=>{
                            ui.strong(format!("新增 {} · 更新 {} · 内容相同 {} · 保留本机差异 {}",plan.added,plan.updated,plan.unchanged,plan.skipped));
                            egui::ScrollArea::vertical().id_salt("ics-import-preview").max_height(230.0).show(ui,|ui| {
                                for item in &review.incoming.records {
                                    ui.push_id(&item.id,|ui| {ui.collapsing(&item.title,|ui| {
                                        if let Some(old)=review.current.iter().find(|old|old.id==item.id) {ui.label("本机现有"); super::backup_ui::item_details(ui,old);}
                                        ui.label("文件中的安排"); super::backup_ui::item_details(ui,item);
                                    });});
                                }
                                for notice in &review.incoming.notices {ui.label(notice);}
                            });
                            if plan.updated>0 {
                                let response=ui.checkbox(&mut review.confirmed,format!("确认更新 {} 条，并关闭这些条目的提醒",plan.updated));
                                #[cfg(feature="ui-preview")] {rects[2]=Some(response.rect);}
                                let _=response;
                            }
                            ui.small("按 UID 识别同一日程，同标题可并存。提交时检查本机是否变化；任何冲突会取消整次导入。");
                            ui.horizontal_wrapped(|ui| {
                                let a=ui.button("取消，不导入"); cancel=a.clicked();
                                let b=ui.add_enabled(plan.added+plan.updated>0 && (plan.updated==0 || review.confirmed),egui::Button::new("确认导入到本机"));import=b.clicked();
                                #[cfg(feature="ui-preview")] {rects[0]=Some(a.rect);rects[3]=Some(b.rect);}
                            });
                        }
                        Err(e)=>{ui.colored_label(ui.visuals().error_fg_color,e);cancel=ui.button("取消").clicked();}
                    }
                }
                Review::Export(review)=>{
                    ui.heading("导出 ICS 日历");
                    let previous=(review.selected_only,review.completed,review.reminders);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_enabled_ui(review.selected_id.is_some(),|ui| {ui.radio_value(&mut review.selected_only,true,"当前日程");});
                        ui.radio_value(&mut review.selected_only,false,"全部日程（不受日期 / 搜索筛选影响）");
                    });
                    ui.checkbox(&mut review.completed,"包含已完成日程");
                    ui.checkbox(&mut review.reminders,"同时导出已开启的弹出提醒（目标日历可能弹出）");
                    if previous!=(review.selected_only,review.completed,review.reminders) {review.rebuild();}
                    ui.label("日期、时间、标题、详情与重复规则会写入文件。时间不附时区，按目标日历的当地钟表解释；全天事件保持日期。重复遵守各条目的截止日期，最多至 2099 年末。");
                    ui.small("不导出回收站；完成、置顶、知晓 / 延后状态不在 ICS 中，需完整保留时使用 JSON 备份。地点 / 链接已作为详情文本保留。");
                    match &review.text {
                        Ok(text)=>{
                            ui.strong(format!("{} 条日程 · {} 字节 · 仅创建新文件",review.count,text.len()));
                            egui::ScrollArea::vertical().id_salt("ics-export-preview").max_height(220.0).show(ui,|ui| {
                                for item in review.selected() {ui.push_id(&item.id,|ui| {ui.collapsing(&item.title,|ui|super::backup_ui::item_details(ui,&item));});}
                            });
                            ui.horizontal_wrapped(|ui| {cancel=ui.button("取消").clicked();save=ui.button("选择路径并导出…").clicked();});
                        }
                        Err(e)=>{ui.colored_label(ui.visuals().error_fg_color,e);cancel=ui.button("取消").clicked();}
                    }
                }
            }
        });
        #[cfg(feature = "ui-preview")]
        {
            self.preview_ics_rects = rects;
        }
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.ics_review = None;
            return;
        }
        let mut result = Ok(());
        if import {
            result = self.import_ics();
        }
        #[cfg(windows)]
        if save
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("ICS 日历", &["ics"])
                .set_file_name("Zi-日程.ics")
                .save_file()
        {
            result = self.save_ics(path);
        }
        #[cfg(not(windows))]
        let _ = save;
        if let Err(e) = result {
            self.message = format!("{e:#}");
            self.error = true;
        }
    }
}
