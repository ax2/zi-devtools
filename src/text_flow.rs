//! Versioned native text actions and serial execution. No implicit file/network actions.
use crate::tasks::{Job, Phase};
use anyhow::{Result, bail, ensure};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

const LIMIT: usize = 1024 * 1024;
const TRACE_LIMIT: usize = 8 * LIMIT;
const STEPS: usize = 16;

mod files;
pub(crate) mod material;
mod picker;
enum Reply {
    Run(Run),
    Input(String),
    File(crate::material_files::FileMaterial),
    Definition(std::path::PathBuf, Definition),
    Saved(std::path::PathBuf, usize),
}
pub use material::{ACTIONS, Action, Effect, Kind, Material, OutputPort};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: String,
    pub version: u32,
}
impl Step {
    fn registered(&self) -> Result<&'static Action> {
        ACTIONS
            .iter()
            .find(|a| a.id == self.action && a.version == self.version)
            .ok_or_else(|| anyhow::anyhow!("未知或不兼容的操作：{} v{}", self.action, self.version))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u32,
    pub steps: Vec<Step>,
}
impl Default for Definition {
    fn default() -> Self {
        Self {
            version: 1,
            steps: vec![],
        }
    }
}
impl Definition {
    fn output_kind(&self) -> Result<Kind> {
        self.validate()?;
        match self.steps.last() {
            Some(step) => Ok(step.registered()?.output),
            None => self.start_kind(),
        }
    }
    pub(crate) fn start_kind(&self) -> Result<Kind> {
        if self.version >= 3 {
            if let Some(first) = self.steps.first() {
                let action = first.registered()?;
                if action.input == Kind::File {
                    ensure!(
                        action.id == "file.sha256",
                        "文件引用只能从文件 SHA-256 操作开始"
                    );
                    return Ok(Kind::File);
                }
                if action.input == Kind::Image {
                    ensure!(
                        self.version >= 4 && action.id == "image.info",
                        "图片材料只能从图片信息操作开始"
                    );
                    return Ok(Kind::Image);
                }
            }
        }
        Ok(Kind::Text)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(matches!(self.version, 1..=4), "不支持此流程版本");
        ensure!(self.steps.len() <= STEPS, "文本流程最多16步");
        let mut kind = self.start_kind()?;
        for (index, step) in self.steps.iter().enumerate() {
            let action = step.registered()?;
            let mut ports = std::collections::BTreeSet::from(["result"]);
            for port in action.side_outputs {
                ensure!(
                    !port.id.trim().is_empty()
                        && !port.label.trim().is_empty()
                        && ports.insert(port.id),
                    "第{}步 {} 输出端口定义无效或重复",
                    index + 1,
                    action.label
                );
            }
            ensure!(
                self.version >= 2 || !action.typed(),
                "第{}步需要流程格式2或3",
                index + 1
            );
            ensure!(
                action.input == kind,
                "第{}步 {}需要{}，上一步输出为{}",
                index + 1,
                action.label,
                action.input.label(),
                kind.label()
            );
            kind = action.output;
        }
        Ok(())
    }
}
#[derive(Debug, Default)]
pub struct Run {
    pub outputs: Vec<Material>,
    pub materials: Vec<RunMaterial>,
    pub failure: Option<String>,
    pub cancelled: bool,
}
#[derive(Debug)]
pub struct RunMaterial {
    pub step: usize,
    pub port: &'static str,
    pub label: &'static str,
    pub material: Material,
}
pub fn execute(definition: &Definition, input: &str, cancel: &AtomicBool) -> Result<Run> {
    ensure!(input.len() <= LIMIT, "输入最多1 MiB");
    execute_material(definition, Material::Text(input.to_owned()), cancel)
}
pub fn execute_material(
    definition: &Definition,
    input: Material,
    cancel: &AtomicBool,
) -> Result<Run> {
    definition.validate()?;
    ensure!(!definition.steps.is_empty(), "请先添加操作步骤");
    ensure!(
        input.kind() == definition.start_kind()?,
        "流程需要{}输入，当前材料为{}",
        definition.start_kind()?.label(),
        input.kind().label()
    );
    if let Material::Text(text) = &input {
        ensure!(text.len() <= LIMIT, "输入最多1 MiB");
    }
    let mut run = Run::default();
    let mut current = input;
    let mut bytes = 0usize;
    for (index, step) in definition.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            run.cancelled = true;
            break;
        }
        let action = step.registered()?;
        match action.execute(&current, cancel) {
            Ok(output) => {
                let size = output.bytes()?;
                if size > LIMIT || bytes.saturating_add(size) > TRACE_LIMIT {
                    run.failure = Some(format!(
                        "第{}步 {}：结果超过单步1 MiB或累计8 MiB，后续步骤未运行",
                        index + 1,
                        action.label
                    ));
                    break;
                }
                bytes += size;
                run.materials.push(RunMaterial {
                    step: index + 1,
                    port: "result",
                    label: action.label,
                    material: output.clone(),
                });
                for port in action.side_outputs {
                    let material = match action.side_material(port, &current) {
                        Ok(material) => material,
                        Err(error) => {
                            run.failure = Some(format!(
                                "第{}步 {}：{}；后续步骤未运行",
                                index + 1,
                                action.label,
                                error
                            ));
                            break;
                        }
                    };
                    if material.kind() != port.kind {
                        run.failure = Some(format!(
                            "第{}步输出端口 {} 的类型配置不匹配；后续步骤未运行",
                            index + 1,
                            port.id
                        ));
                        break;
                    }
                    run.materials.push(RunMaterial {
                        step: index + 1,
                        port: port.id,
                        label: port.label,
                        material,
                    });
                }
                if run.failure.is_some() {
                    break;
                }
                current = output.clone();
                run.outputs.push(output);
            }
            Err(error) => {
                run.failure = Some(format!(
                    "第{}步 {}：{error:#}；后续步骤未运行",
                    index + 1,
                    action.label
                ));
                break;
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        run.cancelled = true;
    }
    Ok(run)
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    input: String,
    definition: Definition,
    #[serde(skip)]
    picker: picker::Picker,
    #[serde(skip)]
    picker_open: bool,
    #[serde(skip)]
    result: Option<Run>,
    #[serde(skip)]
    input_file: Option<crate::material_files::FileMaterial>,
    #[serde(skip)]
    input_image: Option<crate::image_tools::ImageMaterial>,
    #[serde(skip)]
    receiver: Option<Receiver<std::result::Result<Reply, String>>>,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
    #[serde(skip)]
    pub(crate) job: Job,
    #[serde(skip)]
    message: String,
    #[serde(skip)]
    send: Option<String>,
    #[serde(skip)]
    table_send: Option<crate::workbench::Dataset>,
    #[serde(skip)]
    file_send: Option<crate::material_files::FileMaterial>,
    #[serde(skip)]
    preview_cache: Option<(usize, String)>,
    #[serde(skip)]
    recipe: String,
    #[serde(skip)]
    review: Option<Definition>,
    #[serde(skip)]
    input_review: Option<String>,
    #[serde(skip)]
    pub(crate) loaded: Option<crate::preferences::SavedWorkflow>,
    #[serde(skip)]
    output_review: Option<files::Output>,
    #[serde(skip)]
    reveal: bool,
    #[serde(skip)]
    scroll_until: Option<std::time::Instant>,
    #[serde(skip)]
    selected: usize,
    #[cfg(feature = "ui-preview")]
    #[serde(skip)]
    buttons: [Option<(egui::Rect, egui::Rect)>; 15],
    #[cfg(feature = "ui-preview")]
    #[serde(skip)]
    output_buttons: Vec<(egui::Rect, egui::Rect)>,
    #[cfg(feature = "ui-preview")]
    #[serde(skip)]
    file_path: Option<std::path::PathBuf>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(crate) fn snapshot_clone(&self) -> Self {
        let mut snapshot = Self::default();
        snapshot.input = self.input.clone();
        snapshot.definition = self.definition.clone();
        snapshot
    }

    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_image_input_check(&self) {
        assert_eq!(self.definition.version, 4);
        assert_eq!(self.definition.start_kind().unwrap(), Kind::Image);
        assert_eq!(
            self.definition
                .steps
                .first()
                .map(|step| step.action.as_str()),
            Some("image.info")
        );
        assert!(self.input_image.is_some());
        assert!(self.result.is_none());
        assert!(!self.busy());
        assert!(self.message.contains("已接收内存图片"));
    }

    pub(crate) fn open(&mut self) {
        self.reveal = true;
        self.scroll_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.review.is_some() || self.input_review.is_some() || self.output_review.is_some()
    }
    pub(crate) fn receive_recipe(&mut self, definition: Definition) -> Result<()> {
        definition.validate()?;
        ensure!(
            !self.busy() && !self.modal_open(),
            "请先完成当前工具流程或审核"
        );
        self.review = Some(definition);
        self.open();
        Ok(())
    }
    pub(crate) fn receive(&mut self, input: String) -> Result<()> {
        ensure!(input.len() <= LIMIT, "文本流程输入最多1 MiB");
        ensure!(!self.busy(), "文本流程正在运行");
        let input_kind = self.definition.start_kind()?;
        let replacing_material_input = matches!(input_kind, Kind::File | Kind::Image);
        if replacing_material_input {
            let mut definition = self.definition.clone();
            let first_action = match input_kind {
                Kind::File => "file.sha256",
                Kind::Image => "image.info",
                _ => unreachable!(),
            };
            ensure!(
                definition
                    .steps
                    .first()
                    .is_some_and(|step| step.action == first_action),
                "当前流程材料输入首步无效"
            );
            definition.steps.remove(0);
            definition.version = if definition
                .steps
                .iter()
                .any(|step| step.registered().is_ok_and(Action::typed))
            {
                2
            } else {
                1
            };
            definition.validate()?;
            self.definition = definition;
            self.input_file = None;
            self.input_image = None;
        }
        self.input = input;
        self.invalidate();
        if replacing_material_input {
            self.message = "已切换为文本输入；材料专用首步已移除，其余步骤保留。".into();
        }
        self.open();
        Ok(())
    }
    pub(crate) fn receive_file(&mut self, file: crate::material_files::FileMaterial) -> Result<()> {
        ensure!(
            !self.busy() && !self.modal_open(),
            "请先完成当前流程任务或审核"
        );
        let mut definition = self.definition.clone();
        match definition.start_kind()? {
            Kind::File => ensure!(
                definition
                    .steps
                    .first()
                    .is_some_and(|step| step.action == "file.sha256"),
                "当前文件流程缺少 SHA-256 首步"
            ),
            Kind::Text => {
                ensure!(
                    definition.steps.len() < STEPS,
                    "当前流程已达16步，无法插入文件校验"
                );
                definition.version = 3;
                definition.steps.insert(
                    0,
                    Step {
                        action: "file.sha256".into(),
                        version: 1,
                    },
                );
            }
            Kind::Table => bail!("当前流程以表格材料开始，不能接收文件引用"),
            Kind::Image => bail!("当前流程需要图片材料；请从图片工具明确接力"),
        }
        definition.validate()?;
        self.definition = definition;
        self.input.clear();
        self.input_file = Some(file);
        self.input_image = None;
        self.invalidate();
        self.message = "已接收文件引用；点击运行后才会分块读取并计算 SHA-256。".into();
        self.open();
        Ok(())
    }
    pub(crate) fn receive_image(&mut self, image: crate::image_tools::ImageMaterial) -> Result<()> {
        ensure!(
            !self.busy() && !self.modal_open(),
            "请先完成当前流程任务或审核"
        );
        ensure!(
            image.image.width() > 0
                && image.image.height() > 0
                && image.image.width() <= 12000
                && image.image.height() <= 12000
                && u64::from(image.image.width()) * u64::from(image.image.height())
                    <= crate::image_tools::MAX_PIXELS
                && image.image.as_bytes().len() <= 192 * 1024 * 1024,
            "接收图片超出尺寸或内存限制"
        );
        self.definition = Definition {
            version: 4,
            steps: vec![Step {
                action: "image.info".into(),
                version: 1,
            }],
        };
        self.input.clear();
        self.input_file = None;
        self.input_image = Some(image);
        self.invalidate();
        self.message = "已接收内存图片；点击运行生成信息报告，不会自动运行或保存。".into();
        self.open();
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn has_content(&self) -> bool {
        !self.input.is_empty()
            || self.input_file.is_some()
            || self.input_image.is_some()
            || !self.definition.steps.is_empty()
            || self.modal_open()
    }
    pub fn validate(&self) -> Result<()> {
        self.definition.validate()?;
        match self.definition.start_kind()? {
            Kind::Text => {
                ensure!(self.input.len() <= LIMIT, "文本流程输入最多1 MiB");
                ensure!(self.input_file.is_none(), "请先切换到文本输入")
            }
            Kind::File => {}
            Kind::Image => {}
            Kind::Table => bail!("当前流程需要表格材料输入"),
        }
        Ok(())
    }
    fn validate_run(&self) -> Result<()> {
        self.validate()?;
        if self.definition.start_kind()? == Kind::File {
            ensure!(
                self.input_file.is_some(),
                "此配方需要文件引用；请选择文件或从其他工具接力"
            );
        }
        if self.definition.start_kind()? == Kind::Image {
            ensure!(
                self.input_image.is_some(),
                "此流程需要图片材料；请从图片工具重新接力"
            );
        }
        Ok(())
    }
    pub fn take_table_send(&mut self) -> Option<crate::workbench::Dataset> {
        self.table_send.take()
    }
    pub fn take_file_send(&mut self) -> Option<crate::material_files::FileMaterial> {
        self.file_send.take()
    }
    fn send_selected(&mut self) {
        let Some(material) = self
            .result
            .as_ref()
            .and_then(|run| run.materials.get(self.selected))
            .map(|output| output.material.clone())
        else {
            return;
        };
        match material {
            Material::Text(text) => self.send = Some(text),
            Material::Table(data) => self.table_send = Some(data),
            Material::File(file) => self.file_send = Some(file),
            Material::Image(_) => {}
        }
    }
    pub(crate) fn transfer_failed(&mut self, message: String) {
        self.message = message;
        self.open();
    }
    pub fn take_send(&mut self) -> Option<String> {
        self.send.take()
    }
    pub(crate) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.job.cancelling();
    }
    fn invalidate(&mut self) {
        self.result = None;
        self.send = None;
        self.table_send = None;
        self.file_send = None;
        self.input_review = None;
        self.output_review = None;
        self.preview_cache = None;
        self.message.clear();
        self.selected = 0;
    }
    pub(crate) fn start(&mut self) -> Result<()> {
        ensure!(!self.modal_open(), "请先完成或关闭当前审核");
        ensure!(!self.busy(), "请等待当前流程结束");
        self.validate_run()?;
        ensure!(!self.definition.steps.is_empty(), "请先添加操作步骤");
        self.invalidate();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let definition = self.definition.clone();
        let input = if let Some(image) = self.input_image.clone() {
            Material::Image(image)
        } else if let Some(file) = self.input_file.clone() {
            Material::File(file)
        } else {
            Material::Text(self.input.clone())
        };
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(
                execute_material(&definition, input, &cancel)
                    .map(Reply::Run)
                    .map_err(|e| format!("{e:#}")),
            );
        });
        Ok(())
    }
    pub(crate) fn poll(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let reply = match receiver.try_recv() {
            Ok(reply) => reply,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("文本流程任务意外结束".into()),
        };
        self.receiver = None;
        match reply {
            Ok(Reply::Input(_) | Reply::File(_) | Reply::Definition(..))
                if self.cancel.load(Ordering::Relaxed) =>
            {
                self.message = "文件读取已取消；原输入、步骤与结果保留".into();
                self.job.finish(Phase::Cancelled, &self.message);
            }
            Ok(Reply::Run(mut run)) => {
                if self.cancel.load(Ordering::Relaxed) {
                    run.cancelled = true;
                }
                let phase = if run.cancelled {
                    Phase::Cancelled
                } else if run.failure.is_some() {
                    Phase::Failed
                } else {
                    Phase::Done
                };
                self.message = if run.cancelled {
                    "流程已取消；已完成的步骤结果保留，原输入保留".into()
                } else if let Some(error) = &run.failure {
                    error.clone()
                } else {
                    format!(
                        "{}步完成；原输入保留，可选择任一步结果继续处理",
                        run.outputs.len()
                    )
                };
                self.job.finish(phase, &self.message);
                self.selected = run
                    .materials
                    .iter()
                    .rposition(|output| output.port == "result")
                    .unwrap_or(0);
                self.result = Some(run);
                self.open();
            }
            Ok(Reply::Input(input)) => {
                self.input_review = Some(input);
                self.job.finish(Phase::Done, "材料已读取，待确认；未运行");
                self.open();
            }
            Ok(Reply::File(file)) => match self.receive_file(file) {
                Ok(()) => self.job.finish(Phase::Done, "文件引用已选定；尚未读取内容"),
                Err(error) => {
                    self.message = error.to_string();
                    self.job.finish(Phase::Failed, &self.message);
                }
            },
            Ok(Reply::Definition(path, definition)) => {
                self.loaded = Some(
                    crate::workflow_document::Document::Tool(definition.clone()).metadata(&path),
                );
                self.review = Some(definition);
                self.job.finish(Phase::Done, "配方已读取，待确认；未运行");
                self.open();
            }
            Ok(Reply::Saved(path, bytes)) => {
                self.message = format!(
                    "完整文件已保存：{} · {}字节；流程材料与步骤结果保留",
                    path.display(),
                    bytes
                );
                self.job.finish(Phase::Done, "完整文件已保存");
                self.open();
            }
            Err(error) => {
                let phase = if self.cancel.load(Ordering::Relaxed) {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                self.job.finish(phase, &error);
                self.message = error;
            }
        }
    }
    fn file_task(
        &mut self,
        task: impl FnOnce(&AtomicBool) -> Result<Reply> + Send + 'static,
    ) -> Result<()> {
        ensure!(!self.busy() && !self.modal_open(), "请先结束任务或当前审核");
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.message.clear();
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(task(&cancel).map_err(|e| format!("{e:#}")));
        });
        Ok(())
    }
    fn choose_path(&mut self, save: bool, name: &str) -> Option<std::path::PathBuf> {
        #[cfg(feature = "ui-preview")]
        if let Some(path) = self.file_path.take() {
            return Some(path);
        }
        let dialog = rfd::FileDialog::new().set_title(if save {
            "保存流程文件（选择新文件名）"
        } else {
            "读取流程文件（确认后载入）"
        });
        if save {
            dialog.set_file_name(name).save_file()
        } else {
            dialog.pick_file()
        }
    }
    fn choose_input_file(&mut self) -> Option<std::path::PathBuf> {
        #[cfg(feature = "ui-preview")]
        if let Some(path) = self.file_path.take() {
            return Some(path);
        }
        rfd::FileDialog::new()
            .set_title("选择待校验文件；这里只建立引用，运行流程时才读取")
            .pick_file()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        if self.reveal && (self.picker_open || self.definition.steps.is_empty()) {
            self.picker.focus();
        }
        let response = egui::CollapsingHeader::new("操作流程 · 文本 / 表格 / 图片材料")
            .id_salt("text-tool-flow").open(self.reveal.then_some(true)).default_open(self.has_content()).show(ui, |ui| {
            ui.label("每一步接收上一步材料，输入/输出类型必须匹配。原输入保留，可保存当前工作实例；最多16步。");
            let busy = self.busy();
            let mut changed = false;
            ui.add_enabled_ui(!busy, |ui| {
                if self.definition.start_kind().ok() == Some(Kind::Image) {
                    ui.label("图片输入");
                    ui.label(
                        self.input_image
                            .as_ref()
                            .map(|image| Material::Image(image.clone()).summary())
                            .unwrap_or_else(|| "图片输入尚未接入；请从图片工具重新发送图片接力。".into()),
                    );
                } else if self.definition.start_kind().ok() == Some(Kind::File) {
                    ui.horizontal(|ui| {
                        ui.label("文件输入");
                        let select = ui.add_enabled(
                            !self.modal_open(),
                            egui::Button::new("选择文件引用…"),
                        );
                        #[cfg(feature = "ui-preview")]
                        {
                            self.buttons[9] = Some((select.rect, ui.clip_rect()));
                        }
                        if select.clicked()
                            && let Some(path) = self.choose_input_file()
                            && let Err(error) = self.file_task(move |cancel| {
                                ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
                                let file = crate::material_files::FileMaterial::selected(
                                    &path,
                                    usize::MAX,
                                )?;
                                ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
                                Ok(Reply::File(file))
                            })
                        {
                            self.message = error.to_string();
                        }
                    });
                    ui.label(
                        self.input_file
                            .as_ref()
                            .map(|file| Material::File(file.clone()).summary())
                            .unwrap_or_else(|| "尚未选择文件；接收只固定文件身份，点击运行后才读取。".into()),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.label("原输入");
                        let read = ui.add_enabled(
                            !self.modal_open(),
                            egui::Button::new("读取UTF-8文件…"),
                        );
                        #[cfg(feature = "ui-preview")]
                        {
                            self.buttons[9] = Some((read.rect, ui.clip_rect()));
                        }
                        if read.clicked()
                            && let Some(path) = self.choose_path(false, "")
                            && let Err(error) = self.file_task(move |cancel| {
                                files::input(&path, cancel).map(Reply::Input)
                            })
                        {
                            self.message = error.to_string();
                        }
                        let file = ui.add_enabled(
                            !self.modal_open(),
                            egui::Button::new("选择文件引用…"),
                        );
                        #[cfg(feature = "ui-preview")]
                        {
                            self.buttons[14] = Some((file.rect, ui.clip_rect()));
                        }
                        if file.clicked()
                            && let Some(path) = self.choose_input_file()
                            && let Err(error) = self.file_task(move |cancel| {
                                ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
                                let file = crate::material_files::FileMaterial::selected(
                                    &path,
                                    usize::MAX,
                                )?;
                                ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
                                Ok(Reply::File(file))
                            })
                        {
                            self.message = error.to_string();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("text-flow-input")
                        .max_height(75.0)
                        .show(ui, |ui| {
                            changed |= ui
                                .add(
                                    egui::TextEdit::multiline(&mut self.input)
                                        .desired_rows(3)
                                        .desired_width(f32::INFINITY)
                                        .char_limit(LIMIT),
                                )
                                .changed()
                        });
                }
                ui.horizontal(|ui| {
                    if ui.button("添加操作…").clicked() { self.picker_open = !self.picker_open; if self.picker_open { self.picker.focus(); } }
                    let button=ui.add_enabled(self.input.is_empty() && self.definition.steps.is_empty(), egui::Button::new("示例"))
                        .on_hover_text("已有草稿时保留内容；可在新实例中载入示例");
                    #[cfg(feature="ui-preview")] { self.buttons[0]=Some((button.rect,ui.clip_rect())); }
                    if button.clicked() { self.input = "{\"名称\":\"本地工具🦀\",\"数量\":2}".into(); self.definition.steps = ["json.minify","base64.encode","base64.decode","json.pretty"].into_iter().map(|id| Step{action:id.into(),version:1}).collect(); self.open(); changed=true; }
                    let table_button=ui.add_enabled(self.input.is_empty() && self.definition.steps.is_empty(),egui::Button::new("表格示例"));
                    #[cfg(feature="ui-preview")] { self.buttons[3]=Some((table_button.rect,ui.clip_rect())); }
                    if table_button.clicked() { self.input="编号,名称,数量\n001, Zi Tools ,002\n002, 本地工具🦀 ,003".into();self.definition=Definition{version:2,steps:["table.parse_csv","table.trim","table.export_schema","table.parse_schema"].into_iter().map(|id|Step{action:id.into(),version:1}).collect()};self.open();changed=true; }

                });
                if self.picker_open || self.definition.steps.is_empty() {
                let next_kind=self.definition.output_kind().ok();
                if let Some(action)=self.picker.ui(ui,next_kind,self.definition.steps.len()<STEPS) {
                    self.definition.steps.push(Step{action:action.id.into(),version:action.version});
                    if action.input == Kind::File {
                        self.definition.version = 3;
                    } else if action.typed() && self.definition.version < 2 {
                        self.definition.version = 2;
                    }
                    self.picker.added();
                    changed=true;
                }
                }
                let mut edit = None;
                egui::ScrollArea::vertical().id_salt("text-flow-step-list").max_height(125.0).show(ui, |ui| {
                for (index, step) in self.definition.steps.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let action=step.registered().ok();
                        ui.label(format!("{}. {} · v{}",index+1,action.map_or(step.action.as_str(),|a| a.label),step.version)).on_hover_text(action.map_or_else(String::new,|a|format!("{} → {}\n{}\n影响：{}",a.input.label(),a.output.label(),a.note,a.effect.details())));
                        if let Some(action) = action {
                            ui.weak(action.effect.label()).on_hover_text(action.effect.details());
                        }
                        if ui.add_enabled(index>0,egui::Button::new("↑")).clicked() { edit=Some((index,0)); }
                        if ui.add_enabled(index+1<self.definition.steps.len(),egui::Button::new("↓")).clicked() { edit=Some((index,1)); }
                        if ui.button("移除").clicked() { edit=Some((index,2)); }
                    });
                }
                });
                if let Some((index,op))=edit { match op { 0=>self.definition.steps.swap(index,index-1),1=>self.definition.steps.swap(index,index+1),_=>{self.definition.steps.remove(index);} }; changed=true; }
            });
            if changed { self.invalidate(); }
            if let Err(error)=self.definition.validate() { ui.colored_label(ui.visuals().error_fg_color,error.to_string()); }
            ui.horizontal(|ui| {
                let effects = self.definition.steps.iter()
                    .filter_map(|step| step.registered().ok())
                    .map(|action| action.effect.label())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let effect_hint = if effects.is_empty() {
                    "运行只处理本流程材料；不会自动保存或联网。".to_owned()
                } else {
                    format!("本流程会：{}。运行不会自动保存或联网；另存结果需单独确认。", effects.join("、"))
                };
                let button=ui.add_enabled(!busy&&!self.definition.steps.is_empty()&&self.validate_run().is_ok(),egui::Button::new("运行流程")).on_hover_text(effect_hint);
                #[cfg(feature="ui-preview")] { self.buttons[1]=Some((button.rect,ui.clip_rect())); }
                if button.clicked() && let Err(error)=self.start() { self.message=error.to_string(); }
                if ui.add_enabled(busy,egui::Button::new("取消")).on_hover_text("在当前操作返回后停止；已完成结果保留").clicked() { self.cancel(); }
            });
            if busy { ui.horizontal(|ui| { ui.spinner(); ui.label("正在处理流程或文件，原输入保留…"); }); }
            if !self.message.is_empty() {
                if self.job.phase == Phase::Failed { ui.colored_label(ui.visuals().error_fg_color, &self.message); }
                else { ui.label(&self.message); }
            }
            if let Some(run)=&self.result {
                #[cfg(feature="ui-preview")]
                self.output_buttons.clear();
                ui.horizontal_wrapped(|ui| {
                    for (index, output) in run.materials.iter().enumerate() {
                        let response = ui.selectable_value(
                            &mut self.selected,
                            index,
                            format!("第{}步 · {}", output.step, output.label),
                        );
                        #[cfg(feature="ui-preview")]
                        self.output_buttons.push((response.rect, ui.clip_rect()));
                        #[cfg(not(feature="ui-preview"))]
                        let _ = response;
                    }
                });
                if let Some(selected) = run.materials.get(self.selected) {
                    let output = selected.material.clone();
                    ui.label(output.summary());
                    ui.small("文本预览最多8192字节；表格仅前3行×6列，表头256/单元格1024字节；复制与接力使用完整材料。");
                    if self.preview_cache.as_ref().is_none_or(|(index,_)| *index!=self.selected) { self.preview_cache=Some((self.selected,output.preview().unwrap_or_else(|e|e.to_string()))); }
                    let mut preview=self.preview_cache.as_ref().unwrap().1.clone();
                    ui.add_enabled_ui(!busy && self.review.is_none() && self.input_review.is_none() && self.output_review.is_none(),|ui| { ui.horizontal(|ui| {
                        let copy=ui.add_enabled(!matches!(output.kind(), Kind::File | Kind::Image),egui::Button::new("复制完整结果"));
                        if copy.clicked() { match output.render() {Ok(text)=>ui.ctx().copy_text(text),Err(e)=>self.message=e.to_string()} }
                        let button=ui.button(if output.kind()==Kind::Table {"送到数据工作台…"} else if output.kind()==Kind::File {"发送文件引用…"} else {"发送到其他工具…"});
                        #[cfg(feature="ui-preview")] { self.buttons[2]=Some((button.rect,ui.clip_rect())); }
                        if button.clicked() { self.send_selected(); }
                        let save=ui.add_enabled(!matches!(output.kind(), Kind::File | Kind::Image),egui::Button::new("完整结果另存…"));
                        #[cfg(feature="ui-preview")] {self.buttons[6]=Some((save.rect,ui.clip_rect()));}
                        if save.clicked(){
                            #[cfg(feature="ui-preview")] let fixture=self.file_path.take();
                            #[cfg(not(feature="ui-preview"))] let fixture:Option<std::path::PathBuf>=None;
                            if let Some(path)=fixture.or_else(||rfd::FileDialog::new().set_title("保存完整步骤结果（选择新文件名）").set_file_name(if output.kind()==Kind::Table{"flow-result.json"}else{"flow-result.txt"}).save_file()) {
                            match files::Output::prepare(&output,path){Ok(review)=>self.output_review=Some(review),Err(e)=>self.message=e.to_string()}
                        } }
                    }); });
                    egui::ScrollArea::vertical().id_salt("text-flow-result").max_height(95.0).show(ui, |ui| { ui.add(egui::TextEdit::multiline(&mut preview).interactive(false).desired_width(f32::INFINITY).desired_rows(4)); });
                }
            }
            egui::CollapsingHeader::new("流程配方 · 保存与载入").id_salt("text-flow-recipe").show(ui, |ui| {
                ui.label("当前输入和步骤可随工作实例保存。配方仅包含版本和步骤，不包含输入、结果、文件目标或授权。");
                if ui.button("复制当前配方").clicked() { match serde_json::to_string_pretty(&self.definition) { Ok(text)=>ui.ctx().copy_text(text),Err(e)=>self.message=e.to_string() } }
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy && !self.modal_open(),egui::Button::new("配方另存…")).clicked()
                        && let Some(path)=rfd::FileDialog::new().add_filter("工具流程配方",&["json"]).set_file_name("tool-flow.json").save_file(){
                        let definition=self.definition.clone();
                        if let Err(e)=self.file_task(move|cancel|files::save_recipe(&definition,&path,cancel).map(|path|Reply::Saved(path,serde_json::to_vec_pretty(&definition).unwrap().len()))){self.message=e.to_string();}
                    }
                    if ui.add_enabled(!busy && !self.modal_open(),egui::Button::new("读取配方…")).clicked()
                        && let Some(path)=rfd::FileDialog::new().add_filter("工具流程配方",&["json"]).pick_file()
                        && let Err(e)=self.file_task(move|cancel|files::recipe(&path,cancel).map(|d|Reply::Definition(path,d))){self.message=e.to_string();}
                });
                ui.add(egui::TextEdit::multiline(&mut self.recipe).desired_rows(3).desired_width(f32::INFINITY).char_limit(65536));
                if ui.add_enabled(!busy && !self.modal_open(),egui::Button::new("审核载入配方…")).clicked() {
                    match serde_json::from_str::<Definition>(&self.recipe).map_err(anyhow::Error::from).and_then(|d| {d.validate()?;Ok(d)}) {
                        Ok(d)=>self.review=Some(d),Err(e)=>self.message=e.to_string()
                    }
                }
            });
        });
        if !self.picker_open
            && !self.definition.steps.is_empty()
            && self
                .scroll_until
                .is_some_and(|until| std::time::Instant::now() < until)
        {
            response
                .header_response
                .scroll_to_me(Some(egui::Align::Min));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        if response.body_response.is_some() {
            self.reveal = false;
        }

        let mut input_choice = None;
        if let Some(input) = &self.input_review {
            egui::Modal::new(egui::Id::new("text-flow-file-input")).show(ui.ctx(),|ui|{
                ui.heading("载入文件内容？");ui.label(format!("完整UTF-8文本：{}字节。确认替换原输入，保留兼容步骤，清除旧结果；不会自动运行。若当前流程以文件引用开始，将移除首步 SHA-256，保留其余文本步骤。",input.len()));
                let mut preview=material::prefix(input,8192);
                egui::ScrollArea::vertical().max_height(180.0).show(ui,|ui|{ui.add(egui::TextEdit::multiline(&mut preview).interactive(false).desired_width(520.0));});
                ui.horizontal(|ui|{let back=ui.button("返回");let confirm=ui.button("确认替换输入");
                #[cfg(feature="ui-preview")] {self.buttons[10]=Some((confirm.rect,ui.clip_rect()));self.buttons[11]=Some((back.rect,ui.clip_rect()));}
                if back.clicked(){input_choice=Some(false);}else if confirm.clicked(){input_choice=Some(true);}});
            });
        }
        if let Some(confirm) = input_choice {
            let input = self.input_review.take().unwrap();
            if confirm {
                if let Err(error) = self.receive(input) {
                    self.message = error.to_string();
                }
            }
            self.open();
        }
        let mut output_choice = None;
        if let Some(review) = &self.output_review {
            egui::Modal::new(egui::Id::new("text-flow-file-output")).show(ui.ctx(), |ui| {
                ui.heading("保存完整步骤结果？");
                ui.label(&review.summary);
                ui.label(review.format);
                ui.label(format!(
                    "完整文件：{}字节；预览是有界样本。",
                    review.bytes.len()
                ));
                ui.label(format!("目标：{}", review.path.display()));
                ui.label("仅创建新文件。若目标已存在，保留已有文件和当前结果。确认不运行流程。");
                let mut preview = review.preview.clone();
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut preview)
                                .interactive(false)
                                .desired_width(520.0),
                        );
                    });
                ui.horizontal(|ui| {
                    let back = ui.button("返回");
                    let confirm = ui.button("确认保存新文件");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.buttons[7] = Some((confirm.rect, ui.clip_rect()));
                        self.buttons[8] = Some((back.rect, ui.clip_rect()));
                    }
                    if back.clicked() {
                        output_choice = Some(false);
                    } else if confirm.clicked() {
                        output_choice = Some(true);
                    }
                });
            });
        }
        if let Some(confirm) = output_choice {
            let output = self.output_review.take().unwrap();
            if confirm
                && let Err(e) = self.file_task(move |cancel| {
                    output
                        .save(cancel)
                        .map(|(path, bytes)| Reply::Saved(path, bytes))
                })
            {
                self.message = e.to_string();
            }
            self.open();
        }
        let mut choice = None;
        if let Some(definition) = &self.review {
            egui::Modal::new(egui::Id::new("text-flow-import")).show(ui.ctx(), |ui| {
                ui.heading("载入工具流程？");
                ui.label("只替换步骤，原输入保留；不会自动执行。当前步骤结果将清除。");
                for (i, step) in definition.steps.iter().enumerate() {
                    ui.label(format!(
                        "{}. {} · v{}",
                        i + 1,
                        step.registered().unwrap().label,
                        step.version
                    ));
                }
                ui.horizontal(|ui| {
                    let back = ui.button("返回");
                    let confirm = ui.button("确认载入");
                    #[cfg(feature = "ui-preview")]
                    {
                        self.buttons[12] = Some((confirm.rect, ui.clip_rect()));
                        self.buttons[13] = Some((back.rect, ui.clip_rect()));
                    }
                    if back.clicked() {
                        choice = Some(false);
                    } else if confirm.clicked() {
                        choice = Some(true);
                    }
                });
            });
        }
        if let Some(confirm) = choice {
            let definition = self.review.take().unwrap();
            if confirm {
                let file_input = definition.start_kind().ok() == Some(Kind::File);
                self.definition = definition;
                self.input_file = None;
                if file_input {
                    self.input.clear();
                }
                self.invalidate();
            }
            self.open();
        }
        if self.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_recipe_check(&self, phase: u8) {
        assert_eq!(self.input, "frozen tutorial text");
        assert!(self.result.is_none());
        assert!(!self.busy());
        if phase == 2 {
            assert_eq!(
                self.review.as_ref().unwrap().steps[0].action,
                "base64.encode"
            );
            assert!(self.definition.steps.is_empty());
        } else if phase == 4 {
            assert!(self.review.is_none());
            assert_eq!(self.definition.steps[0].action, "base64.encode");
        } else {
            assert!(self.review.is_none());
            assert!(self.definition.steps.is_empty());
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_path(&mut self, path: std::path::PathBuf) {
        self.file_path = Some(path);
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_output_ready(&self) -> bool {
        self.result.as_ref().is_some_and(|run| {
            run.materials
                .iter()
                .any(|output| output.port == "source-file")
        })
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_report_selected(&self) -> bool {
        self.result
            .as_ref()
            .and_then(|run| run.materials.get(self.selected))
            .is_some_and(|output| output.port == "result" && output.material.kind() == Kind::Text)
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_output_choice_position(&self, path: &std::path::Path) -> egui::Pos2 {
        let expected = std::fs::canonicalize(path).unwrap();
        let run = self.result.as_ref().expect("file flow result is ready");
        let index = run
            .materials
            .iter()
            .position(|output| {
                output.port == "source-file"
                    && matches!(
                        &output.material,
                        Material::File(file) if file.path() == expected
                    )
            })
            .expect("original file output exists");
        let (rect, clip) = self.output_buttons.get(index).copied().unwrap();
        assert!(
            clip.contains_rect(rect),
            "file output choice must be visible"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_output_selected(&self, path: &std::path::Path) -> bool {
        let expected = std::fs::canonicalize(path).unwrap();
        self.result.as_ref().is_some_and(|run| {
            run.materials.iter().enumerate().any(|(index, output)| {
                index == self.selected
                    && output.port == "source-file"
                    && matches!(
                        &output.material,
                        Material::File(file) if file.path() == expected
                    )
            })
        })
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_output_position(&self) -> egui::Pos2 {
        self.preview_position(2)
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_file_check(&self, path: &std::path::Path, phase: u8) {
        if phase < 4 {
            self.preview_table_check();
        }
        match phase {
            0 => {
                assert!(self.output_review.is_some());
                assert!(!path.exists());
            }
            1 => {
                assert!(self.output_review.is_none());
                assert!(!path.exists());
            }
            2 => {
                assert!(!self.busy());
                assert_eq!(self.job.phase, Phase::Done);
                let stored = std::fs::read_to_string(path).unwrap();
                assert_eq!(
                    stored,
                    self.result.as_ref().unwrap().outputs[3].render().unwrap()
                );
            }
            3 => {
                assert!(self.input_review.is_some());
                assert!(self.input.contains(" Zi Tools "));
            }
            4 => {
                assert!(self.result.is_none());
                assert!(self.input_review.is_none());
                assert_eq!(self.input, std::fs::read_to_string(path).unwrap());
                assert_eq!(self.definition.steps.len(), 4);
                assert!(!self.busy());
            }
            _ => panic!("unknown fixture phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_picker_prepare(&mut self) {
        self.input = "编号,名称\n001, 本地工具 ".into();
        self.picker_open = true;
        self.open();
    }
    #[cfg(feature = "ui-preview")]
    pub(crate) fn preview_picker_check(&self, phase: usize) {
        assert_eq!(self.input, "编号,名称\n001, 本地工具 ");
        let expected = &["table.parse_csv", "table.trim", "table.export_schema"][..phase.min(3)];
        assert_eq!(
            self.definition
                .steps
                .iter()
                .map(|step| step.action.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        if phase == 4 {
            assert_eq!(self.job.phase, Phase::Done);
            let run = self.result.as_ref().unwrap();
            assert_eq!(run.outputs.len(), 3);
            let data: crate::workbench::Dataset =
                serde_json::from_str(run.outputs[2].text().unwrap()).unwrap();
            assert_eq!(data.rows[0][1], "本地工具");
            assert!(run.failure.is_none());
        } else {
            assert!(!self.busy());
            assert!(self.result.is_none());
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_open(&mut self) {
        self.reveal = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_position(&self, index: usize) -> egui::Pos2 {
        let (rect, clip) = self.buttons[index].expect("visible flow button");
        assert!(
            clip.contains_rect(rect),
            "control {index} must be visible: {rect:?} clip={clip:?}"
        );
        rect.center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_check(&self) {
        assert_eq!(self.job.phase, Phase::Done);
        assert!(self.input.contains("本地工具🦀"));
        let run = self.result.as_ref().unwrap();
        assert_eq!(run.outputs.len(), 4);
        assert!(run.outputs[3].text().unwrap().contains("本地工具🦀"));
        assert!(run.outputs[3].text().unwrap().contains('\n'));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_table_check(&self) {
        assert_eq!(self.job.phase, Phase::Done);
        assert!(self.input.contains(" Zi Tools "));
        let run = self.result.as_ref().unwrap();
        assert_eq!(run.outputs.len(), 4);
        let Material::Table(data) = &run.outputs[3] else {
            panic!("typed table output")
        };
        assert_eq!(data.headers, vec!["编号", "名称", "数量"]);
        assert_eq!(
            data.rows[0],
            vec![
                serde_json::json!("001"),
                serde_json::json!("Zi Tools"),
                serde_json::json!("002")
            ]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_action_declares_its_effect_and_file_hash_is_the_only_file_read() {
        assert!(
            ACTIONS
                .iter()
                .all(|action| !action.effect.label().is_empty())
        );
        assert_eq!(
            ACTIONS
                .iter()
                .filter(|action| action.effect == Effect::ReadSelectedFile)
                .map(|action| action.id)
                .collect::<Vec<_>>(),
            ["file.sha256"]
        );
        assert_eq!(
            ACTIONS
                .iter()
                .find(|action| action.id == "image.info")
                .unwrap()
                .effect,
            Effect::ProcessInMemoryImage
        );
        assert!(
            ACTIONS
                .iter()
                .filter(|action| action.id != "file.sha256" && action.id != "image.info")
                .all(|action| action.effect == Effect::Transform)
        );
    }
    #[test]
    fn direct_recipe_file_read_emits_current_metadata_and_keeps_previous_materials() {
        let root = std::env::temp_dir().join(format!("zi-direct-recipe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("native.json");
        let definition = recipe(&["sha256.digest"]);
        std::fs::write(&path, serde_json::to_vec(&definition).unwrap()).unwrap();
        let mut state = State::default();
        state.input = "original".into();
        state.definition = recipe(&["base64.encode"]);
        state.result =
            Some(execute(&state.definition, &state.input, &AtomicBool::new(false)).unwrap());
        let task_path = path.clone();
        state
            .file_task(move |cancel| {
                files::recipe(&task_path, cancel).map(|d| Reply::Definition(task_path, d))
            })
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.busy() {
            assert!(std::time::Instant::now() < deadline);
            state.poll();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(state.loaded.as_ref().unwrap().path, path);
        assert!(state.loaded.as_ref().unwrap().valid());
        assert_eq!(state.review, Some(definition));
        assert_eq!(state.definition, recipe(&["base64.encode"]));
        assert_eq!(state.input, "original");
        assert!(state.result.is_some());
        let saved = serde_json::to_string(&state).unwrap();
        assert!(!saved.contains("native.json"));
        assert!(!saved.contains("sha256.digest"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn file_reply_cancellation_preserves_draft_and_published_save_wins_late_cancel() {
        let mut state = State::default();
        state.input = "source".into();
        state.definition = recipe(&["base64.encode"]);
        state.result =
            Some(execute(&state.definition, &state.input, &AtomicBool::new(false)).unwrap());
        let (tx, rx) = mpsc::channel();
        state.receiver = Some(rx);
        state.job.begin();
        tx.send(Ok(Reply::Input("replacement".into()))).unwrap();
        state.cancel();
        state.poll();
        assert_eq!(state.input, "source");
        assert!(state.input_review.is_none());
        assert!(state.result.is_some());
        assert_eq!(state.job.phase, Phase::Cancelled);
        let (tx, rx) = mpsc::channel();
        state.receiver = Some(rx);
        state.job.begin();
        tx.send(Ok(Reply::Saved(
            std::path::PathBuf::from("published.txt"),
            4,
        )))
        .unwrap();
        state.poll();
        assert_eq!(state.job.phase, Phase::Done);
        assert!(state.result.is_some());
    }
    fn recipe(ids: &[&str]) -> Definition {
        Definition {
            version: 1,
            steps: ids
                .iter()
                .map(|id| Step {
                    action: (*id).into(),
                    version: 1,
                })
                .collect(),
        }
    }
    fn typed(ids: &[&str]) -> Definition {
        let mut d = recipe(ids);
        d.version = 2;
        d
    }
    #[test]
    fn csv_native_encoding_and_typed_table_steps_preserve_order_and_leading_zero_text() {
        let source = "z,a,n\n001, 中文🦀 ,002\n003, x ,004";
        let d = typed(&[
            "table.parse_csv",
            "table.trim",
            "table.export_schema",
            "base64.encode",
            "base64.decode",
            "table.parse_schema",
        ]);
        let run = execute(&d, source, &AtomicBool::new(false)).unwrap();
        assert!(run.failure.is_none());
        assert_eq!(run.outputs.len(), 6);
        let Material::Table(data) = &run.outputs[5] else {
            panic!("table")
        };
        assert_eq!(data.headers, vec!["z", "a", "n"]);
        assert_eq!(
            data.rows[0],
            vec![
                serde_json::json!("001"),
                serde_json::json!("中文🦀"),
                serde_json::json!("002")
            ]
        );
        assert_eq!(run.outputs[2], run.outputs[4]);
        assert!(source.contains(" 中文🦀 "));
    }
    #[test]
    fn typed_json_values_and_empty_schema_survive_serial_operations() {
        let source = r#"[{"z":" x ","n":9223372036854775807,"yes":true,"missing":null,"nested":[1,"a"],"nul":"中文\u0000🦀"}]"#;
        let run = execute(
            &typed(&[
                "table.parse_json",
                "table.trim",
                "table.export_schema",
                "table.parse_schema",
            ]),
            source,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.failure.is_none());
        let Material::Table(data) = &run.outputs[3] else {
            panic!("table")
        };
        for (column, expected) in [
            ("z", serde_json::json!("x")),
            ("n", serde_json::json!(i64::MAX)),
            ("yes", serde_json::json!(true)),
            ("missing", serde_json::Value::Null),
            ("nested", serde_json::json!([1, "a"])),
            ("nul", serde_json::json!("中文\0🦀")),
        ] {
            let i = data.headers.iter().position(|c| c == column).unwrap();
            assert_eq!(data.rows[0][i], expected);
        }
        let run = execute(
            &typed(&[
                "table.parse_csv",
                "table.export_schema",
                "table.parse_schema",
            ]),
            "z,中文,a\n",
            &AtomicBool::new(false),
        )
        .unwrap();
        let Material::Table(data) = &run.outputs[2] else {
            panic!("table")
        };
        assert_eq!(data.headers, vec!["z", "中文", "a"]);
        assert!(data.rows.is_empty());
        let run = execute(
            &typed(&["table.parse_csv", "table.export_json"]),
            "z,中文,a\n",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(run.outputs.len(), 1);
        assert!(run.failure.unwrap().contains("列结构JSON"));
    }
    #[test]
    fn typed_contract_rejects_incompatible_chains_before_running_and_invalid_shapes_at_read() {
        let incompatible = typed(&["table.parse_csv", "base64.encode"]);
        assert!(
            incompatible
                .validate()
                .unwrap_err()
                .to_string()
                .contains("第2步")
        );
        assert!(recipe(&["table.parse_csv"]).validate().is_err());
        for source in [
            r#"{"headers":["a"],"rows":[[1,2]]}"#,
            r#"{"headers":["a","a"],"rows":[]}"#,
            r#"{"headers":["a"],"rows":[],"command":"run"}"#,
        ] {
            let run = execute(
                &typed(&["table.parse_schema", "table.export_schema"]),
                source,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(run.outputs.is_empty());
            assert!(run.failure.unwrap().contains("第1步"));
        }
        let run = execute(
            &typed(&["table.parse_json"]),
            r#"[{"":1}]"#,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.outputs.is_empty());
        assert!(run.failure.is_some());
        let run = execute(
            &typed(&["table.parse_tsv", "table.export_csv", "table.parse_csv"]),
            "b\ta\n002\t中文",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.failure.is_none());
        assert_eq!(run.outputs[0], run.outputs[2]);
    }
    #[test]
    fn serial_native_actions_keep_source_and_exact_intermediates() {
        let input = "{\"名称\":\"中文🦀\",\"n\":2}";
        let d = recipe(&[
            "json.minify",
            "base64.encode",
            "base64.decode",
            "json.pretty",
            "sha256.digest",
        ]);
        let run = execute(&d, input, &AtomicBool::new(false)).unwrap();
        assert!(run.failure.is_none());
        assert!(!run.cancelled);
        assert_eq!(run.outputs.len(), 5);
        assert_eq!(run.outputs[0], run.outputs[2]);
        assert_eq!(
            run.outputs[1],
            Material::Text(crate::tools::base64_encode(run.outputs[0].text().unwrap()))
        );
        assert_eq!(
            run.outputs[4],
            Material::Text(crate::tools::sha256(run.outputs[3].text().unwrap()))
        );
        assert_eq!(input, "{\"名称\":\"中文🦀\",\"n\":2}");
        let mut ids = std::collections::BTreeSet::new();
        for a in ACTIONS {
            assert!(ids.insert(a.id));
            if let material::Executor::Native(tool, operation) = a.executor {
                assert!(operation < tool.actions().len());
            }
        }
    }
    #[test]
    fn selected_file_hash_report_flows_into_text_actions_without_exposing_path() {
        use sha2::Digest;
        let root = std::env::temp_dir().join(format!("zi-file-flow-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("中文 source.txt");
        let content = "{\"name\":\"local 🦀\"}".as_bytes();
        std::fs::write(&path, content).unwrap();
        let file = crate::material_files::FileMaterial::selected(&path, usize::MAX).unwrap();
        let definition = Definition {
            version: 3,
            steps: ["file.sha256", "json.minify"]
                .into_iter()
                .map(|action| Step {
                    action: action.into(),
                    version: 1,
                })
                .collect(),
        };
        let run = execute_material(
            &definition,
            Material::File(file.clone()),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.failure.is_none());
        assert_eq!(run.outputs.len(), 2);
        assert_eq!(run.materials.len(), 3);
        assert_eq!(run.materials[0].step, 1);
        assert_eq!(run.materials[0].port, "result");
        assert_eq!(run.materials[0].label, "文件 · SHA-256 报告");
        assert_eq!(run.materials[1].step, 1);
        assert_eq!(run.materials[1].port, "source-file");
        assert_eq!(run.materials[1].label, "原始文件引用");
        assert_eq!(run.materials[1].material, Material::File(file.clone()));
        assert_eq!(run.materials[2].step, 2);
        assert_eq!(run.materials[2].port, "result");
        let report = run.outputs[0].text().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(report).unwrap();
        assert_eq!(parsed["material"], "file-sha256-report");
        assert_eq!(parsed["action"]["id"], "file.sha256");
        assert_eq!(parsed["action"]["version"], 1);
        let provenance = crate::tool_provenance::current_for_tool("text-flow").unwrap();
        assert_eq!(parsed["generatedBy"]["toolId"], "text-flow");
        assert_eq!(
            parsed["generatedBy"]["toolVersion"].as_str(),
            Some(provenance.tool_version.as_str())
        );
        assert_eq!(parsed["fileName"], "中文 source.txt");
        assert_eq!(parsed["bytes"], content.len());
        assert_eq!(
            parsed["sha256"],
            format!("{:x}", sha2::Sha256::digest(content))
        );
        assert!(!report.contains(&path.to_string_lossy().to_string()));
        assert!(serde_json::from_str::<serde_json::Value>(run.outputs[1].text().unwrap()).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), content);
        let mut state = State::default();
        state.result = Some(run);
        state.selected = 1;
        state.send_selected();
        assert_eq!(state.take_file_send(), Some(file.clone()));
        let snapshot = serde_json::to_string(&state).unwrap();
        assert!(!snapshot.contains(&path.to_string_lossy().to_string()));

        std::fs::write(&path, b"changed").unwrap();
        let changed =
            execute_material(&definition, Material::File(file), &AtomicBool::new(false)).unwrap();
        assert!(changed.failure.is_some());
        assert!(changed.outputs.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn receiving_file_prepends_versioned_hash_step_and_text_return_removes_only_it() {
        let root =
            std::env::temp_dir().join(format!("zi-file-flow-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("source.txt");
        std::fs::write(&path, "source").unwrap();
        let mut state = State::default();
        state.input = "old draft".into();
        state.definition = recipe(&["json.minify", "base64.encode"]);
        state
            .receive_file(crate::material_files::FileMaterial::selected(&path, usize::MAX).unwrap())
            .unwrap();
        assert_eq!(state.definition.version, 3);
        assert_eq!(
            state
                .definition
                .steps
                .iter()
                .map(|step| step.action.as_str())
                .collect::<Vec<_>>(),
            ["file.sha256", "json.minify", "base64.encode"]
        );
        assert!(state.input.is_empty());
        assert!(state.result.is_none());
        assert!(state.input_file.is_some());
        state.validate().unwrap();
        state.receive("replacement text".into()).unwrap();
        assert_eq!(state.input, "replacement text");
        assert!(state.input_file.is_none());
        assert_eq!(state.definition.version, 1);
        assert_eq!(state.definition.steps[0].action, "json.minify");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn image_material_produces_versioned_report_and_chains_into_text_actions() {
        let image = Arc::new(image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
            3,
            2,
            |x, y| image::Rgba([x as u8, y as u8, 7, if x == 1 && y == 1 { 0 } else { 255 }]),
        )));
        let material = crate::image_tools::ImageMaterial {
            image,
            origins: vec![crate::image_tools::Origin {
                id: "screenshot-workbench".into(),
                version: "1.0.0".into(),
                utc: "2026-10-15T00:00:00Z".into(),
            }],
            kind: "透明截图选区",
        };
        let definition = Definition {
            version: 4,
            steps: ["image.info", "json.minify"]
                .into_iter()
                .map(|action| Step {
                    action: action.into(),
                    version: 1,
                })
                .collect(),
        };
        let run = execute_material(
            &definition,
            Material::Image(material.clone()),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.failure.is_none());
        assert_eq!(run.outputs.len(), 2);
        let report: serde_json::Value =
            serde_json::from_str(run.outputs[0].text().unwrap()).unwrap();
        assert_eq!(report["schemaVersion"], 2);
        assert_eq!(report["material"], "image-information");
        assert_eq!(report["width"], 3);
        assert_eq!(report["height"], 2);
        assert_eq!(report["pixels"], 6);
        assert_eq!(report["hasAlphaChannel"], true);
        assert_eq!(report["hasTransparentPixels"], true);
        assert_eq!(report["encoding"], serde_json::Value::Null);
        assert_eq!(report["origins"][0]["toolId"], "screenshot-workbench");
        assert_eq!(report["generatedBy"]["toolId"], "text-flow");
        assert!(!run.outputs[1].text().unwrap().contains("\n"));
        assert!(!run.outputs[0].text().unwrap().contains("C:\\"));

        let mut state = State::default();
        state.receive_image(material).unwrap();
        assert_eq!(state.definition.start_kind().unwrap(), Kind::Image);
        assert_eq!(state.definition.version, 4);
        assert!(state.validate().is_ok());
        let snapshot = serde_json::to_string(&state).unwrap();
        assert!(!snapshot.contains("透明截图选区"));
        assert!(!snapshot.contains("screenshot-workbench"));
        let restored: State = serde_json::from_str(&snapshot).unwrap();
        assert!(restored.validate().is_ok());
        assert!(restored.validate_run().is_err());
    }
    #[test]
    fn failures_stop_at_actual_step_and_keep_completed_outputs() {
        let run = execute(
            &recipe(&["base64.encode", "json.pretty", "sha256.digest"]),
            "abc",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(run.outputs, vec![Material::Text("YWJj".into())]);
        assert!(run.failure.unwrap().contains("第2步"));
        let run = execute(&recipe(&["hex.encode"]), "abc", &AtomicBool::new(true)).unwrap();
        assert!(run.cancelled);
        assert!(run.outputs.is_empty());
        assert!(
            execute(
                &recipe(&["url.encode"]),
                &"x".repeat(LIMIT + 1),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        let run = execute(
            &recipe(&["base64.encode", "sha256.digest"]),
            &"x".repeat(LIMIT),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(run.outputs.is_empty());
        assert!(run.failure.unwrap().contains("第1步"));
    }
    #[test]
    fn serialized_recipe_rejects_unknown_versions_and_authority() {
        for text in [
            r#"{"version":1,"steps":[{"action":"json.pretty","version":2}]}"#,
            r#"{"version":1,"steps":[],"path":"C:/output"}"#,
            r#"{"version":1,"steps":[{"action":"shell.run","version":1}]}"#,
        ] {
            assert!(
                serde_json::from_str::<Definition>(text)
                    .map_err(anyhow::Error::from)
                    .and_then(|d| d.validate())
                    .is_err()
            );
        }
        let d = recipe(&["url.encode", "url.decode"]);
        let bytes = serde_json::to_vec(&d).unwrap();
        assert_eq!(serde_json::from_slice::<Definition>(&bytes).unwrap(), d);
        let run = execute(&d, "a b🦀", &AtomicBool::new(false)).unwrap();
        assert_eq!(run.outputs[1].text().unwrap(), "a b🦀");
        let mut d = d;
        d.steps = vec![d.steps[0].clone(); STEPS + 1];
        assert!(d.validate().is_err());
    }
    #[test]
    fn draft_restore_does_not_run_or_restore_stale_results() {
        let mut state = State::default();
        state.input = "中文".into();
        state.definition = recipe(&["base64.encode"]);
        state.start().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.busy() {
            state.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(state.job.phase, Phase::Done);
        let bytes = serde_json::to_vec(&state).unwrap();
        let restored: State = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.input, state.input);
        assert_eq!(restored.definition, state.definition);
        assert!(!restored.busy());
        assert!(restored.result.is_none());
        assert_eq!(restored.job.phase, Phase::Idle);
        state.send = Some("stale".into());
        state.invalidate();
        assert!(state.result.is_none());
        assert!(state.take_send().is_none());
    }
    #[test]
    fn cumulative_budget_keeps_prior_results_and_native_escape_pairs_roundtrip() {
        for pair in [
            ["html.escape", "html.unescape"],
            ["text.escape", "text.unescape"],
            ["hex.encode", "hex.decode"],
        ] {
            let input = "中文🦀\0<&\"\n";
            let run = execute(&recipe(&pair), input, &AtomicBool::new(false)).unwrap();
            assert!(run.failure.is_none());
            assert_eq!(run.outputs[1].text().unwrap(), input);
        }
        let input = serde_json::to_string(&"x".repeat(700_000)).unwrap();
        let definition = recipe(&["json.minify"; 16]);
        let run = execute(&definition, &input, &AtomicBool::new(false)).unwrap();
        assert_eq!(run.outputs.len(), 11);
        assert!(run.failure.unwrap().contains("第12步"));
        assert_eq!(run.outputs[10].text().unwrap(), input);
    }
}
