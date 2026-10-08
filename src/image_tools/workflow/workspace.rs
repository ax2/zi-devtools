//! Independent in-memory image work, including inactive job ownership.
use super::*;
use anyhow::Context;
use std::ops::{Deref, DerefMut};
const MAX_INSTANCES: usize = 8;
struct Instance {
    id: String,
    title: String,
    state: State,
}
pub(crate) struct Workspace {
    instances: Vec<Instance>,
    memory: super::super::memory::Pool,
    memory_limit_mb: u32,
    active: usize,
    sequence: u64,
    close_confirm: Option<String>,
    message: String,
    closed_tasks: std::collections::VecDeque<crate::tasks::Row>,
    closed_receipts: std::collections::VecDeque<crate::preferences::SavedWorkflow>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Destination {
    #[default]
    New,
    Existing {
        id: String,
        revision: u64,
    },
}
pub(crate) struct RelayChoice {
    pub destination: Destination,
    pub title: String,
    pub busy: bool,
    pub has_work: bool,
}
impl Default for Workspace {
    fn default() -> Self {
        let memory = super::super::memory::Pool::default();
        Self {
            memory: memory.clone(),
            memory_limit_mb: 1024,
            instances: vec![Instance {
                id: uuid::Uuid::new_v4().to_string(),
                title: "图片流程 1".into(),
                state: State::with_memory(memory.clone()),
            }],
            active: 0,
            sequence: 1,
            close_confirm: None,
            message: String::new(),
            closed_receipts: Default::default(),
            closed_tasks: Default::default(),
        }
    }
}
impl Deref for Workspace {
    type Target = State;
    fn deref(&self) -> &State {
        &self.instances[self.active].state
    }
}
impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.instances[self.active].state
    }
}
impl Workspace {
    pub(crate) fn active_id(&self) -> String {
        self.instances[self.active].id.clone()
    }
    pub(crate) fn relay_choices(&self, source_id: Option<&str>) -> Vec<RelayChoice> {
        self.instances
            .iter()
            .filter(|i| Some(i.id.as_str()) != source_id)
            .map(|i| {
                let (busy, has_work) = i.state.target_state();
                RelayChoice {
                    destination: Destination::Existing {
                        id: i.id.clone(),
                        revision: i.state.revision,
                    },
                    title: i.title.clone(),
                    busy: busy || self.close_confirm.is_some(),
                    has_work,
                }
            })
            .collect()
    }
    pub(crate) fn relay_destination_state(
        &self,
        destination: &Destination,
        source_id: Option<&str>,
    ) -> Result<(bool, bool)> {
        ensure!(self.close_confirm.is_none(), "请先确认或取消关闭实例");
        match destination {
            Destination::New => {
                ensure!(
                    self.instances.len() < MAX_INSTANCES,
                    "已有8份图片实例，请先关闭不需要的实例"
                );
                Ok((false, false))
            }
            Destination::Existing { id, revision } => {
                ensure!(
                    Some(id.as_str()) != source_id,
                    "不能替换来源实例，请选择另一实例或新建"
                );
                let instance = self
                    .instances
                    .iter()
                    .find(|i| &i.id == id)
                    .context("接收实例已关闭，请重新选择")?;
                ensure!(
                    instance.state.revision == *revision,
                    "接收实例已变化，请重新选择并确认替换"
                );
                Ok(instance.state.target_state())
            }
        }
    }
    pub(crate) fn receive_at(
        &mut self,
        destination: &Destination,
        source_id: Option<&str>,
        replace: bool,
        prepared: &relay::Prepared,
    ) -> Result<()> {
        let (busy, has_work) = self.relay_destination_state(destination, source_id)?;
        ensure!(!busy, "接收实例正在处理或等待导入确认");
        ensure!(!has_work || replace, "请明确允许替换接收实例的输入和结果");
        self.memory
            .share(
                &prepared.image,
                super::super::memory::pixels(&prepared.image),
            )
            .map_err(anyhow::Error::msg)?;
        let index = match destination {
            Destination::New => {
                self.create()?;
                self.active
            }
            Destination::Existing { id, .. } => self
                .instances
                .iter()
                .position(|i| &i.id == id)
                .context("接收实例已关闭")?,
        };
        self.instances[index].state.receive(prepared)?;
        self.active = index;
        Ok(())
    }
    pub(crate) fn background_active(&self) -> bool {
        self.instances
            .iter()
            .any(|i| i.state.busy() || i.state.pending_import())
            || self.close_confirm.is_some()
    }
    pub(crate) fn can_receive(&self) -> bool {
        self.close_confirm.is_none() && !self.busy() && !self.pending_import()
    }
    pub(crate) fn target_state(&self) -> (bool, bool) {
        let (busy, content) = self.deref().target_state();
        (busy || self.close_confirm.is_some(), content)
    }
    pub(crate) fn relay_source(&self) -> Option<(relay::Source, Vec<relay::Origin>, &'static str)> {
        if self.close_confirm.is_some() {
            None
        } else {
            self.deref().relay_source()
        }
    }
    pub(crate) fn poll(&mut self, ctx: &egui::Context) {
        for instance in &mut self.instances {
            instance.state.poll(ctx);
        }
    }
    pub(crate) fn take_loaded(&mut self) -> Option<crate::preferences::SavedWorkflow> {
        self.closed_receipts.pop_front().or_else(|| {
            self.instances
                .iter_mut()
                .find_map(|i| i.state.take_loaded())
        })
    }
    pub(crate) fn snapshots(&self) -> Vec<crate::tasks::Row> {
        self.instances
            .iter()
            .filter_map(|i| {
                i.state.task_snapshot().map(|mut r| {
                    r.instance = Some(i.id.clone());
                    r.instance_name = Some(i.title.clone());
                    r
                })
            })
            .collect()
    }
    pub(crate) fn take_task_receipts(&mut self) -> Vec<crate::tasks::Row> {
        let mut rows: Vec<_> = self.closed_tasks.drain(..).collect();
        for instance in &mut self.instances {
            rows.extend(instance.state.completed_tasks.drain(..).map(|mut row| {
                row.instance = Some(instance.id.clone());
                row.instance_name = Some(instance.title.clone());
                row
            }));
        }
        rows
    }
    pub(crate) fn cancel_task(&mut self, id: &str, generation: u64) -> Result<()> {
        let state = &mut self
            .instances
            .iter_mut()
            .find(|i| i.id == id)
            .context("图片实例已关闭")?
            .state;
        ensure!(
            state
                .task_snapshot()
                .is_some_and(|r| r.generation == generation && r.phase.active()),
            "任务已结束或已变化"
        );
        state.request_cancel();
        Ok(())
    }
    pub(crate) fn open_task(&mut self, id: &str, generation: u64) -> Result<()> {
        let index = self
            .instances
            .iter()
            .position(|i| i.id == id)
            .context("图片实例已关闭，历史状态保留")?;
        ensure!(
            self.instances[index]
                .state
                .task_snapshot()
                .is_some_and(|r| r.generation == generation),
            "历史任务已被新操作替换，请从图片页选择实例"
        );
        self.select(index)
    }
    fn create(&mut self) -> Result<()> {
        ensure!(self.close_confirm.is_none(), "请先确认或取消关闭");
        ensure!(
            self.instances.len() < MAX_INSTANCES,
            "最多同时保留8份图片流程；保存定义和结果后关闭不用的实例"
        );
        self.sequence += 1;
        self.instances.push(Instance {
            id: uuid::Uuid::new_v4().to_string(),
            title: format!("图片流程 {}", self.sequence),
            state: State::with_memory(self.memory.clone()),
        });
        self.active = self.instances.len() - 1;
        self.message.clear();
        Ok(())
    }
    fn select(&mut self, index: usize) -> Result<()> {
        ensure!(self.close_confirm.is_none(), "请先确认或取消关闭");
        ensure!(index < self.instances.len(), "图片实例已关闭");
        self.active = index;
        self.message.clear();
        Ok(())
    }
    fn request_close(&mut self) -> Result<()> {
        ensure!(!self.busy(), "请先取消或等待该实例任务结束，再关闭");
        self.close_confirm = Some(self.instances[self.active].id.clone());
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        let id = self.close_confirm.as_ref().context("没有待关闭实例")?;
        let index = self
            .instances
            .iter()
            .position(|i| &i.id == id)
            .context("实例已关闭")?;
        ensure!(!self.instances[index].state.busy(), "请等待该实例任务收尾");
        let final_row = self.instances[index].state.task_snapshot();
        ensure!(
            !final_row.as_ref().is_some_and(|r| r.phase.active()),
            "请等待该实例任务收尾"
        );
        let id = self.instances[index].id.clone();
        let name = self.instances[index].title.clone();
        let mut rows: Vec<_> = self.instances[index]
            .state
            .completed_tasks
            .drain(..)
            .collect();
        if let Some(row) = final_row
            && !rows.iter().any(|r| r.generation == row.generation)
        {
            rows.push(row);
        }
        for mut row in rows {
            row.instance = Some(id.clone());
            row.instance_name = Some(name.clone());
            self.closed_tasks.push_back(row);
        }
        if let Some(receipt) = self.instances[index].state.take_loaded() {
            self.closed_receipts.push_back(receipt);
        }
        self.instances.remove(index);
        if self.instances.is_empty() {
            self.sequence += 1;
            self.instances.push(Instance {
                id: uuid::Uuid::new_v4().to_string(),
                title: format!("图片流程 {}", self.sequence),
                state: State::with_memory(self.memory.clone()),
            });
        }
        self.active = self.active.min(self.instances.len() - 1);
        self.close_confirm = None;
        self.message.clear();
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_instance_check(&self, phase: u8) {
        match phase {
            1 => {
                assert_eq!(self.instances.len(), 2);
                assert_eq!(self.active, 1);
                assert!(self.source.is_none());
                assert!(self.run.is_none());
                assert!(self.instances[0].state.run.is_some());
            }
            2 | 3 => {
                assert_eq!(self.instances.len(), 2);
                assert_eq!(self.active, 0);
                assert_eq!(self.selected, 0);
                assert_eq!(self.source.as_ref().unwrap().dimensions(), (320, 180));
                assert_eq!(self.run.as_ref().unwrap().outputs.len(), 4);
                assert!(self.close_confirm.is_none());
            }
            4 => {
                assert_eq!(self.instances.len(), 1);
                assert!(self.source.is_none());
                assert!(self.run.is_none());
                assert!(self.close_confirm.is_none());
            }
            _ => panic!("unknown phase"),
        }
    }
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, unlocked: bool) {
        self.poll(ui.ctx());
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                match self.memory.snapshot() {
                    Ok((retained, reserved, limit)) => { ui.small(format!("图片内存 {:.1} MiB · 处理中 {:.1} MiB / {:.0} MiB", retained as f64 / 1048576.0, reserved as f64 / 1048576.0, limit as f64 / 1048576.0)); },
                    Err(message) => { ui.label(message); }
                }
                ui.menu_button("内存设置", |ui| {
                    ui.set_max_width(340.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("本次流程预算 MiB");
                    let old = self.memory_limit_mb;
                    if ui.add(egui::DragValue::new(&mut self.memory_limit_mb).range(64..=8192)).changed()
                        && let Err(message) = self.memory.set_limit(self.memory_limit_mb as usize * 1048576) {
                        self.memory_limit_mb = old;
                        self.message = message.into();
                    }
                });
                ui.small("包含流程图片、编码缓冲、文件载入缓冲与解码像素预留。其他图片工具、编解码器内部临时内存、预览和GPU未统一；不是程序总内存。不自动清除工作，仅当前会话有效。");
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.strong("图片工作实例");
                let enabled = unlocked && self.close_confirm.is_none();
                let mut selected = self.active;
                ui.add_enabled_ui(enabled, |ui| {
                    let picker = egui::ComboBox::from_id_salt("image-workspace-select")
                        .selected_text(&self.instances[self.active].title)
                        .show_ui(ui, |ui| {
                            for (index, instance) in self.instances.iter().enumerate() {
                                let status = if instance.state.busy() { "运行中" } else if instance.state.pending_import() { "待确认" } else if instance.state.run.is_some() { "有结果" } else { "草稿" };
                                let row = ui.selectable_value(&mut selected, index, format!("{} · {}", instance.title, status));
                                #[cfg(feature = "ui-preview")]
                                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(format!("image-instance-select-{index}")),row.rect.intersect(ui.clip_rect())));
                                #[cfg(not(feature = "ui-preview"))]
                                let _ = row;
                            }
                        });
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-instance-picker"),picker.response.rect.intersect(ui.clip_rect())));
                    #[cfg(not(feature = "ui-preview"))]
                    let _ = picker;
                });
                if selected != self.active && let Err(e) = self.select(selected) { self.message = e.to_string(); }
                let create = ui.add_enabled(enabled && self.instances.len() < MAX_INSTANCES, egui::Button::new("新建流程"));
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("image-instance-create"),create.rect.intersect(ui.clip_rect())));
                if create.clicked() && let Err(e) = self.create() { self.message = e.to_string(); }
                let close = ui.add_enabled(enabled && !self.busy(), egui::Button::new("关闭当前…")).on_hover_text("确认后丢弃此实例内存工作；已保存文件保留。运行中请先取消并等待收尾。");
                #[cfg(feature = "ui-preview")]
                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-instance-close"),close.rect.intersect(ui.clip_rect())));
                if close.clicked() && let Err(e) = self.request_close() { self.message=e.to_string(); }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("名称");
                ui.add_enabled(unlocked && self.close_confirm.is_none(), egui::TextEdit::singleline(&mut self.instances[self.active].title).char_limit(40).desired_width(180.0));
                ui.small(format!("{} / {}份 · 输入、步骤、结果与后台任务分别保留",self.instances.len(),MAX_INSTANCES));
            });
            ui.small("切换不会自动运行或保存。每份流程结果预算256MiB，多份大图会累积占用内存；用完后保存并关闭。");
            if let Some(id) = &self.close_confirm {
                let title = self.instances.iter().find(|i| &i.id == id).map(|i| i.title.clone()).unwrap_or_default();
                ui.label(format!("关闭「{title}」？将丢弃该实例的图片、步骤、结果和未应用导入；已保存文件保留。"));
                ui.horizontal_wrapped(|ui| {
                    let confirm = ui.button("确认关闭此实例");
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-instance-close-confirm"),confirm.rect.intersect(ui.clip_rect())));
                    if confirm.clicked() && let Err(e) = self.close() { self.message = e.to_string(); }
                    let keep = ui.button("保留实例");
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-instance-close-keep"),keep.rect.intersect(ui.clip_rect())));
                    if keep.clicked() { self.close_confirm = None; }
                });
            }
            if !self.message.is_empty() { ui.label(&self.message); }
        });
        let id = self.instances[self.active].id.clone();
        let editable = unlocked && self.close_confirm.is_none();
        egui::ScrollArea::vertical()
            .id_salt(("image-instance-content", id.clone()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_enabled_ui(editable, |ui| {
                    ui.push_id(id, |ui| self.deref_mut().ui(ui));
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_jobs_poll_original_owner_and_close_requires_terminal_confirmation() {
        let mut work = Workspace::default();
        let original_id = work.instances[0].id.clone();
        let image = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            6,
            image::Rgba([1, 2, 3, 120]),
        )));
        work.source = Some(image.clone());
        let original = work.definition.clone();
        let (tx, rx) = mpsc::channel();
        work.receiver = Some(rx);
        assert!(work.request_close().is_err());
        work.create().unwrap();
        work.definition.steps = vec![Step::Info { version: 1 }];
        let second = work.definition.clone();
        assert!(work.source.is_none());
        assert!(work.background_active());
        tx.send(Ok(Reply::Run(
            execute(&original, image.clone(), &AtomicBool::new(false)).unwrap(),
        )))
        .unwrap();
        work.poll(&egui::Context::default());
        assert_eq!(work.definition, second);
        assert!(work.run.is_none());
        assert!(work.instances[0].state.run.is_some());
        work.select(0).unwrap();
        assert_eq!(work.instances[0].id, original_id);
        assert!(Arc::ptr_eq(work.source.as_ref().unwrap(), &image));
        work.request_close().unwrap();
        assert!(work.create().is_err());
        assert!(work.select(1).is_err());
        assert!(!work.can_receive());
        work.close_confirm = None;
        assert!(work.run.is_some());
        work.request_close().unwrap();
        work.close().unwrap();
        assert_eq!(work.definition, second);
        assert_eq!(work.instances.len(), 1);
        work.request_close().unwrap();
        work.close().unwrap();
        assert_eq!(work.instances.len(), 1);
        assert!(work.source.is_none());
    }
    #[test]
    fn two_real_workers_preserve_distinct_images_and_results_after_switch() {
        let ctx = egui::Context::default();
        let mut work = Workspace::default();
        let first = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            12,
            8,
            image::Rgba([200, 10, 20, 90]),
        )));
        let second = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            6,
            4,
            image::Rgba([20, 30, 200, 255]),
        )));
        let def = Definition {
            steps: vec![
                Step::Encode {
                    version: 1,
                    format: Encoding::Webp,
                    quality: 80,
                },
                Step::Info { version: 1 },
            ],
            ..Definition::default()
        };
        work.source = Some(first.clone());
        work.definition = def.clone();
        let original = first.clone();
        let definition = def.clone();
        work.launch(&ctx, Kind::Run, move |cancel| {
            Ok(Reply::Run(execute(&definition, original, cancel)?))
        });
        assert!(work.request_close().is_err());
        work.create().unwrap();
        work.source = Some(second.clone());
        work.definition = def.clone();
        let original = second.clone();
        work.launch(&ctx, Kind::Run, move |cancel| {
            Ok(Reply::Run(execute(&def, original, cancel)?))
        });
        work.select(0).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while work.instances.iter().any(|i| i.state.busy()) {
            assert!(std::time::Instant::now() < deadline);
            work.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        for (index, expected) in [first, second].iter().enumerate() {
            let state = &work.instances[index].state;
            assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), expected));
            let run = state.run.as_ref().unwrap();
            assert!(run.failure.is_none() && !run.cancelled);
            assert_eq!(run.outputs[1].image.to_rgba8(), expected.to_rgba8());
            assert_eq!(
                image::load_from_memory(run.outputs[1].encoded.as_ref().unwrap())
                    .unwrap()
                    .to_rgba8(),
                expected.to_rgba8()
            );
        }
        assert_eq!(work.active, 0);
        assert!(!work.background_active());
    }
    #[test]
    fn inactive_import_and_definition_receipts_are_not_lost() {
        let mut work = Workspace::default();
        work.receive_definition(Definition::default()).unwrap();
        work.instances[0].state.loaded = Some(
            crate::workflow_document::Document::Image(Definition::default())
                .metadata(&std::env::temp_dir().join("saved-image.json")),
        );
        work.create().unwrap();
        assert!(work.background_active());
        assert!(work.can_receive());
        assert!(work.take_loaded().is_some());
        assert!(work.take_loaded().is_none());
        work.select(0).unwrap();
        assert!(work.pending_import());
        assert!(!work.can_receive());
        let metadata = crate::workflow_document::Document::Image(Definition::default())
            .metadata(&std::env::temp_dir().join("closed-saved-image.json"));
        work.instances[0].state.loaded = Some(metadata.clone());
        work.request_close().unwrap();
        work.close().unwrap();
        assert_eq!(work.take_loaded().unwrap().path, metadata.path);
        assert!(work.take_loaded().is_none());
        while work.instances.len() < MAX_INSTANCES {
            work.create().unwrap();
        }
        assert!(work.create().is_err());
    }
}

#[cfg(feature = "ui-preview")]
mod task_preview;
#[cfg(test)]
mod task_tests;

#[cfg(test)]
mod relay_tests;

#[cfg(feature = "ui-preview")]
mod relay_preview;

#[cfg(test)]
mod memory_tests;
