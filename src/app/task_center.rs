use super::*;
use crate::tasks::Phase;

impl DevToolsApp {
    pub(super) fn open_task_result(
        &mut self,
        key: &str,
        instance: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(id) = instance {
            self.data_state.select(id)?;
        }
        let key = if key == "sqlite-export" {
            self.data_state.show_sqlite_export();
            "data"
        } else {
            key
        };
        let entry = self
            .entries("")
            .into_iter()
            .find(|e| e.id == key)
            .ok_or_else(|| anyhow::anyhow!("任务入口不可用"))?;
        self.open_entry(&entry);
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_tasks(&mut self) {
        self.tasks = Default::default();
        let mut job = crate::tasks::Job::default();
        job.begin();
        job.finish(Phase::Done, "250 行 / 6 列");
        self.tasks.observe(job.snapshot("data", "数据解析", false));
        job.begin();
        job.finish(Phase::Failed, "打开合并工具查看详情");
        self.tasks
            .observe(job.snapshot("csv-merge", "表格合并与关联", true));
        job.begin();
        job.progress = Some(0.42);
        self.tasks
            .observe(job.snapshot("files", "批量文件校验", true));
        job.begin();
        job.cancelling();
        self.tasks
            .observe(job.snapshot("csv-merge", "表格合并与关联", true));
    }

    fn task_snapshots(&self) -> Vec<crate::tasks::Row> {
        let mut rows = self.data_state.snapshots();
        rows.extend(self.file_state.job.snapshot("files", "批量文件校验", true));
        rows
    }
    pub(super) fn observe_tasks(&mut self) {
        for row in self.task_snapshots() {
            self.tasks.observe(Some(row));
        }
    }
    pub(super) fn poll_tasks(&mut self, ctx: &egui::Context) {
        self.observe_tasks();
        let before = self.task_snapshots();
        self.data_state.poll();
        self.file_state.poll();
        let after = self.task_snapshots();
        for row in &after {
            if !row.phase.active()
                && before.iter().any(|r| {
                    r.key == row.key
                        && r.instance == row.instance
                        && r.generation == row.generation
                        && r.phase.active()
                })
            {
                self.toast = Some((
                    format!(
                        "{} · {} · {}",
                        row.instance_name.as_deref().unwrap_or("文件校验"),
                        row.title,
                        row.phase.label()
                    ),
                    Instant::now(),
                ));
            }
        }
        self.observe_tasks();
        if after.iter().any(|r| r.phase.active()) || self.data_state.operation_pending() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub(super) fn tasks_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("后台任务中心");
        ui.label("数据解析、表格合并与文件校验 · 切换页面后继续收取结果");
        ui.small("保留本次运行最近 50 条记录；这里显示任务状态和实例名称，不保存正文或凭据。其他工具任务暂在各自页面查看。");
        let active = self.tasks.rows.iter().filter(|r| r.phase.active()).count();
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!(
                "{active} 项运行中 · {} 条记录",
                self.tasks.rows.len()
            ));
            if ui.button("清除已结束记录").clicked() {
                self.tasks.clear_finished();
            }
        });
        ui.separator();
        let mut open = None;
        let mut cancel = None;
        egui::ScrollArea::vertical()
            .id_salt("task-history")
            .show(ui, |ui| {
                if self.tasks.rows.is_empty() {
                    ui.add_space(24.0);
                    ui.label("尚无任务。解析一份数据或开始文件校验，状态会出现在这里。");
                }
                let mut rows: Vec<_> = self.tasks.rows.iter().rev().collect();
                rows.sort_by_key(|row| !row.phase.active());
                for row in rows {
                    ui.push_id((row.key, &row.instance, row.generation), |ui| {
                        egui::Frame::new()
                            .fill(self.colors.card)
                            .corner_radius(10)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(row.title);
                                    if let Some(name) = &row.instance_name {
                                        ui.label(name);
                                    }
                                    let color = match row.phase {
                                        Phase::Failed => self.colors.red,
                                        Phase::Done => self.colors.green,
                                        Phase::Cancelled | Phase::Cancelling => self.colors.amber,
                                        _ => self.colors.accent,
                                    };
                                    ui.colored_label(color, row.phase.label());
                                    ui.label(format!("{:.1} 秒", row.elapsed.as_secs_f32()));
                                    if ui.button("打开工具").clicked() {
                                        open = Some((row.key, row.instance.clone()));
                                    }
                                    if row.cancellable
                                        && row.phase == Phase::Running
                                        && ui.button("取消任务").clicked()
                                    {
                                        cancel =
                                            Some((row.key, row.instance.clone(), row.generation));
                                    }
                                });
                                if row.phase.active() {
                                    if let Some(progress) = row.progress {
                                        ui.add(
                                            egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                                                .show_percentage(),
                                        );
                                    } else {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(if row.phase == Phase::Cancelling {
                                                "等待任务结束…"
                                            } else {
                                                "处理中；该任务不提供百分比进度"
                                            });
                                        });
                                    }
                                }
                                if !row.summary.is_empty() {
                                    ui.label(&row.summary);
                                }
                            });
                        ui.add_space(8.0);
                    });
                }
            });
        if let Some((key, instance, generation)) = cancel {
            match (key, instance) {
                ("csv-merge", Some(id)) => self.data_state.cancel(&id, generation),
                ("sqlite-export", Some(id)) => self.data_state.cancel_sqlite(&id, generation),
                ("files", _) => self.file_state.cancel_task(),
                _ => {}
            }
        }
        if let Some((key, instance)) = open {
            if let Err(error) = self.open_task_result(key, instance.as_deref()) {
                self.toast = Some((error.to_string(), Instant::now()));
            }
        }
    }
}
