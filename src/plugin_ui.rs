use crate::plugins::{self, Adapter, Store};
use eframe::egui;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

#[derive(Default)]
struct Draft {
    input: String,
    output: String,
    model: String,
    endpoint: String,
    settings_key: String,
    settings_message: String,
    profile_id: Option<String>,
    profile_name: String,
    profile_editor: bool,
    profile_delete: bool,
    discovery_open: bool,
    models: Vec<String>,
    models_source: String,
    discovery_message: String,
    chat_mode: bool,
    conversation: crate::conversation::Conversation,
    pending_chat: Option<(String, String)>,
    chat_message: String,
}
struct DiscoveryJob {
    id: String,
    source: String,
    settings_key: String,
    receiver: Receiver<Result<crate::model_discovery::Report, String>>,
}
pub struct PluginState {
    pub store: Store,
    settings: crate::plugin_settings::Settings,
    pub selected: Option<String>,
    path: String,
    preview: Option<Vec<u8>>,
    message: String,
    drafts: HashMap<String, Draft>,
    token: String,
    use_saved: bool,
    discovery_token: String,
    discovery_running: Option<DiscoveryJob>,
    credential_message: String,
    pending_remove: Option<String>,
    copied_at: Option<std::time::Instant>,
    running: Option<(String, Receiver<Result<String, String>>)>,
}
impl PluginState {
    #[cfg(feature = "ui-preview")]
    pub fn preview_conversation(&mut self) {
        self.preview_profiles();
        let id = "plugin:openai-local/chat";
        let tool = self
            .store
            .tool_refs()
            .find(|(key, _)| key == id)
            .unwrap()
            .1
            .clone();
        let draft = self.drafts.get_mut(id).unwrap();
        draft.profile_editor = false;
        draft.chat_mode = true;
        draft.conversation.bind(format!(
            "{}:{}",
            crate::plugin_settings::Settings::key(id, &tool),
            draft.model
        ));
        draft
            .conversation
            .complete(
                "JSON 与 YAML 有什么区别？".into(),
                "JSON 结构严格，适合程序交换数据；YAML 更方便手写配置，但需要注意缩进。".into(),
            )
            .unwrap();
        draft.input = "为刚才的区别补充一个配置示例。".into();
        draft.chat_message = "示例会话 · 截图未发送请求".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_model_discovery(&mut self) {
        self.preview_profiles();
        let draft = self.drafts.get_mut("plugin:openai-local/chat").unwrap();
        draft.profile_editor = false;
        draft.discovery_open = true;
        draft.models_source = draft.endpoint.clone();
        draft.models = vec!["local-example-model".into(), "local-code-model".into()];
        draft.discovery_message = "示例结果 · 2 个模型（截图未发送请求）".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_profiles(&mut self) {
        let id = "plugin:openai-local/chat";
        let tool = self
            .store
            .tool_refs()
            .find(|(key, _)| key == id)
            .unwrap()
            .1
            .clone();
        let connection = crate::plugin_settings::Connection {
            endpoint: "http://127.0.0.1:1234/v1/chat/completions".into(),
            model: "local-example-model".into(),
        };
        let existing = self
            .settings
            .profiles()
            .find(|(_, p)| p.name == "本机推理 · 示例")
            .map(|(id, _)| id.clone());
        let profile_id = match existing {
            Some(id) => id,
            None => self
                .settings
                .save_profile(
                    None,
                    crate::plugin_settings::Profile {
                        name: "本机推理 · 示例".into(),
                        shape: crate::plugin_settings::request_shape(&tool).unwrap(),
                        connection: connection.clone(),
                    },
                )
                .unwrap(),
        };
        self.drafts.insert(
            id.into(),
            Draft {
                endpoint: connection.endpoint,
                model: connection.model,
                settings_key: crate::plugin_settings::Settings::key(id, &tool),
                profile_id: Some(profile_id),
                profile_name: "本机推理 · 示例".into(),
                profile_editor: true,
                ..Default::default()
            },
        );
    }
    pub fn new(root: PathBuf) -> Self {
        Self {
            settings: crate::plugin_settings::Settings::load(
                root.join("settings/connections.json"),
            ),
            store: Store::load(root),
            selected: None,
            path: String::new(),
            preview: None,
            message: String::new(),
            drafts: HashMap::new(),
            token: String::new(),
            use_saved: false,
            discovery_token: String::new(),
            discovery_running: None,
            credential_message: String::new(),
            pending_remove: None,
            copied_at: None,
            running: None,
        }
    }
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.clear_token();
        }
        self.selected = Some(id.into());
    }
    pub fn clear_token(&mut self) {
        self.use_saved = false;
        self.credential_message.clear();
        self.token.clear();
        self.discovery_token.clear();
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some() || self.discovery_running.is_some()
    }
    pub fn poll(&mut self) -> Option<String> {
        if let Some(job) = &self.discovery_running {
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("模型查询线程已退出".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let draft = self.drafts.entry(job.id.clone()).or_default();
                if draft.endpoint == job.source && draft.settings_key == job.settings_key {
                    draft.models.clear();
                    draft.models_source = job.source.clone();
                    draft.discovery_message = match result {
                        Ok(report) => {
                            draft.models = report.models;
                            format!(
                                "模型列表查询成功 · {} ms · {} 个模型；不代表生成请求一定可用",
                                report.elapsed_ms,
                                draft.models.len()
                            )
                        }
                        Err(error) => format!("查询失败：{error}"),
                    };
                }
                self.discovery_running = None;
                return Some("模型查询已结束，请查看原工具的连接检查结果".into());
            }
        }
        let (id, rx) = self.running.as_ref()?;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err("执行线程已退出".into()),
            Err(mpsc::TryRecvError::Empty) => return None,
        };
        let succeeded = result.is_ok();
        let draft = self.drafts.entry(id.clone()).or_default();
        if let Some((binding, input)) = draft.pending_chat.take() {
            if draft.conversation.binding() == binding {
                if let Ok(output) = &result {
                    match draft.conversation.complete(input, output.clone()) {
                        Ok(()) => {
                            draft.input.clear();
                            draft.chat_message.clear();
                        }
                        Err(error) => draft.chat_message = error.to_string(),
                    }
                } else {
                    draft.chat_message = "请求失败，消息与此前上下文保留；可再次运行重试".into();
                }
            }
        }
        draft.output = result.unwrap_or_else(|e| format!("失败：{e}"));
        self.running = None;
        Some(
            if succeeded {
                "插件执行完成"
            } else {
                "插件执行失败，请查看结果"
            }
            .into(),
        )
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, allow_shortcuts: bool) {
        ui.heading(egui::RichText::new("插件与连接器").size(28.0));
        ui.label(
            "安装工具定义，按需连接本机或远程服务。已启用工具会自动进入分类、收藏与 Ctrl K 搜索。",
        );
        ui.add_space(12.0);
        if self.selected.is_some() && ui.button("← 返回插件管理").clicked() {
            self.selected = None;
            self.clear_token();
        }
        if let Some(id) = self.selected.clone() {
            if let Some((_, mut tool)) = self
                .store
                .tool_refs()
                .find(|(key, _)| key == &id)
                .map(|(key, t)| (key, t.clone()))
            {
                let busy = self.running.as_ref().is_some_and(|(key, _)| key == &id)
                    || self
                        .discovery_running
                        .as_ref()
                        .is_some_and(|job| job.id == id);
                let get_only = matches!(&tool.adapter,Adapter::Http{method,..} if method=="GET");
                ui.heading(if self.drafts.get(&id).is_some_and(|d| d.chat_mode) {
                    "多轮模型对话"
                } else {
                    &tool.name
                });
                ui.label(&tool.description);
                let settings_key = crate::plugin_settings::Settings::key(&id, &tool);
                let profile_tool = tool.clone();
                let default_endpoint = match &tool.adapter {
                    Adapter::Http { url, .. } => url.clone(),
                    _ => String::new(),
                };
                let draft = self.drafts.entry(id.clone()).or_default();
                if draft.settings_key != settings_key {
                    let saved = self.settings.get(&settings_key);
                    draft.model = saved.map_or_else(|| tool.model.clone(), |v| v.model.clone());
                    draft.endpoint =
                        saved.map_or_else(|| default_endpoint.clone(), |v| v.endpoint.clone());
                    draft.settings_key = settings_key.clone();
                    draft.settings_message.clear();
                    draft.profile_id = None;
                    draft.profile_name.clear();
                    draft.profile_delete = false;
                    self.token.clear();
                    self.discovery_token.clear();
                    self.use_saved = false;
                    self.credential_message.clear();
                }
                let mut connection_valid = true;
                if let Adapter::Http {
                    url, method, body, ..
                } = &mut tool.adapter
                {
                    ui.add_enabled_ui(!busy, |ui| {
                        egui::CollapsingHeader::new("连接设置 · 地址与模型")
                            .id_salt(("connection-settings", &id))
                            .default_open(draft.profile_editor)
                            .show(ui, |ui| {
                                ui.label("完整接口地址（远程 HTTPS / 本机 HTTP）");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut draft.endpoint)
                                            .desired_width(f32::INFINITY),
                                    )
                                    .changed()
                                {
                                    self.token.clear();
                                    self.discovery_token.clear();
                                    self.use_saved = false;
                                    self.credential_message.clear();
                                    draft.settings_message.clear();
                                }
                                if method == "POST" && plugins::uses_model(body) {
                                    ui.horizontal(|ui| {
                                        ui.label("模型名称");
                                        if ui
                                            .add(
                                                egui::TextEdit::singleline(&mut draft.model)
                                                    .hint_text("填入服务提供的模型名称"),
                                            )
                                            .changed()
                                        {
                                            draft.settings_message.clear();
                                        }
                                    });
                                }
                                if profiles_ui(ui, &mut self.settings, draft, &profile_tool) {
                                    self.token.clear();
                                    self.discovery_token.clear();
                                    self.use_saved = false;
                                    self.credential_message.clear();
                                }
                                let value = crate::plugin_settings::Connection {
                                    endpoint: draft.endpoint.clone(),
                                    model: draft.model.clone(),
                                };
                                let changed = self.settings.get(&settings_key) != Some(&value);
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            changed
                                                && value.validate().is_ok()
                                                && self.settings.error.is_none(),
                                            egui::Button::new("保存连接设置"),
                                        )
                                        .clicked()
                                    {
                                        draft.settings_message =
                                            match self.settings.set(&settings_key, Some(value)) {
                                                Ok(()) => "连接设置已保存；未保存令牌".into(),
                                                Err(_) => "保存失败，请检查配置目录写入权限".into(),
                                            };
                                    }
                                    if ui
                                        .add_enabled(
                                            self.settings.error.is_none(),
                                            egui::Button::new("恢复插件默认"),
                                        )
                                        .clicked()
                                    {
                                        match self.settings.set(&settings_key, None) {
                                            Ok(()) => {
                                                draft.endpoint = default_endpoint.clone();
                                                draft.model = tool.model.clone();
                                                self.token.clear();
                                                self.discovery_token.clear();
                                                self.use_saved = false;
                                                self.credential_message.clear();
                                                draft.settings_message =
                                                    "已恢复插件默认；系统凭据保留".into();
                                            }
                                            Err(_) => {
                                                draft.settings_message =
                                                    "恢复失败，原设置保留".into()
                                            }
                                        }
                                    }
                                    if changed {
                                        ui.weak("当前编辑可直接运行；保存后重启仍可使用");
                                    }
                                });
                                if !draft.settings_message.is_empty() {
                                    ui.label(&draft.settings_message);
                                }
                                if let Some(error) = &self.settings.error {
                                    ui.colored_label(ui.visuals().error_fg_color, error);
                                }
                            });
                    });
                    *url = draft.endpoint.clone();
                    if let Err(error) = (crate::plugin_settings::Connection {
                        endpoint: url.clone(),
                        model: draft.model.clone(),
                    })
                    .validate()
                    {
                        connection_valid = false;
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                    ui.label(format!("请求目标：{method} {url}"));
                    if method == "POST" && plugins::uses_model(body) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("当前模型");
                            ui.strong(if draft.model.trim().is_empty() {
                                "尚未设置 · 展开连接设置或发现模型"
                            } else {
                                &draft.model
                            });
                        });
                    }
                    ui.add_enabled_ui(!busy && connection_valid, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.use_saved, "使用已保存凭据");
                            ui.label("临时 Bearer 令牌（可选）");
                            ui.add_enabled(!self.use_saved, egui::TextEdit::singleline(&mut self.token).password(true));
                            if ui.small_button("清空输入").clicked() { self.token.clear(); }
                        });
                        egui::CollapsingHeader::new("保存与管理凭据")
                            .id_salt(("credential-management", &id)).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                            if ui.add_enabled(!self.token.trim().is_empty() && !self.use_saved, egui::Button::new("保存 / 替换凭据")).clicked() {
                                self.credential_message = match crate::credentials::Target::plugin(&id, method, url).and_then(|target| crate::credentials::save(&target, &self.token)) {
                                    Ok(()) => { self.token.clear(); self.use_saved = true; "已保存到当前 Windows 用户的凭据管理器".into() },
                                    Err(e) => e.to_string(),
                                };
                            }
                            if ui.button("删除此工具凭据").clicked() {
                                self.credential_message = match crate::credentials::Target::plugin(&id, method, url).and_then(|target| crate::credentials::delete(&target)) {
                                    Ok(()) => { self.use_saved = false; "此工具当前地址的凭据已删除".into() },
                                    Err(e) => e.to_string(),
                                };
                            }
                            });
                            ui.label("凭据仅用于当前工具、请求方法和接口地址，保存在 Windows 凭据管理器。更换地址后需重新选择认证。");
                            ui.weak("删除不会中止已发送的请求。旧地址的凭据可在 Windows 凭据管理器中清理。");
                        });
                        if !self.credential_message.is_empty() { ui.label(&self.credential_message); }
                    });
                    if method == "POST" && plugins::uses_model(body) {
                        ui.toggle_value(&mut draft.discovery_open, "发现模型 / 检查列表连接");
                        if draft.discovery_open {
                            ui.group(|ui| {
                                let target = crate::model_discovery::target(url);
                                match &target {
                                    Ok(target) => { ui.label(format!("将发送 GET {target}")); }
                                    Err(error) => { ui.weak(error.to_string()); }
                                }
                                ui.label("仅查询模型列表，不发送对话或生成请求。查询认证需单独输入临时令牌；不会读取已保存的问答凭据。");
                                ui.horizontal_wrapped(|ui| {
                                    ui.label("列表查询令牌（可选）");
                                    ui.add_enabled(!busy, egui::TextEdit::singleline(&mut self.discovery_token).password(true));
                                    if ui.add_enabled(self.discovery_running.is_none() && self.running.is_none() && target.is_ok() && connection_valid, egui::Button::new("查询模型列表")).clicked() {
                                        let source = url.clone();
                                        let token = std::mem::take(&mut self.discovery_token);
                                        let (sender, receiver) = mpsc::channel();
                                        self.discovery_running = Some(DiscoveryJob { id: id.clone(), source: source.clone(), settings_key: settings_key.clone(), receiver });
                                        draft.models.clear();
                                        draft.models_source = source.clone();
                                        draft.discovery_message = "查询中，最长 10 秒；可切换页面".into();
                                        std::thread::spawn(move || {
                                            let result = crate::model_discovery::fetch(&source, token).map_err(|e| e.to_string());
                                            let _ = sender.send(result);
                                        });
                                    }
                                });
                                if draft.models_source == *url {
                                    if !draft.discovery_message.is_empty() { ui.label(&draft.discovery_message); }
                                    if !draft.models.is_empty() {
                                        ui.add_enabled_ui(!busy, |ui| {
                                            egui::ComboBox::from_id_salt("discovered-models")
                                                .selected_text("选择模型并填入名称").width(280.0).show_ui(ui, |ui| {
                                                    for model in &draft.models {
                                                        if ui.selectable_label(draft.model == *model, model).clicked() {
                                                            draft.model = model.clone();
                                                            draft.settings_message.clear();
                                                        }
                                                    }
                                                });
                                        });
                                    }
                                }
                            });
                        }
                    }
                } else {
                    ui.label("本地配方 · 无网络请求");
                }
                let chat_supported = crate::conversation::supported(&tool);
                let binding = format!(
                    "{}:{}",
                    crate::plugin_settings::Settings::key(&id, &tool),
                    draft.model
                );
                if draft.conversation.bind(binding.clone()) {
                    draft.chat_message = "连接、模型或工具定义已改变，旧会话已清空".into();
                }
                if chat_supported {
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.checkbox(&mut draft.chat_mode, "多轮上下文").changed()
                                && !draft.chat_mode
                            {
                                draft.conversation.clear();
                                draft.chat_message = "已切回单轮并清空会话".into();
                            }
                            if draft.chat_mode {
                                ui.label(format!(
                                    "{} / 16 轮 · {} / 256 KiB",
                                    draft.conversation.turns().len(),
                                    draft.conversation.bytes().div_ceil(1024)
                                ));
                                if ui.button("清空会话").clicked() {
                                    draft.conversation.clear();
                                    draft.output.clear();
                                    draft.chat_message = "会话已清空".into();
                                }
                            }
                        });
                    });
                    if draft.chat_mode {
                        ui.weak("仅保存在内存；发送时包含本工具此前成功的问答。不会自动调用工具；切换模型或地址会清空上下文。");
                        if !draft.conversation.turns().is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt(("conversation", &id))
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    for (index, (question, answer)) in
                                        draft.conversation.turns().iter().enumerate()
                                    {
                                        ui.strong(format!("你 · 第 {} 轮", index + 1));
                                        ui.label(question);
                                        ui.strong("模型");
                                        ui.label(answer);
                                        ui.separator();
                                    }
                                });
                        }
                    }
                    if !draft.chat_message.is_empty() {
                        ui.label(&draft.chat_message);
                    }
                } else {
                    draft.chat_mode = false;
                }
                ui.add_space(8.0);
                ui.separator();
                if !get_only {
                    ui.horizontal(|ui| {
                        ui.strong("输入");
                        if ui
                            .add_enabled(
                                !busy && draft.input.is_empty(),
                                egui::Button::new("填入示例"),
                            )
                            .clicked()
                        {
                            draft.input = tool.sample.clone();
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("清空输入与结果"))
                            .clicked()
                        {
                            draft.input.clear();
                            draft.output.clear();
                        }
                    });
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.add_sized(
                            [
                                ui.available_width(),
                                if draft.chat_mode { 100.0 } else { 160.0 },
                            ],
                            egui::TextEdit::multiline(&mut draft.input)
                                .font(egui::TextStyle::Monospace),
                        );
                    });
                } else {
                    ui.label("此工具只读取接口结果，无需输入文本或模型名称。");
                }
                let shortcut = allow_shortcuts
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));
                if (ui
                    .add_enabled(
                        self.running.is_none()
                            && self.discovery_running.is_none()
                            && connection_valid,
                        egui::Button::new("运行工具 · Ctrl Enter"),
                    )
                    .clicked()
                    || shortcut)
                    && self.running.is_none()
                    && self.discovery_running.is_none()
                    && connection_valid
                {
                    let messages = if draft.chat_mode {
                        match draft.conversation.prepare(&draft.input) {
                            Ok(messages) => Some(messages),
                            Err(error) => {
                                draft.chat_message = error.to_string();
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    let (input, model, token) = (
                        draft.input.clone(),
                        draft.model.clone(),
                        if self.use_saved {
                            String::new()
                        } else {
                            self.token.clone()
                        },
                    );
                    let use_saved = self.use_saved;
                    draft.pending_chat = draft.chat_mode.then(|| (binding, input.clone()));
                    let credential_id = id.clone();
                    self.token.clear();
                    let (tx, rx) = mpsc::channel();
                    self.running = Some((id, rx));
                    std::thread::spawn(move || {
                        let result = execute_with_credentials(
                            &tool,
                            &credential_id,
                            &input,
                            &model,
                            token,
                            use_saved,
                            messages.as_deref(),
                        )
                        .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    });
                }
                if self.running.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("执行中，请求最多等待 30 秒；可切换页面");
                    });
                }
                ui.horizontal(|ui| {
                    ui.strong("上次执行结果");
                    if ui
                        .add_enabled(!draft.output.is_empty(), egui::Button::new("复制结果"))
                        .clicked()
                    {
                        ui.ctx().copy_text(draft.output.clone());
                        self.copied_at = Some(std::time::Instant::now());
                    }
                });
                if self
                    .copied_at
                    .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2))
                {
                    ui.label("已复制结果");
                }
                let mut output = draft.output.as_str();
                ui.add_sized(
                    [ui.available_width(), 260.0],
                    egui::TextEdit::multiline(&mut output).font(egui::TextStyle::Monospace),
                );
            } else {
                ui.label("工具已停用或卸载，请返回管理页。");
            }
            return;
        }
        ui.collapsing("安装本地插件清单（JSON）", |ui| {
            ui.label(
                "先预览名称、工具和请求地址，再安装。安装后默认停用；不执行安装脚本，不加载 DLL。",
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("插件 JSON 文件的完整路径")
                        .desired_width(420.0),
                );
                if ui.button("读取预览").clicked() {
                    match plugins::read_manifest(std::path::Path::new(
                        self.path.trim().trim_matches('"'),
                    ))
                    .and_then(|b| {
                        plugins::parse(&b)?;
                        Ok(b)
                    }) {
                        Ok(bytes) => {
                            self.preview = Some(bytes);
                            self.message.clear();
                        }
                        Err(e) => {
                            self.preview = None;
                            self.message = e.to_string();
                        }
                    }
                }
            });
            if let Some(bytes) = self.preview.clone()
                && let Ok(m) = plugins::parse(&bytes)
            {
                ui.strong(format!("{} · v{} · {}", m.name, m.version, m.id));
                ui.label(m.description);
                for tool in &m.tools {
                    ui.label(format!("{} · {}", tool.name, tool.category));
                    if let Adapter::Http { url, method, .. } = &tool.adapter {
                        ui.label(format!("网络权限：{method} {url}"));
                    }
                }
                if ui.button("安装此清单（默认停用）").clicked() {
                    self.message = match self.store.install(&bytes) {
                        Ok(()) => {
                            self.preview = None;
                            "安装完成，请核对地址后启用".into()
                        }
                        Err(e) => e.to_string(),
                    };
                }
            }
        });
        egui::CollapsingHeader::new("官方示例 · 无需下载即可安装")
            .default_open(true)
            .show(ui, |ui| {
                for (name, bytes) in [
                    (
                        "本地文本配方",
                        include_bytes!("../plugins-examples/local-text.json").as_slice(),
                    ),
                    (
                        "Ollama 连接器",
                        include_bytes!("../plugins-examples/ollama.json").as_slice(),
                    ),
                    (
                        "OpenAI 兼容连接器",
                        include_bytes!("../plugins-examples/openai-compatible.json").as_slice(),
                    ),
                ] {
                    if ui.button(format!("安装 {name}")).clicked() {
                        self.message = match self.store.install(bytes) {
                            Ok(()) => "已安装，默认停用；请核对声明后启用".into(),
                            Err(e) => e.to_string(),
                        };
                    }
                }
            });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        for error in &self.store.errors {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.add_space(14.0);
        if ui
            .add_enabled(
                self.running.is_none() && self.discovery_running.is_none(),
                egui::Button::new("刷新已安装清单"),
            )
            .clicked()
        {
            self.store = Store::load(self.store.root.clone());
            self.clear_token();
        }
        ui.strong(format!("已安装 {} 个插件", self.store.packages.len()));
        enum Action {
            Enable(String, bool),
            Remove(String),
            Open(String),
        }
        let mut action = None;
        for package in &self.store.packages {
            let m = &package.manifest;
            egui::Frame::group(ui.style())
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(12)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 4.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(&m.name);
                        ui.label(format!("v{} · {} 个工具", m.version, m.tools.len()));
                        ui.label(if package.enabled {
                            "已启用"
                        } else {
                            "已停用 / 内容变更后需重新启用"
                        });
                    });
                    ui.label(&m.description);
                    for tool in &m.tools {
                        if let Adapter::Http { url, method, .. } = &tool.adapter {
                            ui.label(format!("网络声明：{method} {url}"));
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.running.is_none() && self.discovery_running.is_none(),
                                egui::Button::new(if package.enabled {
                                    "停用"
                                } else if m
                                    .tools
                                    .iter()
                                    .any(|t| matches!(t.adapter, Adapter::Http { .. }))
                                {
                                    "启用并允许上述网络请求"
                                } else {
                                    "启用本地处理插件"
                                }),
                            )
                            .clicked()
                        {
                            action = Some(Action::Enable(m.id.clone(), !package.enabled));
                        }
                        if ui
                            .add_enabled(
                                self.running.is_none() && self.discovery_running.is_none(),
                                egui::Button::new("卸载"),
                            )
                            .clicked()
                        {
                            self.pending_remove = Some(m.id.clone());
                        }
                    });
                    if self.pending_remove.as_deref() == Some(&m.id) {
                        ui.horizontal(|ui| {
                            ui.label("移除清单与启用状态？");
                            if ui.button("确认卸载").clicked() {
                                action = Some(Action::Remove(m.id.clone()));
                            }
                            if ui.button("取消").clicked() {
                                self.pending_remove = None;
                            }
                        });
                    }
                    if package.enabled {
                        for t in &m.tools {
                            if ui.button(format!("打开 {} →", t.name)).clicked() {
                                action = Some(Action::Open(format!("plugin:{}/{}", m.id, t.id)));
                            }
                        }
                    }
                });
            ui.add_space(8.0);
        }
        match action {
            Some(Action::Enable(id, enabled)) => {
                self.message = match self.store.set_enabled(&id, enabled) {
                    Ok(()) => if enabled {
                        "插件已启用"
                    } else {
                        "插件已停用"
                    }
                    .into(),
                    Err(e) => e.to_string(),
                };
                self.clear_token();
            }
            Some(Action::Remove(id)) => {
                match self.store.uninstall(&id) {
                    Ok(()) => {
                        self.drafts
                            .retain(|key, _| !key.starts_with(&format!("plugin:{id}/")));
                        self.message = "插件已卸载".into();
                    }
                    Err(e) => self.message = e.to_string(),
                }
                self.clear_token();
                self.pending_remove = None;
            }
            Some(Action::Open(id)) => self.select(&id),
            None => {}
        }
        if self.running.is_some() {
            ui.label("有工具正在运行，完成后可停用或卸载。");
        }
    }
}

fn profiles_ui(
    ui: &mut egui::Ui,
    settings: &mut crate::plugin_settings::Settings,
    draft: &mut Draft,
    tool: &plugins::PluginTool,
) -> bool {
    use crate::plugin_settings::{Connection, Profile, request_shape};
    let Some(shape) = request_shape(tool) else {
        return false;
    };
    let mut applied = false;
    let selected_name = draft
        .profile_id
        .as_deref()
        .and_then(|id| settings.profile(id))
        .map_or("选择兼容档案", |p| p.name.as_str())
        .to_owned();
    ui.horizontal_wrapped(|ui| {
        ui.label("连接档案");
        egui::ComboBox::from_id_salt("connection-profile")
            .selected_text(selected_name)
            .width(200.0)
            .show_ui(ui, |ui| {
                let mut profiles: Vec<_> = settings
                    .profiles()
                    .filter(|(_, p)| p.shape == shape)
                    .collect();
                profiles.sort_by(|a, b| a.1.name.cmp(&b.1.name));
                if profiles.is_empty() {
                    ui.weak("尚无兼容档案，可保存当前连接");
                }
                for (id, profile) in profiles {
                    if ui
                        .selectable_label(draft.profile_id.as_ref() == Some(id), &profile.name)
                        .on_hover_text(&profile.connection.endpoint)
                        .clicked()
                    {
                        draft.profile_id = Some(id.clone());
                        draft.profile_name = profile.name.clone();
                        draft.profile_delete = false;
                    }
                }
            });
        let selected = draft
            .profile_id
            .as_deref()
            .and_then(|id| settings.profile(id));
        if ui
            .add_enabled(
                selected.is_some_and(|p| p.shape == shape),
                egui::Button::new("应用到当前工具"),
            )
            .clicked()
        {
            if let Some(value) = selected.and_then(|p| p.for_tool(tool).ok()) {
                draft.endpoint = value.endpoint;
                draft.model = value.model;
                draft.settings_message =
                    "已应用档案；请核对目标并重新选择认证，保存连接设置后可在重启时复用".into();
                applied = true;
            }
        }
        ui.toggle_value(&mut draft.profile_editor, "管理档案");
    });
    if draft.profile_editor {
        ui.group(|ui| {
            ui.label("档案只包含接口与模型；应用时复制配置，不自动发送请求。仅显示请求和响应格式兼容的档案。");
            ui.horizontal_wrapped(|ui| {
                ui.label("档案名称");
                ui.add(egui::TextEdit::singleline(&mut draft.profile_name).hint_text("例如：本机推理 / 开发环境"));
                let profile = Profile { name: draft.profile_name.clone(), shape: shape.clone(), connection: Connection { endpoint: draft.endpoint.clone(), model: draft.model.clone() } };
                let valid = !profile.name.trim().is_empty() && profile.connection.validate().is_ok() && settings.error.is_none();
                if ui.add_enabled(valid, egui::Button::new("另存新档案")).clicked() {
                    match settings.save_profile(None, profile.clone()) {
                        Ok(id) => { draft.profile_id = Some(id); draft.profile_delete = false; draft.settings_message = "档案已创建；未保存认证信息".into(); }
                        Err(e) => draft.settings_message = e.to_string(),
                    }
                }
                if ui.add_enabled(valid && draft.profile_id.is_some(), egui::Button::new("更新选中档案")).clicked() {
                    draft.settings_message = match settings.save_profile(draft.profile_id.as_deref(), profile) {
                        Ok(_) => "档案已更新；已应用到其他工具的设置保持原值".into(),
                        Err(e) => e.to_string(),
                    };
                }
                if ui.add_enabled(draft.profile_id.is_some() && settings.error.is_none(), egui::Button::new("删除档案")).clicked() { draft.profile_delete = true; }
            });
            if draft.profile_delete {
                ui.horizontal_wrapped(|ui| {
                    ui.label("删除选中档案？已应用的工具设置与系统凭据会保留。");
                    if ui.button("确认删除").clicked() {
                        if let Some(id) = &draft.profile_id {
                            match settings.delete_profile(id) {
                                Ok(()) => { draft.profile_id = None; draft.profile_name.clear(); draft.settings_message = "档案已删除".into(); }
                                Err(e) => draft.settings_message = e.to_string(),
                            }
                        }
                        draft.profile_delete = false;
                    }
                    if ui.button("取消").clicked() { draft.profile_delete = false; }
                });
            }
        });
    }
    applied
}

fn execute_with_credentials(
    tool: &plugins::PluginTool,
    id: &str,
    input: &str,
    model: &str,
    temporary: String,
    saved: bool,
    messages: Option<&[serde_json::Value]>,
) -> anyhow::Result<String> {
    let secret = if saved {
        let Adapter::Http { url, method, .. } = &tool.adapter else {
            anyhow::bail!("本地配方不使用网络凭据");
        };
        let target = crate::credentials::Target::plugin(id, method, url)?;
        Some(crate::credentials::read(&target)?.ok_or_else(|| {
            anyhow::anyhow!("当前工具和接口地址没有已保存凭据，请先保存或取消勾选")
        })?)
    } else if temporary.trim().is_empty() {
        None
    } else {
        Some(crate::credentials::Secret::new(temporary)?)
    };
    let token = secret.as_ref().map_or("", |s| s.expose());
    let redact = |text: String| {
        if token.is_empty() {
            text
        } else {
            let escaped = serde_json::to_string(token).unwrap_or_default();
            text.replace(token, "[REDACTED]").replace(
                escaped
                    .get(1..escaped.len().saturating_sub(1))
                    .unwrap_or(token),
                "[REDACTED]",
            )
        }
    };
    plugins::execute_with_context(tool, input, model, token, messages)
        .map(redact)
        .map_err(|e| anyhow::anyhow!(redact(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_failure_keeps_draft_and_success_commits_once_in_original_tool() {
        let mut state = PluginState::new(
            std::env::temp_dir().join(format!("zi-chat-{}", uuid::Uuid::new_v4())),
        );
        let draft = state.drafts.entry("chat".into()).or_default();
        draft.conversation.bind("binding".into());
        draft.input = "question".into();
        for success in [false, true] {
            let draft = state.drafts.get_mut("chat").unwrap();
            draft.pending_chat = Some(("binding".into(), draft.input.clone()));
            let (sender, receiver) = mpsc::channel();
            state.running = Some(("chat".into(), receiver));
            state.select("other");
            sender
                .send(if success {
                    Ok("answer".into())
                } else {
                    Err("fixture error".into())
                })
                .unwrap();
            assert!(state.poll().is_some());
            let draft = &state.drafts["chat"];
            assert_eq!(draft.conversation.turns().len(), usize::from(success));
            assert_eq!(draft.input.is_empty(), success);
            assert!(draft.pending_chat.is_none());
            assert!(state.poll().is_none());
        }
        assert!(!state.drafts.contains_key("other"));
    }
    #[test]
    fn discovery_result_returns_to_source_tool_and_discards_changed_destination() {
        let root = std::env::temp_dir().join(format!("zi-discovery-ui-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root);
        state.drafts.insert(
            "first".into(),
            Draft {
                endpoint: "http://localhost:1234/v1/chat/completions".into(),
                settings_key: "original".into(),
                ..Default::default()
            },
        );
        for changed in [false, true] {
            let (sender, receiver) = mpsc::channel();
            state.discovery_running = Some(DiscoveryJob {
                id: "first".into(),
                source: "http://localhost:1234/v1/chat/completions".into(),
                settings_key: "original".into(),
                receiver,
            });
            state.discovery_token = "synthetic-only".into();
            state.select("second");
            state.clear_token();
            assert!(state.discovery_token.is_empty());
            if changed {
                let draft = state.drafts.get_mut("first").unwrap();
                draft.endpoint = "http://localhost:4321/v1/chat/completions".into();
                draft.models.clear();
            }
            sender
                .send(Ok(crate::model_discovery::Report {
                    models: vec!["fixture".into()],
                    elapsed_ms: 1,
                }))
                .unwrap();
            assert!(state.is_running());
            assert!(state.poll().is_some());
            assert!(!state.is_running());
            assert_eq!(state.drafts["first"].models.is_empty(), changed);
            assert!(!state.drafts.contains_key("second"));
        }
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "explicit isolated Windows vault and localhost HTTP integration"]
    fn saved_credential_reaches_only_bound_tool_and_is_redacted_from_response() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/test", listener.local_addr().unwrap());
        let id = format!("plugin:test-{}/request", uuid::Uuid::new_v4());
        let target = crate::credentials::Target::plugin(&id, "GET", &url).unwrap();
        struct Cleanup(crate::credentials::Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = crate::credentials::delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        crate::credentials::save(&cleanup.0, "synthetic-fixture-token").unwrap();
        let tool = plugins::PluginTool {
            id: "request".into(),
            name: "Fixture".into(),
            description: String::new(),
            category: "AI 与模型".into(),
            keywords: vec![],
            sample: String::new(),
            model: String::new(),
            adapter: Adapter::Http {
                url: url.clone(),
                method: "GET".into(),
                body: serde_json::Value::Null,
                response_pointer: Some("/output".into()),
            },
        };
        // A different tool identity must fail before sending any request.
        assert!(
            execute_with_credentials(
                &tool,
                &format!("{id}-other"),
                "",
                "",
                String::new(),
                true,
                None
            )
            .is_err()
        );
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "fixture request timed out"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => panic!("fixture accept failed"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
            }
            let authorized = String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-fixture-token");
            let body = r#"{"output":"synthetic-fixture-token"}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            authorized
        });
        let result =
            execute_with_credentials(&tool, &id, "", "", String::new(), true, None).unwrap();
        assert!(server.join().unwrap());
        assert_eq!(result, "[REDACTED]");
    }
    #[test]
    fn background_result_keeps_own_draft_and_clears_busy() {
        let root = std::env::temp_dir().join(format!("zi-plugin-ui-{}", uuid::Uuid::new_v4()));
        let mut state = PluginState::new(root);
        let (tx, rx) = mpsc::channel();
        state.running = Some(("first".into(), rx));
        state.select("second");
        assert!(state.poll().is_none());
        tx.send(Ok("done".into())).unwrap();
        assert!(state.poll().is_some());
        assert!(!state.is_running());
        assert_eq!(state.drafts["first"].output, "done");
        assert_eq!(state.selected.as_deref(), Some("second"));
        state.token = "test-only".into();
        state.use_saved = true;
        state.select("first");
        assert!(state.token.is_empty());
        assert!(!state.use_saved);
        let (tx, rx) = mpsc::channel();
        state.running = Some(("first".into(), rx));
        drop(tx);
        assert!(state.poll().unwrap().contains("失败"));
        assert!(!state.is_running());
    }
}
