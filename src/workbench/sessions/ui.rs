use super::*;
use std::time::Duration;

impl Workspace {
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let active = self.active_id().to_owned();
        let pending = self.operation_pending();
        card(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.strong("工作实例");
                let mut selected = active.clone();
                egui::ComboBox::from_id_salt("data-instance")
                    .width(210.0)
                    .selected_text(&self.instances[self.active].name)
                    .show_ui(ui, |ui| {
                        for i in &self.instances {
                            ui.selectable_value(
                                &mut selected,
                                i.id.clone(),
                                format!(
                                    "{}{}",
                                    i.name,
                                    if i.state.busy() { " · 运行中" } else { "" }
                                ),
                            );
                        }
                    });
                if selected != active {
                    let _ = self.select(&selected);
                }
                if ui
                    .add_enabled(self.instances.len() < 16, egui::Button::new("＋ 新实例"))
                    .clicked()
                {
                    let name = format!("未命名数据 {}", self.instances.len() + 1);
                    if let Err(e) = self.create(&name) {
                        self.status = e.to_string();
                    }
                }
                if ui
                    .add_enabled(!pending, egui::Button::new("已保存实例…"))
                    .clicked()
                {
                    self.library_open = true;
                    self.list_saved();
                }
                if ui
                    .add_enabled(!pending && !self.busy(), egui::Button::new("关闭当前"))
                    .clicked()
                {
                    if self.has_content() {
                        self.close_confirm = Some(self.active_id().into());
                    } else {
                        let id = self.active_id().to_owned();
                        if let Err(e) = self.close(&id, false) {
                            self.status = e.to_string();
                        }
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("名称");
                ui.add(
                    egui::TextEdit::singleline(&mut self.instances[self.active].name)
                        .char_limit(80)
                        .desired_width(220.0),
                );
                if ui
                    .add_enabled(!pending && !self.busy(), primary(ui, "保存快照…"))
                    .clicked()
                {
                    self.save_confirm = true;
                    self.save_copy = false;
                }
                if ui
                    .add_enabled(!pending && !self.busy(), egui::Button::new("另存副本…"))
                    .clicked()
                {
                    self.save_confirm = true;
                    self.save_copy = true;
                }
                if let Some((_, version)) = &self.instances[self.active].saved {
                    ui.small(format!("已保存第 {version} 版"));
                } else {
                    ui.small("临时实例");
                }
                if pending {
                    ui.spinner();
                    ui.small("读写实例库中…");
                }
            });
            ui.small("切换会保留本次工作，后台解析和合并继续运行。修改不会自动保存，退出前请保存需要保留的内容。");
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
        });
        ui.add_space(10.0);
        let id = self.active_id().to_owned();
        ui.push_id(id, |ui| self.deref_mut().ui(ui, ctx));
        self.dialogs(ctx);
        if self.operation_pending() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.save_confirm {
            let mut save = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("save-data-instance")).show(ctx,|ui|{
                ui.set_max_width(520.0);ui.heading(if self.save_copy{"另存工作实例副本"}else{"保存工作实例快照"});
                ui.label(format!("保存“{}”的原始输入、当前表格、导出文本、筛选、文件路径、合并草稿、转换预览/撤销内容，以及工具流程与表格流水线的步骤。",self.instances[self.active].name));
                ui.label("流程运行结果、待确认审核、目标文件路径和写入授权不随流程恢复。恢复后请重新运行或预览，需要写文件时重新确认。");
                ui.label("内容保存在本机，未加密；恢复不会运行命令、读取原文件或重新解析。保存后继续编辑，需要再次保存才能保留修改。");
                ui.small(format!("位置：{}",self.store.path.display()));
                ui.small("最多 256 项 / 128 MiB，单项最多 64 MiB；更新会替换此保存项的旧快照。需要保留旧版请选择另存副本。");
                ui.horizontal(|ui|{cancel=ui.button("取消").clicked();let button=ui.add_enabled(!self.operation_pending()&&!self.busy(),primary(ui,"确认保存到本机"));
                    #[cfg(feature="ui-preview")] ui.ctx().data_mut(|data|data.insert_temp(egui::Id::new("workspace-save-confirm"),button.rect));
                    save=button.clicked();});
            });
            if cancel {
                self.save_confirm = false;
            }
            if save {
                self.save_confirm = false;
                if let Err(e) = self.save_active(self.save_copy) {
                    self.status = e.to_string();
                }
            }
        }
        if let Some(id) = self.close_confirm.clone() {
            let mut close = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("close-data-instance")).show(ctx,|ui|{
                ui.heading("关闭这个工作实例？");
                if let Some(instance)=self.instances.iter().find(|i|i.id==id){ui.label(&instance.name);}
                ui.label("未保存的输入和修改会丢失；已经保存的快照保留在本机。运行中的实例需等待或取消后再关闭。");
                ui.horizontal(|ui|{cancel=ui.button("返回保存").clicked();close=ui.button("放弃未保存修改并关闭").clicked();});
            });
            if cancel {
                self.close_confirm = None;
            }
            if close {
                self.close_confirm = None;
                if let Err(e) = self.close(&id, true) {
                    self.status = e.to_string();
                }
            }
        }
        if self.library_open {
            let mut open = self.library_open;
            let mut load = None;
            let mut delete = None;
            let mut refresh = false;
            egui::Window::new("已保存的工作实例")
                .open(&mut open)
                .default_width(640.0)
                .min_width(480.0)
                .show(ctx, |ui| {
                    ui.label("打开为独立实例；已经打开的保存项会回到原实例，保留其当前修改。");
                    ui.small(format!(
                        "{} 项 · 内容 {:.1} MiB / 128 MiB",
                        self.entries.len(),
                        self.entries.iter().map(|e| e.bytes).sum::<i64>() as f64 / 1048576.0
                    ));
                    ui.small(format!("{}", self.store.path.display()));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.library_query)
                                .hint_text("按名称查找已保存实例")
                                .char_limit(160),
                        );
                        refresh = ui
                            .add_enabled(!self.operation_pending(), egui::Button::new("刷新"))
                            .clicked();
                    });
                    if self.operation_pending() {
                        ui.spinner();
                    }
                    if !self.status.is_empty() {
                        ui.label(&self.status);
                    }
                    let query = self.library_query.trim().to_lowercase();
                    let rows: Vec<_> = self
                        .entries
                        .iter()
                        .filter(|e| e.name.to_lowercase().contains(&query))
                        .collect();
                    if rows.is_empty() {
                        ui.label("没有匹配的保存项。回到实例，选择保存快照。");
                    }
                    egui::ScrollArea::vertical().max_height(360.0).show_rows(
                        ui,
                        48.0,
                        rows.len(),
                        |ui, range| {
                            for row in &rows[range] {
                                ui.push_id(&row.id, |ui| {
                                    ui.horizontal(|ui| {
                                        let title_width = (ui.available_width() - 240.0).max(100.0);
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(title_width, 22.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(&row.name).strong(),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(&row.name);
                                            },
                                        );
                                        ui.small(format!(
                                            "第 {} 版 · {:.0} KiB",
                                            row.revision,
                                            row.bytes as f64 / 1024.0
                                        ));
                                        if ui
                                            .add_enabled(
                                                !self.operation_pending(),
                                                egui::Button::new("打开"),
                                            )
                                            .clicked()
                                        {
                                            load = Some(row.id.clone());
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.operation_pending(),
                                                egui::Button::new("删除…"),
                                            )
                                            .clicked()
                                        {
                                            delete = Some((*row).clone());
                                        }
                                    });
                                    let saved_time =
                                        chrono::DateTime::parse_from_rfc3339(&row.updated)
                                            .map(|t| {
                                                t.with_timezone(&chrono::Local)
                                                    .format("%Y-%m-%d %H:%M")
                                                    .to_string()
                                            })
                                            .unwrap_or_else(|_| "保存时间未知".into());
                                    ui.small(format!("保存于 {saved_time}"));
                                });
                            }
                        },
                    );
                });
            self.library_open = open;
            if refresh {
                self.list_saved();
            }
            if let Some(id) = load {
                self.load_saved(id);
            }
            if delete.is_some() {
                self.delete_confirm = delete;
            }
        }
        if let Some(entry) = self.delete_confirm.clone() {
            let mut delete = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("delete-data-snapshot")).show(ctx, |ui| {
                ui.heading("删除保存的快照？");
                ui.label(format!(
                    "删除“{}”第 {} 版。本操作不可撤销；原始文件和已打开的实例不受影响。",
                    entry.name, entry.revision
                ));
                ui.horizontal(|ui| {
                    cancel = ui.button("取消").clicked();
                    delete = ui
                        .add_enabled(!self.operation_pending(), egui::Button::new("删除此保存项"))
                        .clicked();
                });
            });
            if cancel {
                self.delete_confirm = None;
            }
            if delete {
                self.delete_confirm = None;
                let store = self.store.clone();
                let (tx, rx) = mpsc::channel();
                self.receiver = Some(rx);
                std::thread::spawn(move || {
                    let result = store.delete(&entry.id, entry.revision);
                    let _ = tx.send(Reply::Deleted {
                        id: entry.id,
                        result,
                    });
                });
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_flow_save_prepare(&mut self) {
        self.preview_workflow();
        self.save_confirm = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_flow_disk_restore(&mut self) {
        assert!(!self.operation_pending());
        let saved = self.instances[self.active]
            .saved
            .clone()
            .expect("actual database save completed");
        let live = self.active_id().to_owned();
        self.close(&live, true).unwrap();
        self.load_saved(saved.0);
    }
}
