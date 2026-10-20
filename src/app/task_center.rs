use super::*;
use crate::tasks::{Phase, RunId};

impl DevToolsApp {
    pub(super) fn open_task_result(
        &mut self,
        key: &str,
        instance: Option<&str>,
    ) -> anyhow::Result<()> {
        if key == "mcp-operation" {
            let entry = self
                .entries("")
                .into_iter()
                .find(|entry| entry.id == "mcp-inspector")
                .ok_or_else(|| anyhow::anyhow!("MCP 协议调试台不可用"))?;
            self.open_entry(&entry);
            return Ok(());
        }
        if key == "http-request" {
            let tab_id = instance
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or_else(|| anyhow::anyhow!("HTTP 请求记录缺少有效的标签编号"))?;
            if !self.http_state.select_task_tab(tab_id) {
                self.http_state.pane = crate::dev_tools::HttpPane::Request;
                self.http_state.message = "原请求标签已关闭；已打开 HTTP 请求工作台".to_owned();
            }
            self.page = Page::Http;
            self.visit("http");
            return Ok(());
        }
        if key == "network-diagnostic" {
            if let Some(name) = instance.and_then(|instance_id| {
                self.tasks
                    .rows
                    .iter()
                    .find(|row| row.key == key && row.instance.as_deref() == Some(instance_id))
                    .and_then(|row| row.instance_name.as_deref())
            }) {
                self.network_state.task_pane = match name {
                    "DNS 解析" => crate::network_tools::NetworkPane::Dns,
                    "TCP 连通性" => crate::network_tools::NetworkPane::Tcp,
                    _ => self.network_state.task_pane,
                };
            }
            self.network_state.pane = self.network_state.task_pane;
            self.page = Page::Network;
            self.visit("network");
            return Ok(());
        }
        if matches!(key, "plugin-execution" | "plugin-model-discovery") {
            let id = instance.ok_or_else(|| anyhow::anyhow!("插件任务缺少来源工具"))?;
            let entry = self
                .entries("")
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| anyhow::anyhow!("对应插件工具已不可用"))?;
            self.open_entry(&entry);
            return Ok(());
        }
        if let Some(id) = instance {
            self.data_state.select(id)?;
        }
        let key = if key == "sqlite-export" {
            self.data_state.show_sqlite_export();
            "data"
        } else if key == "workflow-output" {
            self.data_state.show_workflow_output();
            "data"
        } else if key == "text-flow" {
            self.data_state.show_text_flow();
            "data"
        } else if matches!(key, "pipeline" | "pipeline-step-inspect" | "workflow-file") {
            self.data_state.show_workflow();
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
    fn open_task_row(&mut self, run_id: &RunId) -> anyhow::Result<()> {
        if run_id.key() == "image-workflow" {
            self.images.open_task(
                run_id
                    .instance()
                    .ok_or_else(|| anyhow::anyhow!("图片任务缺少实例"))?,
                run_id.generation(),
            )?;
            self.page = Page::Images;
            self.visit("image-workflow");
            Ok(())
        } else {
            self.open_task_result(run_id.key(), run_id.instance())
        }
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
        job.begin();
        let mut plugin = job
            .snapshot("plugin-execution", "插件工具请求", true)
            .unwrap();
        plugin.instance = Some("plugin:preview/http".into());
        self.tasks.observe(Some(plugin));
        job.begin();
        let mut discovery = job
            .snapshot("plugin-model-discovery", "插件模型列表查询", true)
            .unwrap();
        discovery.instance = Some("plugin:preview/models".into());
        self.tasks.observe(Some(discovery));
        job.begin();
        let mut update = job
            .snapshot("plugin-update-check", "插件更新清单检查", true)
            .unwrap();
        update.instance = Some("plugin-update-preview".into());
        self.tasks.observe(Some(update));
        if let Some(tab) = self.http_state.tabs.first_mut() {
            tab.job.begin();
            tab.busy = true;
        }
        self.network_state.request_id = 1;
        self.network_state.task_pane = crate::network_tools::NetworkPane::Dns;
        self.network_state.job.begin();
        self.network_state.busy = true;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_task_cancel_confirmation(&mut self) {
        self.task_cancel_all_pending = true;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_task_history(&mut self) {
        self.task_cancel_all_pending = false;
        self.preferences.task_history_local_enabled = true;
        let now = chrono::Utc::now().timestamp();
        self.task_history_local = vec![
            crate::task_history::Record {
                key: "files".into(),
                title: "批量文件校验".into(),
                phase: crate::tasks::Phase::Done,
                elapsed: Duration::from_millis(3480),
                completed_at: now - 120,
            },
            crate::task_history::Record {
                key: "csv-merge".into(),
                title: "表格合并与关联".into(),
                phase: crate::tasks::Phase::Failed,
                elapsed: Duration::from_millis(8130),
                completed_at: now - 3600,
            },
            crate::task_history::Record {
                key: "plugin-execution".into(),
                title: "插件工具请求".into(),
                phase: crate::tasks::Phase::Cancelled,
                elapsed: Duration::from_millis(10200),
                completed_at: now - 86400,
            },
        ];
    }

    fn task_snapshots(&self) -> Vec<crate::tasks::Row> {
        let mut rows = self.data_state.snapshots();
        rows.extend(self.images.task_snapshots());
        rows.extend(self.file_state.job.snapshot("files", "批量文件校验", true));
        rows.extend(self.directory_compare.task_snapshot());
        rows.extend(
            self.service_batch
                .job
                .snapshot("services", "服务批量操作", true),
        );
        rows.extend(self.agent.task_snapshot());
        rows.extend(self.clipboard.task_snapshot());
        rows.extend(self.frameworks.task_snapshots());
        rows.extend(self.plugins.task_snapshots());
        rows.extend(self.mcp.task_snapshot());
        rows.extend(self.http_state.task_snapshots());
        rows.extend(self.network_state.task_snapshot());
        rows
    }
    pub(super) fn observe_tasks(&mut self) {
        for row in self.images.take_task_receipts() {
            self.tasks.observe(Some(row));
        }
        for row in self.task_snapshots() {
            self.tasks.observe(Some(row));
        }
    }
    pub(super) fn poll_tasks(&mut self, ctx: &egui::Context) {
        self.observe_tasks();
        let before = self.task_snapshots();
        self.agent.poll_background(ctx);
        self.mcp.tick(ctx);
        let framework_message = self.frameworks.poll();
        let plugin_message = self.plugins.poll();
        self.images.poll_screenshot(ctx);
        self.data_state.poll();
        if let Some(delay) = self
            .data_state
            .autosave_wake_after(std::time::Instant::now())
        {
            ctx.request_repaint_after(delay);
        }
        if self.images.can_receive_workflow()
            && let Some(definition) = self.data_state.take_image_workflow()
        {
            match self.images.receive_workflow(definition) {
                Ok(()) => {
                    self.page = Page::Images;
                    self.visit("image-workflow");
                }
                Err(error) => self.toast = Some((format!("{error:#}"), Instant::now())),
            }
        }
        let visits = self.data_state.take_workflow_visits();
        if self.page == Page::Data {
            for tool in visits {
                self.visit(tool);
            }
        }

        let mut loads = self.data_state.take_workflow_loads();
        if let Some(metadata) = self.images.take_workflow_loaded() {
            loads.push(metadata);
        }
        let mut history_error = None;
        if !loads.is_empty() {
            for loaded in loads {
                self.preferences.workflow_loaded(loaded);
            }
            if let Err(error) = self.preferences.save(&self.preferences_path) {
                self.last_workflow_retry = Instant::now();
                history_error = Some((
                    format!("流程已载入，最近记录尚未保存：{error:#}"),
                    Instant::now(),
                ));
            }
        }
        self.file_state.poll();
        self.directory_compare.poll_background();
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
                let task_name = row.instance_name.as_deref().map_or_else(
                    || row.title.to_owned(),
                    |name| format!("{} · {name}", row.title),
                );
                self.toast = Some((
                    format!("{} · {}", task_name, row.phase.label()),
                    Instant::now(),
                ));
                if self.preferences.task_history_local_enabled {
                    match crate::task_history::append(
                        &self.preferences_path.with_file_name("task-history.sqlite3"),
                        row,
                    ) {
                        Ok(record) => {
                            self.task_history_local.insert(0, record);
                            self.task_history_local
                                .truncate(crate::task_history::MAX_RECORDS);
                            self.task_history_load_error = None;
                        }
                        Err(error) => {
                            self.task_history_load_error =
                                Some(format!("本机任务历史保存失败：{error:#}"));
                        }
                    }
                }
            }
        }
        self.observe_tasks();
        if after.iter().any(|r| r.phase.active()) || self.data_state.operation_pending() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if history_error.is_some() {
            self.toast = history_error;
        }
        if let Some(message) = framework_message {
            self.toast = Some((message, Instant::now()));
        }
        if let Some(message) = plugin_message {
            self.toast = Some((message, Instant::now()));
        }
    }
    fn request_task_cancel(&mut self, run_id: RunId) {
        let Some(registration) = task_cancel_registration(run_id.key()) else {
            return;
        };
        if registration.requires_instance && run_id.instance().is_none() {
            self.toast = Some(("任务缺少实例标识，未发送取消请求".into(), Instant::now()));
            return;
        }
        (registration.handler)(self, &run_id);
    }

    pub(super) fn tasks_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("后台任务中心");
        ui.label(
            "数据、文件、服务、图片流程、HTTP 请求、网络诊断、Java/Django、插件请求、MCP、本机 Agent 与剪贴板 OCR · 后台状态持续更新",
        );
        ui.small("本次运行状态最多保留 50 条；跨重启历史默认关闭，开启后只记录不含任务内容的有限元数据。各工具的详细结果留在对应工作台中。");
        let active = self.tasks.rows.iter().filter(|r| r.phase.active()).count();
        let failed = self
            .tasks
            .rows
            .iter()
            .filter(|r| r.phase == Phase::Failed)
            .count();
        let finished = self.tasks.rows.iter().filter(|r| !r.phase.active()).count();
        let cancelable = self
            .tasks
            .cancellable_running()
            .into_iter()
            .filter(task_row_has_cancel_route)
            .collect::<Vec<_>>();
        let mut request_cancel_all_confirmation = false;
        let mut confirm_cancel_all = false;
        let mut dismiss_cancel_all = false;
        let mut history_enabled = self.preferences.task_history_local_enabled;
        let mut history_clear_requested = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!(
                "{active} 项运行中 · {} 条记录",
                self.tasks.rows.len()
            ));
            let clear = ui.button("清除已结束记录");
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("task-history-clear"),
                    clear.rect.intersect(ui.clip_rect()),
                )
            });
            if clear.clicked() {
                self.tasks.clear_finished();
            }
            if ui
                .checkbox(&mut history_enabled, "在本机保存任务历史")
                .changed()
            {
                let mut next = self.preferences.clone();
                next.task_history_local_enabled = history_enabled;
                match next.save(&self.preferences_path) {
                    Ok(()) => {
                        self.preferences = next;
                        if history_enabled {
                            match crate::task_history::load(
                                &self.preferences_path.with_file_name("task-history.sqlite3"),
                            ) {
                                Ok(records) => {
                                    self.task_history_local = records;
                                    self.task_history_load_error = None;
                                }
                                Err(error) => {
                                    self.task_history_load_error =
                                        Some(format!("本机任务历史读取失败：{error:#}"));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.task_history_load_error =
                            Some(format!("保存任务历史设置失败：{error:#}"));
                    }
                }
            }
            history_clear_requested =
                !self.task_history_local.is_empty() && ui.button("清空本机历史").clicked();
            if !cancelable.is_empty()
                && ui
                    .button(format!("取消全部可取消任务 ({})", cancelable.len()))
                    .clicked()
            {
                request_cancel_all_confirmation = true;
            }
        });
        ui.small("关闭保存会停止记录新任务，已有历史保留。开启后仅保存工具名称、状态、耗时和完成时间；不保存实例名、摘要、路径、输入或输出。最多保留 1,000 条，可随时清空。");
        if let Some(error) = &self.task_history_load_error {
            ui.colored_label(self.colors.red, error);
        }
        if history_clear_requested {
            self.task_history_clear_confirm = true;
        }
        if self.task_history_clear_confirm {
            ui.group(|ui| {
                ui.label("清空已保存在本机的任务历史？当前会话中的任务状态不受影响。");
                ui.horizontal(|ui| {
                    if ui.button("确认清空历史").clicked() {
                        match crate::task_history::clear(
                            &self.preferences_path.with_file_name("task-history.sqlite3"),
                        ) {
                            Ok(()) => {
                                self.task_history_local.clear();
                                self.task_history_load_error = None;
                            }
                            Err(error) => {
                                self.task_history_load_error =
                                    Some(format!("清空本机任务历史失败：{error:#}"));
                            }
                        }
                        self.task_history_clear_confirm = false;
                    }
                    if ui.button("返回").clicked() {
                        self.task_history_clear_confirm = false;
                    }
                });
            });
        }
        if self.task_cancel_all_pending {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.strong("确认批量取消任务");
                ui.label(format!(
                    "将请求取消 {} 项当前正在运行、且提供取消入口的任务：",
                    cancelable.len()
                ));
                for row in cancelable.iter().take(6) {
                    let label = row.instance_name.as_deref().map_or_else(
                        || row.title.to_owned(),
                        |name| format!("{} · {name}", row.title),
                    );
                    ui.label(format!("• {label}"));
                }
                if cancelable.len() > 6 {
                    ui.small(format!("另有 {} 项…", cancelable.len() - 6));
                }
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "取消只会向本机任务发送停止请求。已经写入的数据或已发送的网络请求无法撤回；远端服务可能继续处理已收到的请求。",
                );
                if cancelable.is_empty() {
                    ui.label("当前没有可取消的运行中任务。");
                }
                ui.horizontal(|ui| {
                    confirm_cancel_all = ui
                        .add_enabled(
                            !cancelable.is_empty(),
                            egui::Button::new(format!("确认取消 {} 项", cancelable.len())),
                        )
                        .clicked();
                    dismiss_cancel_all = ui.button("返回，不取消").clicked();
                });
            });
        }
        if request_cancel_all_confirmation {
            self.task_cancel_all_pending = true;
        }
        if dismiss_cancel_all {
            self.task_cancel_all_pending = false;
        }
        if confirm_cancel_all {
            self.task_cancel_all_pending = false;
            let targets = self
                .tasks
                .cancellable_running()
                .into_iter()
                .filter(task_row_has_cancel_route)
                .map(|row| row.run_id())
                .collect::<Vec<_>>();
            for run_id in targets {
                self.request_task_cancel(run_id);
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("搜索任务或实例");
            ui.add(
                egui::TextEdit::singleline(&mut self.task_query)
                    .hint_text("例如：图片、账单、文件校验")
                    .desired_width(250.0)
                    .char_limit(256),
            );
            if !self.task_query.is_empty() && ui.small_button("清除搜索").clicked() {
                self.task_query.clear();
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (filter, count) in [
                (crate::tasks::ViewFilter::All, self.tasks.rows.len()),
                (crate::tasks::ViewFilter::Active, active),
                (crate::tasks::ViewFilter::Failed, failed),
                (crate::tasks::ViewFilter::Finished, finished),
            ] {
                ui.selectable_value(
                    &mut self.task_filter,
                    filter,
                    format!("{} {count}", filter.label()),
                );
            }
        });
        let mut rows = self
            .tasks
            .filtered(self.task_filter, &self.task_query)
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| !row.phase.active());
        ui.small(format!(
            "当前显示 {} / {} 条",
            rows.len(),
            self.tasks.rows.len()
        ));
        ui.separator();
        let mut open: Option<RunId> = None;
        let mut cancel: Option<RunId> = None;
        egui::ScrollArea::vertical()
            .id_salt("task-history")
            .show(ui, |ui| {
                if self.tasks.rows.is_empty() {
                    ui.add_space(24.0);
                    ui.label("尚无任务。运行数据处理、HTTP 请求、网络诊断、图片流程、Java/Django 检查、插件请求、MCP 操作、Agent、文件校验或目录对比/复制，状态会出现在这里。");
                }
                if !self.tasks.rows.is_empty() && rows.is_empty() {
                    ui.add_space(24.0);
                    ui.label("没有符合当前搜索和状态筛选的任务。");
                }
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
                    let open_button = ui.button(if row.key == "image-workflow" {
                                        "打开所属实例"
                    } else if row.key == "http-request" {
                        if row
                            .instance
                            .as_deref()
                            .and_then(|id| id.parse::<u64>().ok())
                            .is_some_and(|id| self.http_state.tabs.iter().any(|tab| tab.id == id))
                        {
                            "打开请求标签"
                        } else {
                            "打开 HTTP 工作台"
                        }
                                    } else {
                                        "打开工具"
                                    });
                                    #[cfg(feature = "ui-preview")]
                                    if row.key == "file-compare" {
                                        ui.ctx().data_mut(|d| d.insert_temp(
                                            egui::Id::new(format!("directory-task-open-{}", row.generation)),
                                            open_button.rect.intersect(ui.clip_rect())));
                                    }
                                    #[cfg(feature = "ui-preview")]
                                    if row.key == "http-request" {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(
                                                egui::Id::new(format!(
                                                    "task-center-http-open-{}",
                                                    row.instance.as_deref().unwrap_or_default()
                                                )),
                                                open_button.rect.intersect(ui.clip_rect()),
                                            )
                                        });
                                    }
                                    #[cfg(feature = "ui-preview")]
                                    if row.key == "network-diagnostic" {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(
                                                egui::Id::new("task-center-network-open"),
                                                open_button.rect.intersect(ui.clip_rect()),
                                            )
                                        });
                                    }
                                    #[cfg(feature = "ui-preview")]
                                    if row.key == "image-workflow" {
                                        ui.ctx().data_mut(|d| {
                                            d.insert_temp(
                                                egui::Id::new(format!(
                                                    "image-task-open-{}-{}",
                                                    row.instance.as_deref().unwrap_or_default(),
                                                    row.generation
                                                )),
                                                open_button.rect.intersect(ui.clip_rect()),
                                            )
                                        });
                                    }
                                    if open_button.clicked() {
                                        open = Some(row.run_id());
                                    }
                                    if task_row_has_cancel_route(row)
                                        && row.phase == Phase::Running
                                    {
                                        let cancel_button = ui.button(match row.key {
                                            "plugin-execution" => "停止客户端请求",
                                            "plugin-model-discovery" => "取消模型查询",
                                            _ => "取消任务",
                                        });
                                        #[cfg(feature = "ui-preview")]
                                        let cancel_rect = cancel_button.rect;
                                        let cancel_clicked = cancel_button.clicked();
                                        #[cfg(feature = "ui-preview")]
                                        if row.key == "file-compare" {
                                            ui.ctx().data_mut(|d| d.insert_temp(
                                                egui::Id::new(format!("directory-task-cancel-{}", row.generation)),
                                                cancel_rect.intersect(ui.clip_rect())));
                                        }
                                        if row.key == "plugin-execution" {
                                            cancel_button.on_hover_text(
                                                "停止客户端等待；远端服务可能继续处理已收到的请求",
                                            );
                                        }
                                        #[cfg(feature = "ui-preview")]
                                        if row.key == "image-workflow" {
                                            ui.ctx().data_mut(|d| {
                                                d.insert_temp(
                                                    egui::Id::new(format!(
                                                        "image-task-cancel-{}-{}",
                                                        row.instance.as_deref().unwrap_or_default(),
                                                        row.generation
                                                    )),
                                                    cancel_rect.intersect(ui.clip_rect()),
                                                )
                                            });
                                        }
                                        if cancel_clicked {
                                            cancel = Some(row.run_id());
                                        }
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
                if self.preferences.task_history_local_enabled || !self.task_history_local.is_empty() {
                    ui.separator();
                    ui.heading("本机历史");
                    if self.task_history_local.is_empty() {
                        ui.label("还没有已保存的任务历史。");
                    }
                    let matching_history = self
                        .task_history_local
                        .iter()
                        .filter(|record| {
                            self.task_query.split_whitespace().all(|term| {
                                record.title.to_lowercase().contains(&term.to_lowercase())
                            })
                        })
                        .collect::<Vec<_>>();
                    if matching_history.len() > 100 {
                        ui.small(format!(
                            "匹配 {} 条；为保持列表流畅，当前显示最近 100 条。",
                            matching_history.len()
                        ));
                    }
                    for record in matching_history.into_iter().take(100) {
                        egui::Frame::new()
                            .fill(self.colors.card)
                            .corner_radius(8)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(&record.title);
                                    let color = match record.phase {
                                        Phase::Failed => self.colors.red,
                                        Phase::Done => self.colors.green,
                                        _ => self.colors.amber,
                                    };
                                    ui.colored_label(color, record.phase.label());
                                    ui.label(format!("{:.1} 秒", record.elapsed.as_secs_f32()));
                                    let time = chrono::DateTime::from_timestamp(record.completed_at, 0)
                                        .map(|utc| utc.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                                        .unwrap_or_else(|| "时间未知".into());
                                    ui.label(time);
                                });
                            });
                    }
                }
            });
        if let Some(run_id) = cancel {
            self.request_task_cancel(run_id);
        }
        if let Some(run_id) = open {
            if let Err(error) = self.open_task_row(&run_id) {
                self.toast = Some((error.to_string(), Instant::now()));
            }
        }
    }
}

type TaskCancelHandler = fn(&mut DevToolsApp, &RunId);

#[derive(Clone, Copy)]
struct TaskCancelRegistration {
    key: &'static str,
    requires_instance: bool,
    handler: TaskCancelHandler,
}

const TASK_CANCEL_REGISTRY: &[TaskCancelRegistration] = &[
    TaskCancelRegistration {
        key: "file-compare",
        requires_instance: false,
        handler: cancel_directory_task,
    },
    TaskCancelRegistration {
        key: "image-workflow",
        requires_instance: true,
        handler: cancel_image_workflow,
    },
    TaskCancelRegistration {
        key: "csv-merge",
        requires_instance: true,
        handler: cancel_csv_merge,
    },
    TaskCancelRegistration {
        key: "sqlite-export",
        requires_instance: true,
        handler: cancel_sqlite_export,
    },
    TaskCancelRegistration {
        key: "pipeline",
        requires_instance: true,
        handler: cancel_pipeline,
    },
    TaskCancelRegistration {
        key: "pipeline-step-inspect",
        requires_instance: true,
        handler: cancel_pipeline_step_inspection,
    },
    TaskCancelRegistration {
        key: "text-flow",
        requires_instance: true,
        handler: cancel_text_flow,
    },
    TaskCancelRegistration {
        key: "workflow-output",
        requires_instance: true,
        handler: cancel_workflow_output,
    },
    TaskCancelRegistration {
        key: "files",
        requires_instance: false,
        handler: cancel_file_validation,
    },
    TaskCancelRegistration {
        key: "services",
        requires_instance: false,
        handler: cancel_service_batch,
    },
    TaskCancelRegistration {
        key: "agent-runner",
        requires_instance: false,
        handler: cancel_agent,
    },
    TaskCancelRegistration {
        key: "plugin-execution",
        requires_instance: true,
        handler: cancel_plugin,
    },
    TaskCancelRegistration {
        key: "plugin-model-discovery",
        requires_instance: true,
        handler: cancel_plugin,
    },
    TaskCancelRegistration {
        key: "plugin-update-check",
        requires_instance: true,
        handler: cancel_plugin,
    },
    TaskCancelRegistration {
        key: "plugin-catalog-check",
        requires_instance: true,
        handler: cancel_plugin,
    },
    TaskCancelRegistration {
        key: "mcp-operation",
        requires_instance: false,
        handler: cancel_mcp,
    },
];

const FRAMEWORK_CANCEL_REGISTRATION: TaskCancelRegistration = TaskCancelRegistration {
    key: "framework-tool",
    requires_instance: false,
    handler: cancel_framework_tool,
};

fn task_cancel_registration(key: &str) -> Option<&'static TaskCancelRegistration> {
    TASK_CANCEL_REGISTRY
        .iter()
        .find(|registration| registration.key == key)
        .or_else(|| crate::framework::Tool::from_id(key).map(|_| &FRAMEWORK_CANCEL_REGISTRATION))
}

fn task_row_has_cancel_route(row: &crate::tasks::Row) -> bool {
    row.cancellable
        && task_cancel_registration(row.key)
            .is_some_and(|registration| !registration.requires_instance || row.instance.is_some())
}

fn cancel_image_workflow(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance()
        && let Err(error) = app.images.cancel_task(id, run_id.generation())
    {
        app.toast = Some((error.to_string(), Instant::now()));
    }
}

fn cancel_csv_merge(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance() {
        app.data_state.cancel(id, run_id.generation());
    }
}

fn cancel_sqlite_export(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance() {
        app.data_state.cancel_sqlite(id, run_id.generation());
    }
}

fn cancel_pipeline(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance() {
        app.data_state.cancel_workflow(id, run_id.generation());
    }
}

fn cancel_pipeline_step_inspection(app: &mut DevToolsApp, run_id: &RunId) {
    if !run_id.instance().is_some_and(|id| {
        app.data_state
            .cancel_workflow_step_inspection(id, run_id.generation())
    }) {
        app.toast = Some(("中间结果预览已结束或记录已过期".into(), Instant::now()));
    }
}

fn cancel_text_flow(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance() {
        app.data_state.cancel_text_flow(id, run_id.generation());
    }
}

fn cancel_workflow_output(app: &mut DevToolsApp, run_id: &RunId) {
    if let Some(id) = run_id.instance() {
        app.data_state
            .cancel_workflow_output(id, run_id.generation());
    }
}

fn cancel_file_validation(app: &mut DevToolsApp, run_id: &RunId) {
    if !app.file_state.cancel_task(run_id.generation()) {
        app.toast = Some(("文件校验已结束或记录已过期".into(), Instant::now()));
    }
}

fn cancel_directory_task(app: &mut DevToolsApp, run_id: &RunId) {
    if !app.directory_compare.cancel_task(run_id) {
        app.toast = Some(("目录任务已结束或记录已过期".into(), Instant::now()));
    }
}

fn cancel_service_batch(app: &mut DevToolsApp, run_id: &RunId) {
    app.service_batch.cancel(run_id.generation());
}

fn cancel_agent(app: &mut DevToolsApp, run_id: &RunId) {
    if !app.agent.cancel_task(run_id.generation()) {
        app.toast = Some(("任务已结束或记录已过期".into(), Instant::now()));
    }
}

fn cancel_plugin(app: &mut DevToolsApp, run_id: &RunId) {
    if !run_id
        .instance()
        .is_some_and(|id| app.plugins.cancel_task(id, run_id.generation()))
    {
        app.toast = Some(("任务已结束或记录已过期".into(), Instant::now()));
    }
}

fn cancel_mcp(app: &mut DevToolsApp, run_id: &RunId) {
    if !app.mcp.cancel_task(run_id.generation()) {
        app.toast = Some((
            "MCP 操作已结束、已清理，或属于持续连接；请在 MCP 工作台断开连接".into(),
            Instant::now(),
        ));
    }
}

fn cancel_framework_tool(app: &mut DevToolsApp, run_id: &RunId) {
    if !app
        .frameworks
        .cancel_task(run_id.key(), run_id.generation())
    {
        app.toast = Some(("任务已结束或记录已过期".into(), Instant::now()));
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::{
        FRAMEWORK_CANCEL_REGISTRATION, TASK_CANCEL_REGISTRY, task_cancel_registration,
        task_row_has_cancel_route,
    };
    use crate::tasks::{Phase, Row};
    use std::time::Duration;

    #[test]
    fn registry_advertises_only_keys_with_a_real_handler() {
        let mut keys = std::collections::HashSet::new();
        for registration in TASK_CANCEL_REGISTRY {
            assert!(
                keys.insert(registration.key),
                "duplicate route: {}",
                registration.key
            );
            assert!(task_cancel_registration(registration.key).is_some());
        }
        for tool in crate::framework::Tool::ALL {
            assert!(
                task_cancel_registration(tool.id()).is_some(),
                "{}",
                tool.id()
            );
        }
        for key in ["http-request", "network-diagnostic", "unknown-task"] {
            assert!(task_cancel_registration(key).is_none(), "{key}");
        }
        assert_eq!(FRAMEWORK_CANCEL_REGISTRATION.key, "framework-tool");
    }

    #[test]
    fn rows_require_a_live_cancel_route_and_any_required_instance() {
        let row = |key: &'static str, instance: Option<&str>| Row {
            owner_id: uuid::Uuid::new_v4(),
            key,
            instance: instance.map(str::to_owned),
            instance_name: None,
            generation: 1,
            title: "测试任务",
            phase: Phase::Running,
            elapsed: Duration::ZERO,
            progress: None,
            summary: String::new(),
            cancellable: true,
        };
        assert!(task_row_has_cancel_route(&row("files", None)));
        assert!(task_row_has_cancel_route(&row("file-compare", None)));
        assert!(task_row_has_cancel_route(&row("java-environment", None)));
        assert!(task_row_has_cancel_route(&row(
            "plugin-execution",
            Some("plugin-a")
        )));
        assert!(!task_row_has_cancel_route(&row("plugin-execution", None)));
        assert!(!task_row_has_cancel_route(&row(
            "http-request",
            Some("tab-1")
        )));
        assert!(task_row_has_cancel_route(&row("files", Some("folder-a"))));
        assert!(!task_row_has_cancel_route(&row("image-workflow", None)));
        assert!(task_row_has_cancel_route(&row(
            "image-workflow",
            Some("instance-a")
        )));
    }
}

#[cfg(feature = "ui-preview")]
mod image_preview;

#[cfg(feature = "ui-preview")]
mod directory_preview;
