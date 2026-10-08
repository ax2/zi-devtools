//! Explicit image recipe view and owned background jobs.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
mod core;
mod workspace;
pub(crate) use core::Definition;
use core::{Encoding, RECIPE_LIMIT, Run, Step, execute};
pub(super) use workspace::Workspace;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default)]
enum Kind {
    Input,
    ReadDefinition,
    SaveDefinition,
    #[default]
    Run,
    SaveImage,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Input => "图片输入读取",
            Self::ReadDefinition => "图片流程读取",
            Self::SaveDefinition => "图片流程保存",
            Self::Run => "图片流程运行",
            Self::SaveImage => "图片结果保存",
        }
    }
}
enum Reply {
    Input(Arc<DynamicImage>, String),
    Run(Run),
    Definition(Definition, crate::preferences::SavedWorkflow),
    SavedDefinition(crate::preferences::SavedWorkflow),
    Saved(String),
}
pub(crate) struct State {
    definition: Definition,
    source: Option<Arc<DynamicImage>>,
    origins: Vec<relay::Origin>,
    name: String,
    run: Option<Run>,
    selected: usize,
    texture: Option<egui::TextureHandle>,
    receiver: Option<mpsc::Receiver<Result<Reply, String>>>,
    cancel: Arc<AtomicBool>,
    import: Option<Definition>,
    message: String,
    loaded: Option<crate::preferences::SavedWorkflow>,
    completed_tasks: std::collections::VecDeque<crate::tasks::Row>,
    job: crate::tasks::Job,
    kind: Kind,
}
impl Default for State {
    fn default() -> Self {
        Self {
            definition: Definition::default(),
            source: None,
            origins: vec![],
            name: String::new(),
            run: None,
            selected: 0,
            texture: None,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            import: None,
            message: String::new(),
            loaded: None,
            completed_tasks: Default::default(),
            job: Default::default(),
            kind: Kind::default(),
        }
    }
}
impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_fixture(&mut self) {
        self.source = Some(Arc::new(DynamicImage::ImageRgba8(
            image::RgbaImage::from_fn(320, 180, |x, y| {
                image::Rgba([x as u8, y as u8, 140, if x < 60 { 0 } else { 180 }])
            }),
        )));
        self.name = "透明图片流程示例".into();
        self.definition.steps = vec![
            Step::Crop {
                version: 1,
                x: 2500,
                y: 0,
                width: 5000,
                height: 10000,
            },
            Step::Resize {
                version: 1,
                max_width: 120,
            },
            Step::Encode {
                version: 1,
                format: Encoding::Webp,
                quality: 80,
            },
            Step::Info { version: 1 },
        ];
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_definition(&self) -> Vec<u8> {
        self.definition.bytes().unwrap()
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_ready(&self) -> bool {
        !self.busy() && self.run.is_some()
    }
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_check(&self, phase: u8) {
        let run = self.run.as_ref().unwrap();
        assert!(run.failure.is_none() && !run.cancelled);
        assert_eq!(run.outputs.len(), 4);
        let source = self.source.as_ref().unwrap();
        assert_eq!(source.dimensions(), (320, 180));
        let expected =
            source
                .crop_imm(80, 0, 160, 180)
                .resize_exact(120, 135, FilterType::Lanczos3);
        let out = &run.outputs[3];
        assert_eq!(out.image.to_rgba8(), expected.to_rgba8());
        assert_eq!(
            image::load_from_memory(out.encoded.as_ref().unwrap())
                .unwrap()
                .to_rgba8(),
            expected.to_rgba8()
        );
        assert_eq!(self.selected, if phase == 1 { 3 } else { 0 });
        assert!(self.relay_source().is_some());
    }
    pub(super) fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub(super) fn pending_import(&self) -> bool {
        self.import.is_some()
    }
    pub(super) fn receive_definition(&mut self, definition: Definition) -> Result<()> {
        ensure!(
            !self.busy() && !self.pending_import(),
            "请先完成或取消图片流程当前任务和导入"
        );
        definition.validate()?;
        self.import = Some(definition);
        Ok(())
    }
    pub(super) fn take_loaded(&mut self) -> Option<crate::preferences::SavedWorkflow> {
        self.loaded.take()
    }
    pub(super) fn target_state(&self) -> (bool, bool) {
        (
            self.busy() || self.import.is_some(),
            self.source.is_some() || self.run.is_some(),
        )
    }
    pub(super) fn receive(&mut self, value: &relay::Prepared) {
        self.source = Some(value.image.clone());
        self.origins = value.origins.clone();
        self.name = "接力图片".into();
        self.invalidate();
        self.message = "已接收输入，流程步骤保留；点击运行生成结果。".into();
    }
    fn invalidate(&mut self) {
        self.run = None;
        self.texture = None;
        self.selected = 0;
    }
    fn launch(
        &mut self,
        ctx: &egui::Context,
        kind: Kind,
        job: impl FnOnce(&AtomicBool) -> Result<Reply> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        self.job.begin();
        self.kind = kind;
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = job(&cancel)
                .map_err(|_| "图片流程操作失败：格式、版本、文件或容量不支持；原工作保留。".into());
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.receiver else { return };
        let result = match rx.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(_) => Err("后台任务提前结束，原工作保留".into()),
        };
        self.receiver = None;
        if self.cancel.load(Ordering::Relaxed)
            && matches!(&result, Ok(Reply::Input(..) | Reply::Definition(..)))
        {
            self.message = "已取消载入；原工作保留".into();
            self.finish_task(crate::tasks::Phase::Cancelled, "载入已取消，原工作保留");
            return;
        }
        use crate::tasks::Phase;
        let (phase, summary) = match &result {
            Ok(Reply::Run(run)) if run.cancelled => (
                Phase::Cancelled,
                format!("已取消，保留{}步结果", run.outputs.len()),
            ),
            Ok(Reply::Run(run)) if run.failure.is_some() => (
                Phase::Failed,
                format!("流程失败，保留{}步结果；打开实例查看", run.outputs.len()),
            ),
            Ok(Reply::Run(run)) => (
                Phase::Done,
                format!("完成{}步；未自动保存", run.outputs.len()),
            ),
            Ok(Reply::Input(..)) => (Phase::Done, "输入已读取，未运行".into()),
            Ok(Reply::Definition(..)) => (Phase::Done, "定义已读取，待应用；未运行".into()),
            Ok(Reply::SavedDefinition(..) | Reply::Saved(..)) => {
                (Phase::Done, "文件已主动保存".into())
            }
            Err(_) if self.cancel.load(Ordering::Relaxed) => {
                (Phase::Cancelled, "操作已取消，原工作保留".into())
            }
            Err(_) => (Phase::Failed, "操作失败，打开实例查看".into()),
        };
        self.finish_task(phase, summary);
        match result {
            Ok(Reply::Input(image, name)) => {
                self.source = Some(image);
                self.origins.clear();
                self.name = name;
                self.invalidate();
                self.message = "输入已载入，未运行流程".into();
            }
            Ok(Reply::Run(run)) => {
                self.message = if run.cancelled {
                    "已取消，已完成步骤保留".into()
                } else if let Some(e) = &run.failure {
                    e.clone()
                } else {
                    format!("已完成 {} 步；未自动保存", run.outputs.len())
                };
                self.selected = run.outputs.len().saturating_sub(1);
                self.texture = None;
                self.run = Some(run);
            }
            Ok(Reply::Definition(def, metadata)) => {
                self.import = Some(def);
                self.loaded = Some(metadata);
            }
            Ok(Reply::SavedDefinition(metadata)) => {
                self.message = format!("流程定义已另存：{}", metadata.name);
                self.loaded = Some(metadata);
            }
            Ok(Reply::Saved(name)) => self.message = format!("已另存：{name}"),
            Err(e) => self.message = e,
        }
    }
    fn finish_task(&mut self, phase: crate::tasks::Phase, summary: impl Into<String>) {
        if !self.job.phase.active() {
            return;
        }
        self.job.finish(phase, summary);
        if let Some(row) = self.task_snapshot() {
            self.completed_tasks.push_back(row);
        }
    }
    fn request_cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.job.cancelling();
    }
    fn task_snapshot(&self) -> Option<crate::tasks::Row> {
        self.job.snapshot("image-workflow", self.kind.label(), true)
    }
    pub(super) fn relay_source(&self) -> Option<(relay::Source, Vec<relay::Origin>, &'static str)> {
        if self.busy() || self.import.is_some() {
            return None;
        }
        let out = self.run.as_ref()?.outputs.get(self.selected)?;
        Some((
            out.encoded
                .as_ref()
                .map(|b| relay::Source::Encoded(b.clone()))
                .unwrap_or_else(|| relay::Source::Image(out.image.clone())),
            self.origins.clone(),
            "image-workflow",
        ))
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui.ctx());
        ui.heading("图片流程");
        ui.label("每一步作用于上一步实际图片；报告作为附加结果保留。流程文件只保存步骤，不保存图片或权限。");
        let busy = self.busy();
        ui.add_enabled_ui(!busy && self.import.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("选择输入图片…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("图片", &["png", "jpg", "jpeg", "webp"])
                        .pick_file()
                {
                    self.launch(ui.ctx(), Kind::Input, move |cancel| {
                        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
                        let material = crate::material_files::FileMaterial::selected(
                            &path,
                            MAX_INPUT_BYTES as usize,
                        )?;
                        let bytes = material.read_bytes(MAX_INPUT_BYTES as usize)?;
                        let prepared = relay::prepare(
                            relay::Source::Encoded(Arc::new(bytes)),
                            vec![],
                            "image-workflow",
                        )?;
                        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
                        Ok(Reply::Input(
                            prepared.image,
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        ))
                    });
                }
                if ui.button("载入流程…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("图片流程JSON", &["json"])
                        .pick_file()
                {
                    self.launch(ui.ctx(), Kind::ReadDefinition, move |cancel| {
                        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
                        let m = crate::material_files::FileMaterial::selected(&path, RECIPE_LIMIT)?;
                        let bytes = m.read_bytes(RECIPE_LIMIT)?;
                        ensure!(!cancel.load(Ordering::Relaxed), "已取消");
                        let definition = Definition::parse(&bytes)?;
                        let metadata =
                            crate::workflow_document::Document::Image(definition.clone())
                                .metadata(&path);
                        Ok(Reply::Definition(definition, metadata))
                    });
                }
                if ui.button("保存流程定义…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_file_name("image-workflow.json")
                        .save_file()
                {
                    let def = self.definition.clone();
                    self.launch(ui.ctx(), Kind::SaveDefinition, move |c| {
                        let b = def.bytes()?;
                        crate::local_files::save_new_moved(&path, &b, c)?;
                        Ok(Reply::SavedDefinition(
                            crate::workflow_document::Document::Image(def).metadata(&path),
                        ))
                    });
                }
            });
        });
        if let Some(def) = self.import.clone() {
            ui.group(|ui| {
                ui.label(format!(
                    "待应用：{}步。将替换现有步骤并清除旧结果；输入图片保留，不自动运行。",
                    def.steps.len()
                ));
                ui.horizontal(|ui| {
                    let apply = ui.button("确认应用导入流程");
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new("image-flow-import-apply"),
                            apply.rect.intersect(ui.clip_rect()),
                        )
                    });
                    if apply.clicked() {
                        self.definition = def;
                        self.import = None;
                        self.invalidate();
                    }
                    if ui.button("取消导入").clicked() {
                        self.import = None;
                    }
                });
            });
        }
        if let Some(image) = &self.source {
            ui.label(format!(
                "{} · {}×{}",
                self.name,
                image.width(),
                image.height()
            ));
        } else {
            ui.small("可选择文件，或从其他图片工具接力输入。");
        }
        let mut changed = false;
        ui.add_enabled_ui(!self.busy() && self.import.is_none(), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("image-flow-steps")
                .max_height(220.0)
                .show(ui, |ui| {
                    let mut remove = None;
                    let mut up = None;
                    for (index, step) in self.definition.steps.iter_mut().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{}. {} v1", index + 1, step.label()));
                            match step {
                                Step::Crop {
                                    x,
                                    y,
                                    width,
                                    height,
                                    ..
                                } => {
                                    for (name, v) in
                                        [("X", x), ("Y", y), ("宽", width), ("高", height)]
                                    {
                                        ui.label(name);
                                        changed |= ui
                                            .add(
                                                egui::DragValue::new(v)
                                                    .range(0..=10000)
                                                    .speed(25)
                                                    .custom_formatter(|value, _| {
                                                        format!("{:.1}%", value / 100.0)
                                                    })
                                                    .custom_parser(|text| {
                                                        text.trim()
                                                            .trim_end_matches('%')
                                                            .parse::<f64>()
                                                            .ok()
                                                            .filter(|v| v.is_finite())
                                                            .map(|v| (v * 100.0).round())
                                                    }),
                                            )
                                            .changed();
                                    }
                                }
                                Step::Resize { max_width, .. } => {
                                    changed |= ui
                                        .add(
                                            egui::DragValue::new(max_width)
                                                .range(1..=12000)
                                                .suffix(" px"),
                                        )
                                        .changed()
                                }
                                Step::Encode {
                                    format, quality, ..
                                } => {
                                    egui::ComboBox::from_id_salt(("image-flow-format", index))
                                        .selected_text(format.native().label())
                                        .show_ui(ui, |ui| {
                                            for v in [Encoding::Png, Encoding::Jpeg, Encoding::Webp]
                                            {
                                                changed |= ui
                                                    .selectable_value(format, v, v.native().label())
                                                    .changed();
                                            }
                                        });
                                    if *format == Encoding::Jpeg {
                                        changed |= ui
                                            .add(egui::Slider::new(quality, 35..=95).text("质量"))
                                            .changed();
                                    }
                                }
                                Step::Info { .. } => {
                                    ui.small("实际尺寸、透明通道与已有编码字节");
                                }
                            }
                            if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                                up = Some(index);
                            }
                            if ui.small_button("删除").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        self.definition.steps.remove(i);
                        changed = true;
                    } else if let Some(i) = up {
                        self.definition.steps.swap(i, i - 1);
                        changed = true;
                    }
                });
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(self.definition.steps.len() < 16, |ui| {
                    for step in [
                        Step::Crop {
                            version: 1,
                            x: 0,
                            y: 0,
                            width: 10000,
                            height: 10000,
                        },
                        Step::Resize {
                            version: 1,
                            max_width: 1600,
                        },
                        Step::Encode {
                            version: 1,
                            format: Encoding::Webp,
                            quality: 80,
                        },
                        Step::Info { version: 1 },
                    ] {
                        if ui.button(format!("+ {}", step.label())).clicked() {
                            self.definition.steps.push(step);
                            changed = true;
                        }
                    }
                });
            });
        });
        if changed {
            self.invalidate();
            self.message = "步骤已改变，请重新运行".into();
        }
        if let Err(error) = self.definition.validate() {
            ui.colored_label(ui.visuals().error_fg_color, error.to_string());
        }
        ui.horizontal(|ui| {
            let run = ui.add_enabled(
                !self.busy()
                    && self.source.is_some()
                    && self.import.is_none()
                    && self.definition.validate().is_ok(),
                egui::Button::new("运行并预览"),
            );
            #[cfg(feature = "ui-preview")]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("image-flow-run"), run.rect));
            if run.clicked() {
                let def = self.definition.clone();
                let image = self.source.clone().unwrap();
                self.launch(ui.ctx(), Kind::Run, move |cancel| {
                    Ok(Reply::Run(execute(&def, image, cancel)?))
                });
            }
            if self.busy() && ui.button("取消当前操作").clicked() {
                self.request_cancel();
            }
        });
        ui.label(&self.message);
        if let Some(run) = &self.run {
            ui.horizontal_wrapped(|ui| {
                for (i, out) in run.outputs.iter().enumerate() {
                    let response = ui.selectable_value(
                        &mut self.selected,
                        i,
                        format!("{} {}", i + 1, out.label),
                    );
                    #[cfg(feature = "ui-preview")]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new(format!("image-flow-step-{i}")), response.rect)
                    });
                    if response.changed() {
                        self.texture = None;
                    }
                }
            });
            if let Some(out) = run.outputs.get(self.selected) {
                ui.label(format!(
                    "{}×{} · 编码：{}",
                    out.image.width(),
                    out.image.height(),
                    out.encoded
                        .as_ref()
                        .map(|b| format!("{} 字节", b.len()))
                        .unwrap_or_else(|| "未编码，不能估算文件大小".into())
                ));
                let texture = self.texture.get_or_insert_with(|| {
                    ui.ctx().load_texture(
                        "image-flow-result",
                        preview_image(&out.image),
                        egui::TextureOptions::LINEAR,
                    )
                });
                let size = texture.size_vec2();
                let ratio = (ui.available_width() / size.x).min(220.0 / size.y).min(1.0);
                ui.image((texture.id(), size * ratio));
                if let Some(report) = &out.report {
                    egui::CollapsingHeader::new("信息报告JSON").show(ui, |ui| {
                        ui.monospace(report);
                    });
                }
                let encoded = out.encoded.clone();
                if ui
                    .add_enabled(
                        !self.busy() && encoded.is_some(),
                        egui::Button::new("另存选中编码结果…"),
                    )
                    .clicked()
                    && let Some(bytes) = encoded
                {
                    let format = image::guess_format(&bytes).ok();
                    let extension = match format {
                        Some(ImageFormat::Png) => "png",
                        Some(ImageFormat::Jpeg) => "jpg",
                        _ => "webp",
                    };
                    if let Some(path) = rfd::FileDialog::new()
                        .set_file_name(format!("image-flow-result.{extension}"))
                        .save_file()
                    {
                        self.launch(ui.ctx(), Kind::SaveImage, move |c| {
                            crate::local_files::save_new_moved(&path, &bytes, c)?;
                            Ok(Reply::Saved(
                                path.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                            ))
                        });
                    }
                }
            }
        }
        ui.small("最多16步；累计保留结果≤256MiB。PNG/WebP无损，JPEG质量可调。改步骤后旧结果失效；另存不覆盖已有文件。");
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
