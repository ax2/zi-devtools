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
    credential_message: String,
    pending_remove: Option<String>,
    copied_at: Option<std::time::Instant>,
    running: Option<(String, Receiver<Result<String, String>>)>,
}
impl PluginState {
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
    }
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }
    pub fn poll(&mut self) -> Option<String> {
        let (id, rx) = self.running.as_ref()?;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => Err("执行线程已退出".into()),
            Err(mpsc::TryRecvError::Empty) => return None,
        };
        let succeeded = result.is_ok();
        self.drafts.entry(id.clone()).or_default().output =
            result.unwrap_or_else(|e| format!("失败：{e}"));
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
                let busy = self.running.as_ref().is_some_and(|(key, _)| key == &id);
                let get_only = matches!(&tool.adapter,Adapter::Http{method,..} if method=="GET");
                ui.heading(&tool.name);
                ui.label(&tool.description);
                let settings_key = crate::plugin_settings::Settings::key(&id, &tool);
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
                    self.token.clear();
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
                            .default_open(true)
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
                    ui.small("仅点击运行时发送输入；超时 30 秒。修改地址会清空临时令牌并取消使用已保存凭据。");
                    ui.add_enabled_ui(!busy && connection_valid, |ui| {
                        ui.checkbox(&mut self.use_saved, "本次使用 Windows 已保存凭据");
                        ui.horizontal_wrapped(|ui| {
                            ui.label("临时 Bearer 令牌（可选）");
                            ui.add_enabled(!self.use_saved, egui::TextEdit::singleline(&mut self.token).password(true));
                            if ui.small_button("清空输入").clicked() { self.token.clear(); }
                        });
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
                        ui.small("凭据绑定当前插件工具、方法及精确接口地址，不写入 JSON。更换地址后不会使用旧凭据；旧条目可在 Windows 凭据管理器中删除。删除前可重新输入并保存，删除不会中止已发出的请求。");
                        if !self.credential_message.is_empty() { ui.label(&self.credential_message); }
                    });
                } else {
                    ui.label("本地配方 · 无网络请求");
                }
                ui.add_space(8.0);
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
                            [ui.available_width(), 160.0],
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
                        self.running.is_none() && connection_valid,
                        egui::Button::new("运行工具 · Ctrl Enter"),
                    )
                    .clicked()
                    || shortcut)
                    && self.running.is_none()
                    && connection_valid
                {
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
            .add_enabled(self.running.is_none(), egui::Button::new("刷新已安装清单"))
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
                                self.running.is_none(),
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
                            .add_enabled(self.running.is_none(), egui::Button::new("卸载"))
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

fn execute_with_credentials(
    tool: &plugins::PluginTool,
    id: &str,
    input: &str,
    model: &str,
    temporary: String,
    saved: bool,
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
    plugins::execute(tool, input, model, token)
        .map(redact)
        .map_err(|e| anyhow::anyhow!(redact(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
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
            execute_with_credentials(&tool, &format!("{id}-other"), "", "", String::new(), true)
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
        let result = execute_with_credentials(&tool, &id, "", "", String::new(), true).unwrap();
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
