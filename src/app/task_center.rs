use super::*;
use crate::tasks::Phase;

impl DevToolsApp {
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

    fn observe_tasks(&mut self) {
        self.tasks.observe(
            self.data_state
                .parse_job
                .snapshot("data", "数据解析", false),
        );
        self.tasks.observe(self.data_state.join_job().snapshot(
            "csv-merge",
            "表格合并与关联",
            true,
        ));
        self.tasks
            .observe(self.file_state.job.snapshot("files", "批量文件校验", true));
    }
    pub(super) fn poll_tasks(&mut self, ctx: &egui::Context) {
        self.observe_tasks();
        let before = [
            self.data_state.parse_job.phase,
            self.data_state.join_job().phase,
            self.file_state.job.phase,
        ];
        self.data_state.poll();
        self.file_state.poll();
        let after = [
            self.data_state.parse_job.phase,
            self.data_state.join_job().phase,
            self.file_state.job.phase,
        ];
        for (index, (before, after)) in before.into_iter().zip(after).enumerate() {
            if before.active() && !after.active() {
                self.toast = Some((
                    format!(
                        "{} · {}",
                        ["数据解析", "表格合并", "文件校验"][index],
                        after.label()
                    ),
                    Instant::now(),
                ));
            }
        }
        self.observe_tasks();
        if after.into_iter().any(Phase::active) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    pub(super) fn tasks_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("后台任务中心");
        ui.label("数据解析、表格合并与文件校验 · 切换页面后继续收取结果");
        ui.small("保留本次运行最近 50 条记录；这里只显示任务状态，不保存输入、文件路径或凭据。其他工具任务暂在各自页面查看。");
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
                    ui.push_id((row.key, row.generation), |ui| {
                        egui::Frame::new()
                            .fill(self.colors.card)
                            .corner_radius(10)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(row.title);
                                    let color = match row.phase {
                                        Phase::Failed => self.colors.red,
                                        Phase::Done => self.colors.green,
                                        Phase::Cancelled | Phase::Cancelling => self.colors.amber,
                                        _ => self.colors.accent,
                                    };
                                    ui.colored_label(color, row.phase.label());
                                    ui.label(format!("{:.1} 秒", row.elapsed.as_secs_f32()));
                                    if ui.button("打开工具").clicked() {
                                        open = Some(row.key);
                                    }
                                    if row.cancellable
                                        && row.phase == Phase::Running
                                        && ui.button("取消任务").clicked()
                                    {
                                        cancel = Some(row.key);
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
        if let Some(key) = cancel {
            match key {
                "csv-merge" => self.data_state.cancel_join(),
                "files" => self.file_state.cancel_task(),
                _ => {}
            }
        }
        if let Some(key) = open
            && let Some(entry) = self.entries("").into_iter().find(|e| e.id == key)
        {
            self.open_entry(&entry);
        }
    }
}
