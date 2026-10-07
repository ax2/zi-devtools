//! Versioned native text actions and serial execution. No implicit file/network actions.
use crate::{
    tasks::{Job, Phase},
    tools::{ToolKind, run_tool},
};
use anyhow::{Result, ensure};
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

pub struct Action {
    pub id: &'static str,
    pub version: u32,
    pub label: &'static str,
    pub tool: ToolKind,
    operation: usize,
}
macro_rules! actions {
    ($(($id:literal, $label:literal, $kind:ident, $op:literal)),* $(,)?) => {
        pub static ACTIONS: &[Action] = &[$(Action { id: $id, version: 1, label: $label, tool: ToolKind::$kind, operation: $op }),*];
    }
}
actions![
    ("json.pretty", "JSON · 格式化", Json, 0),
    ("json.minify", "JSON · 压缩", Json, 1),
    ("base64.encode", "Base64 · 编码", Base64, 0),
    ("base64.decode", "Base64 · 解码", Base64, 1),
    ("url.encode", "URL · 编码", Url, 0),
    ("url.decode", "URL · 解码", Url, 1),
    ("html.escape", "HTML · 转义", HtmlEscape, 0),
    ("html.unescape", "HTML · 还原", HtmlEscape, 1),
    ("text.escape", "文本 · 转义", TextEscape, 0),
    ("text.unescape", "文本 · 还原", TextEscape, 1),
    ("hex.encode", "十六进制 · 编码", Hex, 0),
    ("hex.decode", "十六进制 · 解码", Hex, 1),
    ("sha256.digest", "SHA-256 · 摘要", Sha256, 0),
];

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
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "不支持此文本流程版本");
        ensure!(self.steps.len() <= STEPS, "文本流程最多16步");
        for step in &self.steps {
            step.registered()?;
        }
        Ok(())
    }
}
#[derive(Debug, Default)]
pub struct Run {
    pub outputs: Vec<String>,
    pub failure: Option<String>,
    pub cancelled: bool,
}
pub fn execute(definition: &Definition, input: &str, cancel: &AtomicBool) -> Result<Run> {
    definition.validate()?;
    ensure!(!definition.steps.is_empty(), "请先添加操作步骤");
    ensure!(input.len() <= LIMIT, "输入最多1 MiB");
    let mut run = Run::default();
    let mut current = input.to_owned();
    let mut bytes = 0usize;
    for (index, step) in definition.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            run.cancelled = true;
            break;
        }
        let action = step.registered()?;
        match run_tool(action.tool, action.operation, &current, "", 10) {
            Ok(output) => {
                if output.len() > LIMIT || bytes.saturating_add(output.len()) > TRACE_LIMIT {
                    run.failure = Some(format!(
                        "第{}步 {}：结果超过单步1 MiB或累计8 MiB，后续步骤未运行",
                        index + 1,
                        action.label
                    ));
                    break;
                }
                bytes += output.len();
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
    query: String,
    #[serde(skip)]
    picker_open: bool,
    #[serde(skip)]
    result: Option<Run>,
    #[serde(skip)]
    receiver: Option<Receiver<std::result::Result<Run, String>>>,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
    #[serde(skip)]
    pub(crate) job: Job,
    #[serde(skip)]
    message: String,
    #[serde(skip)]
    send: Option<String>,
    #[serde(skip)]
    recipe: String,
    #[serde(skip)]
    review: Option<Definition>,
    #[serde(skip)]
    reveal: bool,
    #[serde(skip)]
    scroll_until: Option<std::time::Instant>,
    #[serde(skip)]
    selected: usize,
    #[cfg(feature = "ui-preview")]
    #[serde(skip)]
    buttons: [Option<egui::Rect>; 3],
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub(crate) fn open(&mut self) {
        self.reveal = true;
        self.scroll_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.review.is_some()
    }
    pub(crate) fn receive(&mut self, input: String) -> Result<()> {
        ensure!(input.len() <= LIMIT, "文本流程输入最多1 MiB");
        ensure!(!self.busy(), "文本流程正在运行");
        self.input = input;
        self.invalidate();
        self.open();
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn has_content(&self) -> bool {
        !self.input.is_empty() || !self.definition.steps.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.input.len() <= LIMIT, "文本流程输入最多1 MiB");
        self.definition.validate()
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
        self.message.clear();
        self.selected = 0;
    }
    fn start(&mut self) -> Result<()> {
        ensure!(!self.busy(), "请等待当前流程结束");
        self.validate()?;
        ensure!(!self.definition.steps.is_empty(), "请先添加操作步骤");
        self.invalidate();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let definition = self.definition.clone();
        let input = self.input.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.job.begin();
        std::thread::spawn(move || {
            let _ = tx.send(execute(&definition, &input, &cancel).map_err(|e| format!("{e:#}")));
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
            Ok(mut run) => {
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
                self.selected = run.outputs.len().saturating_sub(1);
                self.result = Some(run);
                self.open();
            }
            Err(error) => {
                self.job.finish(Phase::Failed, &error);
                self.message = error;
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll();
        let response = egui::CollapsingHeader::new("操作流程 · 文本工具链")
            .id_salt("text-tool-flow").open(self.reveal.then_some(true)).default_open(self.has_content()).show(ui, |ui| {
            ui.label("每一步接收上一步文本。运行只计算结果；原输入保留，可保存当前工作实例。最多16步。");
            let busy = self.busy();
            let mut changed = false;
            ui.add_enabled_ui(!busy, |ui| {
                ui.label("原输入");
                egui::ScrollArea::vertical().id_salt("text-flow-input").max_height(75.0).show(ui, |ui| { changed |= ui.add(egui::TextEdit::multiline(&mut self.input).desired_rows(3).desired_width(f32::INFINITY).char_limit(LIMIT)).changed(); });
                ui.horizontal(|ui| {
                    if ui.button("添加操作…").clicked() { self.picker_open = !self.picker_open; }
                    let button=ui.add_enabled(self.input.is_empty() && self.definition.steps.is_empty(), egui::Button::new("示例"))
                        .on_hover_text("已有草稿时保留内容；可在新实例中载入示例");
                    #[cfg(feature="ui-preview")] { self.buttons[0]=Some(button.rect); }
                    if button.clicked() { self.input = "{\"名称\":\"本地工具🦀\",\"数量\":2}".into(); self.definition.steps = ["json.minify","base64.encode","base64.decode","json.pretty"].into_iter().map(|id| Step{action:id.into(),version:1}).collect(); self.open(); changed=true; }
                });
                if self.picker_open || self.definition.steps.is_empty() {
                ui.horizontal(|ui| { ui.label("搜索操作"); ui.text_edit_singleline(&mut self.query); });
                let query=self.query.trim().to_lowercase();
                egui::ScrollArea::vertical().id_salt("text-flow-actions").max_height(95.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for action in ACTIONS.iter().filter(|a| query.is_empty() || a.id.contains(&query) || a.label.to_lowercase().contains(&query)) {
                            if ui.add_enabled(self.definition.steps.len()<STEPS, egui::Button::new(action.label)).on_hover_text(format!("{} · 操作契约v{} · 文本 → 文本",action.id,action.version)).clicked() {
                                self.definition.steps.push(Step{action:action.id.into(),version:action.version}); changed=true;
                            }
                        }
                    });
                });
                }
                let mut edit = None;
                egui::ScrollArea::vertical().id_salt("text-flow-step-list").max_height(125.0).show(ui, |ui| {
                for (index, step) in self.definition.steps.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {} · v{}",index+1,step.registered().map_or(step.action.as_str(),|a| a.label),step.version));
                        if ui.add_enabled(index>0,egui::Button::new("↑")).clicked() { edit=Some((index,0)); }
                        if ui.add_enabled(index+1<self.definition.steps.len(),egui::Button::new("↓")).clicked() { edit=Some((index,1)); }
                        if ui.button("移除").clicked() { edit=Some((index,2)); }
                    });
                }
                });
                if let Some((index,op))=edit { match op { 0=>self.definition.steps.swap(index,index-1),1=>self.definition.steps.swap(index,index+1),_=>{self.definition.steps.remove(index);} }; changed=true; }
            });
            if changed { self.invalidate(); }
            ui.horizontal(|ui| {
                let button=ui.add_enabled(!busy&&!self.definition.steps.is_empty(),egui::Button::new("运行文本流程"));
                #[cfg(feature="ui-preview")] { self.buttons[1]=Some(button.rect); }
                if button.clicked() && let Err(error)=self.start() { self.message=error.to_string(); }
                if ui.add_enabled(busy,egui::Button::new("取消")).on_hover_text("在当前操作返回后停止；已完成结果保留").clicked() { self.cancel(); }
            });
            if busy { ui.horizontal(|ui| { ui.spinner(); ui.label("正在运行文本流程，原输入保留…"); }); }
            if !self.message.is_empty() {
                if self.job.phase == Phase::Failed { ui.colored_label(ui.visuals().error_fg_color, &self.message); }
                else { ui.label(&self.message); }
            }
            if let Some(run)=&self.result {
                ui.horizontal_wrapped(|ui| { for (index,_) in run.outputs.iter().enumerate() { ui.selectable_value(&mut self.selected,index,format!("第{}步结果",index+1)); } });
                if let Some(output)=run.outputs.get(self.selected) {
                    ui.label(format!("{}字节{}",output.len(),if output.len()>8192 { " · 仅展示前8192字节，复制与接力使用完整结果" } else { "" }));
                    let mut preview=output.chars().scan(0,|bytes,c|{*bytes+=c.len_utf8(); (*bytes<=8192).then_some(c)}).collect::<String>();
                    ui.horizontal(|ui| {
                        if ui.button("复制完整结果").clicked() { ui.ctx().copy_text(output.clone()); }
                        let button=ui.button("发送到其他工具…");
                        #[cfg(feature="ui-preview")] { self.buttons[2]=Some(button.rect); }
                        if button.clicked() { self.send=Some(output.clone()); }
                    });
                    egui::ScrollArea::vertical().id_salt("text-flow-result").max_height(95.0).show(ui, |ui| { ui.add(egui::TextEdit::multiline(&mut preview).interactive(false).desired_width(f32::INFINITY).desired_rows(4)); });
                }
            }
            egui::CollapsingHeader::new("流程配方 · 保存与载入").id_salt("text-flow-recipe").show(ui, |ui| {
                ui.label("当前输入和步骤可随工作实例保存。配方仅包含版本和步骤，不包含输入、结果、文件目标或授权。");
                if ui.button("复制当前配方").clicked() { match serde_json::to_string_pretty(&self.definition) { Ok(text)=>ui.ctx().copy_text(text),Err(e)=>self.message=e.to_string() } }
                ui.add(egui::TextEdit::multiline(&mut self.recipe).desired_rows(3).desired_width(f32::INFINITY).char_limit(65536));
                if ui.add_enabled(!busy,egui::Button::new("审核载入配方…")).clicked() {
                    match serde_json::from_str::<Definition>(&self.recipe).map_err(anyhow::Error::from).and_then(|d| {d.validate()?;Ok(d)}) {
                        Ok(d)=>self.review=Some(d),Err(e)=>self.message=e.to_string()
                    }
                }
            });
        });
        if self
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
        let mut choice = None;
        if let Some(definition) = &self.review {
            egui::Modal::new(egui::Id::new("text-flow-import")).show(ui.ctx(), |ui| {
                ui.heading("载入文本流程？");
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
                    if ui.button("返回").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("确认载入").clicked() {
                        choice = Some(true);
                    }
                });
            });
        }
        if let Some(confirm) = choice {
            let definition = self.review.take().unwrap();
            if confirm {
                self.definition = definition;
                self.invalidate();
            }
        }
        if self.busy() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_open(&mut self) {
        self.reveal = true;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_position(&self, index: usize) -> egui::Pos2 {
        self.buttons[index]
            .expect("visible text flow button")
            .center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_check(&self) {
        assert_eq!(self.job.phase, Phase::Done);
        assert!(self.input.contains("本地工具🦀"));
        let run = self.result.as_ref().unwrap();
        assert_eq!(run.outputs.len(), 4);
        assert!(run.outputs[3].contains("本地工具🦀"));
        assert!(run.outputs[3].contains('\n'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(run.outputs[1], crate::tools::base64_encode(&run.outputs[0]));
        assert_eq!(run.outputs[4], crate::tools::sha256(&run.outputs[3]));
        assert_eq!(input, "{\"名称\":\"中文🦀\",\"n\":2}");
        let mut ids = std::collections::BTreeSet::new();
        for a in ACTIONS {
            assert!(ids.insert(a.id));
            assert!(a.operation < a.tool.actions().len());
        }
    }
    #[test]
    fn failures_stop_at_actual_step_and_keep_completed_outputs() {
        let run = execute(
            &recipe(&["base64.encode", "json.pretty", "sha256.digest"]),
            "abc",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(run.outputs, vec!["YWJj"]);
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
        assert_eq!(run.outputs[1], "a b🦀");
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
            assert_eq!(run.outputs[1], input);
        }
        let input = serde_json::to_string(&"x".repeat(700_000)).unwrap();
        let definition = recipe(&["json.minify"; 16]);
        let run = execute(&definition, &input, &AtomicBool::new(false)).unwrap();
        assert_eq!(run.outputs.len(), 11);
        assert!(run.failure.unwrap().contains("第12步"));
        assert_eq!(run.outputs[10], input);
    }
}
