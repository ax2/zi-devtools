use super::*;
use backup::{Mode, Review};

fn item_details(ui: &mut egui::Ui, item: &Item) {
    ui.strong(&item.title);
    ui.label(format!(
        "{} · {}",
        if item.trash {
            "回收站"
        } else {
            "正常记录"
        },
        if item.pinned {
            "已置顶"
        } else {
            "未置顶"
        }
    ));
    if let Some(s) = &item.schedule {
        ui.label(format!(
            "{} · {} · {} · 提前 {} 分钟",
            s.range_label(s.start),
            s.rule_label(),
            if s.remind {
                "提醒开启"
            } else {
                "提醒关闭"
            },
            s.minutes
        ));
        if let Some(clock) = s.reminder_time {
            ui.label(format!(
                "全天提醒基准钟点：{}（再减去提前分钟）",
                clock.format("%H:%M")
            ));
        }
        ui.label(if s.done {
            "日程已完成 / 结束"
        } else {
            "日程未完成"
        });
        if let Some(at) = s.handled {
            ui.label(format!("已知晓的日程：{}", at.format("%Y-%m-%d %H:%M")));
        }
        if let Some((at, until)) = s.snooze {
            ui.label(format!(
                "{} 的稍后提醒：{}",
                at.format("%Y-%m-%d %H:%M"),
                DateTime::from_timestamp(until, 0)
                    .unwrap()
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
            ));
        }
    }
    ui.label(&item.body);
}

impl State {
    pub(super) fn backup_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let enabled = self.loaded && self.pending.is_none() && !self.has_unsaved();
            if ui
                .add_enabled(enabled, egui::Button::new("完整备份…"))
                .on_hover_text(
                    "备份全部已保存的备忘、日程、提醒处理状态和回收站；未保存草稿不包含。",
                )
                .clicked()
                && let Err(error) = self.prepare_backup()
            {
                self.message = error.to_string();
                self.error = true;
            }
            #[cfg(windows)]
            if ui
                .add_enabled(enabled, egui::Button::new("恢复备份…"))
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Zi 备忘日程备份", &["json"])
                    .pick_file()
                && let Err(error) = self.read_backup(path)
            {
                self.message = format!("{error:#}");
                self.error = true;
            }
            ui.small("全部已保存的备忘与日程 · 包含回收站 · 不含其他工具数据");
        });
    }
    pub(super) fn backup_ui(&mut self, ctx: &egui::Context) {
        let Some(review) = self.backup_review.as_mut() else {
            return;
        };
        let mut cancel = false;
        let mut restore = false;
        #[cfg(feature = "ui-preview")]
        let mut rects = [None; 4];
        #[cfg(windows)]
        let mut export = false;
        egui::Modal::new(egui::Id::new("planner-backup")).show(ctx, |ui| {
            ui.set_width(570.0_f32.min((ctx.screen_rect().width() - 48.0).max(180.0)));
            match review {
                Review::Export(document) => {
                    ui.heading("完整备份备忘与日程");
                    let events = document.records.iter().filter(|i| i.schedule.is_some()).count();
                    let trash = document.records.iter().filter(|i| i.trash).count();
                    ui.label(format!("共 {} 条：备忘 {} · 日程 {events} · 其中回收站 {trash}", document.records.len(), document.records.len()-events));
                    ui.label("包含正文、置顶、日程重复规则、提醒及已知晓/稍后提醒状态。只包含已保存数据；不包含草稿、其他工具或软件配置。");
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("backup-records").max_height(210.0).show(ui, |ui| {
                        for item in &document.records { ui.push_id(&item.id, |ui| { ui.collapsing(format!("{} · {}", if item.schedule.is_some() { "日程" } else { "备忘" }, item.title), |ui| item_details(ui, item)); }); }
                    });
                    ui.add_space(8.0);
                    ui.label("备份是含全部正文的明文 JSON 文件，请保存在你信任的位置。使用新文件名，已有文件不会覆盖。");
                    ui.horizontal_wrapped(|ui| {
                        cancel = ui.button("取消").clicked();
                        #[cfg(windows)] { export = ui.button("选择路径并保存备份…").clicked(); }
                    });
                }
                Review::Restore(review) => {
                    ui.heading("预览备份恢复");
                    ui.label(format!("备份 {} 条 · 当前 {} 条 · 备份时间 {}", review.document.records.len(), review.current.len(), DateTime::from_timestamp(review.document.created_at,0).unwrap().with_timezone(&Local).format("%Y-%m-%d %H:%M")));
                    let previous = (review.mode, review.reminders);
                    ui.radio_value(&mut review.mode, Mode::KeepCurrent, "合并，保留现有记录（同一记录不覆盖）");
                    ui.radio_value(&mut review.mode, Mode::BackupWins, "合并，冲突记录使用备份内容");
                    let replace_response = ui.radio_value(&mut review.mode, Mode::ReplaceAll, "替换全部，仅保留备份中的记录");
                    #[cfg(feature = "ui-preview")]
                    { rects[1] = Some(replace_response.rect); }
                    let _ = replace_response;
                    ui.checkbox(&mut review.reminders, "同时恢复新增 / 覆盖日程的提醒设置");
                    ui.small("默认关闭这些日程的提醒；勾选后到期提醒可能立即弹出。保留的现有记录不变，时间按本机时区解释。");
                    if previous != (review.mode, review.reminders) { review.rebuild(); }
                    ui.separator();
                    match &review.plan {
                        Ok(plan) => {
                            ui.strong(format!("新增 {} · 覆盖 {} · 移除 {} · 保留 {}", plan.added, plan.updated, plan.removed, plan.kept));
                            let before: HashMap<_,_> = review.current.iter().map(|i| (i.id.as_str(),i)).collect();
                            let after: HashMap<_,_> = plan.records.iter().map(|i| (i.id.as_str(),i)).collect();
                            egui::ScrollArea::vertical().id_salt("restore-changes").max_height(200.0).show(ui, |ui| {
                                for change in &plan.changes {
                                    let old = before.get(change.id.as_str()); let new = after.get(change.id.as_str());
                                    let title = new.or(old).map(|i| i.title.as_str()).unwrap_or_default();
                                    ui.push_id(&change.id, |ui| { ui.collapsing(format!("{} · {}", change.label, title), |ui| {
                                        if let Some(item) = old { ui.label("恢复前"); item_details(ui, item); }
                                        if let Some(item) = new { ui.label("恢复后"); item_details(ui, item); }
                                    }); });
                                }
                            });
                            if plan.changes.is_empty() { ui.label("没有需要变更的记录。"); }
                            let destructive = plan.updated > 0 || plan.removed > 0;
                            if destructive {
                                let response = ui.checkbox(&mut review.confirmed, format!("确认覆盖 {} 条并移除 {} 条；此操作无法撤销", plan.updated, plan.removed));
                                #[cfg(feature = "ui-preview")]
                                { rects[2] = Some(response.rect); }
                                let _ = response;
                            }
                            ui.small("提交前会再次检查本地记录；任何变化都会取消整次恢复。需要保留当前版本时，请先取消并创建完整备份。");
                            ui.horizontal_wrapped(|ui| {
                                let cancel_response = ui.button("取消，不恢复");
                                let restore_response = ui.add_enabled(!plan.changes.is_empty() && (!destructive || review.confirmed), egui::Button::new("确认恢复到本机"));
                                cancel = cancel_response.clicked(); restore = restore_response.clicked();
                                #[cfg(feature = "ui-preview")]
                                { rects[0] = Some(cancel_response.rect); rects[3] = Some(restore_response.rect); }
                            });
                        }
                        Err(error) => { ui.colored_label(ui.visuals().error_fg_color,error); cancel = ui.button("取消").clicked(); }
                    }
                }
            }
        });
        #[cfg(feature = "ui-preview")]
        {
            self.preview_backup_rects = rects;
        }
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.backup_review = None;
            return;
        }
        let mut result = Ok(());
        if restore {
            result = self.restore_backup();
        }
        #[cfg(windows)]
        if export
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Zi 备忘日程备份", &["json"])
                .set_file_name(format!(
                    "Zi-备忘日程-{}.json",
                    Local::now().format("%Y%m%d-%H%M%S")
                ))
                .save_file()
        {
            result = self.save_backup(path);
        }
        if let Err(error) = result {
            self.message = format!("{error:#}");
            self.error = true;
            self.backup_review = None;
        }
    }
}
