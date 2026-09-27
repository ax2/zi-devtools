use crate::{
    prompt_library::{Entry, Library},
    prompt_template::{self, Revision, RunOptions},
};
use eframe::egui;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    loaded: bool,
    entries: Vec<Entry>,
    skipped: usize,
    truncated: bool,
    search: String,
    editor_open: bool,
    diff_open: bool,
    selected: Option<String>,
    revisions: Vec<Revision>,
    revision_index: usize,
    name: String,
    tags: String,
    body: String,
    source_tool: String,
    model: String,
    stream: bool,
    multi_turn: bool,
    values: BTreeMap<String, String>,
    preview: Option<String>,
    delete_confirm: bool,
    message: String,
}

pub(super) struct Apply {
    pub text: String,
    pub options: Option<RunOptions>,
}

pub(super) struct Current<'a> {
    pub tool_id: &'a str,
    pub input: &'a str,
    pub model: &'a str,
    pub stream: bool,
    pub multi_turn: bool,
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_fixture(&mut self, tool_id: &str) {
        let options = RunOptions {
            tool_id: tool_id.into(),
            model: "local-example-model".into(),
            stream: true,
            multi_turn: true,
        };
        let previous = Revision::new(
            "代码审查助手",
            vec!["Rust".into(), "审查".into()],
            "请检查 {{file}} 的正确性。",
            options.clone(),
        )
        .unwrap();
        let current = Revision::new(
            "代码审查助手",
            vec!["Rust".into(), "审查".into()],
            "请检查 {{file}} 的正确性，并列出可复现的问题。",
            options,
        )
        .unwrap();
        self.entries = vec![Entry {
            id: "preview-only".into(),
            latest: current.clone(),
            revision_count: 2,
        }];
        self.selected = Some("preview-only".into());
        self.revisions = vec![previous, current];
        self.revision_index = 1;
        self.use_revision();
        self.values.insert("file".into(), "src/app.rs".into());
        self.preview = Some(prompt_template::render(&self.body, &self.values).unwrap());
        self.loaded = true;
        self.open = true;
        self.editor_open = false;
        self.message = "示例模板 · 截图未保存文件或发送请求".into();
    }

    #[cfg(feature = "ui-preview")]
    pub(super) fn preview_editor(&mut self) {
        self.editor_open = true;
        self.diff_open = true;
        self.preview = None;
    }

    fn refresh(&mut self, library: &Library) {
        match library.list() {
            Ok(listing) => {
                self.entries = listing.entries;
                self.skipped = listing.skipped;
                self.truncated = listing.truncated;
                self.loaded = true;
                self.selected = None;
                self.revisions.clear();
                self.preview = None;
                self.delete_confirm = false;
            }
            Err(error) => self.message = format!("模板库读取失败：{error}"),
        }
    }

    fn new_form(&mut self, tool_id: &str, input: &str, model: &str, stream: bool, multi: bool) {
        self.selected = None;
        self.revisions.clear();
        self.revision_index = 0;
        self.name.clear();
        self.tags.clear();
        self.body = input.into();
        self.source_tool = tool_id.into();
        self.model = model.into();
        self.stream = stream;
        self.multi_turn = multi;
        self.values.clear();
        self.preview = None;
        self.delete_confirm = false;
        self.editor_open = true;
        self.message.clear();
    }

    fn select(&mut self, library: &Library, id: &str) {
        match library.load(id) {
            Ok(revisions) => {
                self.selected = Some(id.into());
                self.revision_index = revisions.len() - 1;
                self.revisions = revisions;
                self.use_revision();
                self.editor_open = false;
                self.message.clear();
            }
            Err(error) => self.message = format!("模板读取失败：{error}"),
        }
    }

    fn use_revision(&mut self) {
        let Some(revision) = self.revisions.get(self.revision_index).cloned() else {
            return;
        };
        self.name = revision.name;
        self.tags = revision.tags.join(", ");
        self.body = revision.body;
        self.source_tool = revision.options.tool_id;
        self.model = revision.options.model;
        self.stream = revision.options.stream;
        self.multi_turn = revision.options.multi_turn;
        self.values.clear();
        self.preview = None;
        self.delete_confirm = false;
    }

    fn options(&self) -> RunOptions {
        RunOptions {
            tool_id: self.source_tool.clone(),
            model: self.model.clone(),
            stream: self.stream,
            multi_turn: self.multi_turn,
        }
    }

    fn save(&mut self, library: &Library, tool_id: &str) {
        let revision = prompt_template::parse_tags(&self.tags).and_then(|tags| {
            Revision::new(
                &self.name,
                tags,
                &self.body,
                RunOptions {
                    tool_id: tool_id.into(),
                    model: self.model.clone(),
                    stream: self.stream,
                    multi_turn: self.multi_turn,
                },
            )
        });
        let result = revision.and_then(|revision| match self.selected.as_deref() {
            Some(id) => library.save_revision(id, revision),
            None => library.save_new(revision),
        });
        match result {
            Ok(entry) => {
                let id = entry.id;
                self.refresh(library);
                self.select(library, &id);
                self.message = format!("模板已保存 · 第 {} 版；不会自动运行", entry.revision_count);
            }
            Err(error) => self.message = format!("模板保存失败：{error}"),
        }
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    state: &mut State,
    library: &Library,
    current: Current<'_>,
) -> Option<Apply> {
    let tool_id = current.tool_id;
    let mut apply = None;
    egui::CollapsingHeader::new("提示词模板库")
        .default_open(state.open)
        .show(ui, |ui| {
            ui.weak("本机模板可跨工具填入正文；变量值仅在本次页面内保留。预览后才会替换输入，不会自动发送请求。");
            if !state.loaded {
                state.refresh(library);
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("按名称或标签搜索").desired_width(260.0));
                if ui.button("刷新").clicked() {
                    state.refresh(library);
                }
                if ui.button("新建").clicked() {
                    state.new_form(tool_id, current.input, current.model, current.stream, current.multi_turn);
                }
            });
            if state.skipped > 0 {
                ui.weak(format!("{} 个损坏或超额文件未显示", state.skipped));
            }
            if state.truncated {
                ui.weak("模板目录超过扫描上限；请整理目录后再保存");
            }
            let visible = state
                .entries
                .iter()
                .filter(|entry| prompt_template::matches_query(&entry.latest, &state.search))
                .cloned()
                .collect::<Vec<_>>();
            if visible.is_empty() {
                ui.weak(if state.entries.is_empty() { "尚无已保存模板" } else { "没有匹配模板" });
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("prompt-list", tool_id))
                    .max_height(125.0)
                    .show(ui, |ui| {
                        for entry in &visible {
                            let selected = state.selected.as_deref() == Some(&entry.id);
                            let label = format!(
                                "{} · {} · {} 版",
                                entry.latest.name,
                                entry.latest.tags.join(" / "),
                                entry.revision_count
                            );
                            if ui.selectable_label(selected, label).clicked() {
                                state.select(library, &entry.id);
                            }
                        }
                    });
            }

            if ui.button(if state.editor_open { "收起编辑与修订" } else { "展开编辑与修订" }).clicked() {
                state.editor_open = !state.editor_open;
            }
            if state.editor_open {
            ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label("名称");
                if ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(220.0)).changed() {
                    state.preview = None;
                }
                ui.label("标签（逗号分隔）");
                if ui.add(egui::TextEdit::singleline(&mut state.tags).desired_width(260.0)).changed() {
                    state.preview = None;
                }
            });
            ui.label("模板正文 · 变量写成 {{name}}，字面量双花括号写成 {{{{ 与 }}}}");
            if ui.add(egui::TextEdit::multiline(&mut state.body).desired_rows(4).desired_width(f32::INFINITY)).changed() {
                state.preview = None;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("运行参数快照");
                if ui.add(egui::TextEdit::singleline(&mut state.model).hint_text("模型名称").desired_width(210.0)).changed() {
                    state.preview = None;
                }
                if ui.checkbox(&mut state.stream, "流式").changed() { state.preview = None; }
                if ui.checkbox(&mut state.multi_turn, "多轮").changed() { state.preview = None; }
                ui.weak("不保存接口或凭据");
            });

            if !state.revisions.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("浏览修订");
                    let mut chosen = state.revision_index;
                    egui::ComboBox::from_id_salt(("prompt-revision", tool_id))
                        .selected_text(format!("第 {} / {} 版", chosen + 1, state.revisions.len()))
                        .show_ui(ui, |ui| {
                            for index in 0..state.revisions.len() {
                                ui.selectable_value(&mut chosen, index, format!("第 {} 版", index + 1));
                            }
                        });
                    if chosen != state.revision_index {
                        state.revision_index = chosen;
                        state.use_revision();
                    }
                    ui.weak("选择旧版只改变编辑预览；保存会追加新版");
                });
                if state.revision_index > 0 {
                    egui::CollapsingHeader::new("与前一版的差异")
                        .default_open(state.diff_open)
                        .show(ui, |ui| {
                        let text = prompt_template::diff(
                            &state.revisions[state.revision_index - 1],
                            &state.revisions[state.revision_index],
                        );
                        egui::ScrollArea::vertical()
                            .id_salt(("prompt-diff", tool_id))
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.monospace(text);
                            });
                    });
                }
            }

            ui.horizontal(|ui| {
                let label = if state.selected.is_some() { "保存为新修订" } else { "保存新模板" };
                if ui.button(label).clicked() {
                    state.save(library, tool_id);
                }
                if state.selected.is_some() && ui.button("删除模板").clicked() {
                    state.delete_confirm = true;
                }
            });
            if state.delete_confirm {
                ui.horizontal(|ui| {
                    ui.weak("将删除这份模板及全部修订；不会修改当前输入。");
                    if ui.button("确认删除模板").clicked() {
                        if let Some(id) = state.selected.clone() {
                            match library.delete(&id) {
                                Ok(()) => {
                                    state.refresh(library);
                                    state.message = "模板文件已删除".into();
                                }
                                Err(error) => state.message = format!("删除失败：{error}"),
                            }
                        }
                        state.delete_confirm = false;
                    }
                    if ui.button("取消").clicked() { state.delete_confirm = false; }
                });
            }
            });
            }

            match prompt_template::variables(&state.body) {
                Ok(names) => {
                    if names.is_empty() { ui.weak("此模板没有变量"); }
                    for name in names {
                        ui.horizontal(|ui| {
                            ui.label(format!("{name} ="));
                            if ui.add(egui::TextEdit::singleline(state.values.entry(name).or_default()).desired_width(400.0)).changed() {
                                state.preview = None;
                            }
                        });
                    }
                }
                Err(error) if !state.body.is_empty() => { ui.weak(error.to_string()); }
                Err(_) => {}
            }
            if ui.button("预览变量展开").clicked() {
                match prompt_template::render(&state.body, &state.values) {
                    Ok(text) => { state.preview = Some(text); state.message.clear(); }
                    Err(error) => { state.preview = None; state.message = format!("预览失败：{error}"); }
                }
            }
            if let Some(text) = &state.preview {
                ui.label(format!("展开预览 · {} 字节；应用将替换当前输入，不会发送请求", text.len()));
                egui::ScrollArea::vertical().id_salt(("prompt-preview", tool_id)).max_height(110.0).show(ui, |ui| {
                    ui.label(text);
                });
                ui.horizontal(|ui| {
                    if ui.button("仅填入正文").clicked() {
                        apply = Some(Apply { text: text.clone(), options: None });
                    }
                    let can_apply_options = state.source_tool == tool_id && !state.model.trim().is_empty();
                    if ui.add_enabled(can_apply_options, egui::Button::new("填入正文与运行参数")).clicked() {
                        apply = Some(Apply { text: text.clone(), options: Some(state.options()) });
                    }
                });
                if state.source_tool != tool_id {
                    ui.weak("模板来自其他工具，运行参数不兼容；这里只能填入正文。");
                } else if state.model.trim().is_empty() {
                    ui.weak("没有模型名称，不能应用运行参数；仍可只填入正文。");
                } else {
                    ui.weak("应用参数可能改变模型或多轮模式，并清空不兼容的当前上下文。不会保存连接设置。");
                }
            }
            if !state.message.is_empty() { ui.label(&state.message); }
        });
    apply
}
