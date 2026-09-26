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
}
pub struct PluginState {
    pub store: Store,
    pub selected: Option<String>,
    path: String,
    preview: Option<Vec<u8>>,
    message: String,
    drafts: HashMap<String, Draft>,
    token: String,
    pending_remove: Option<String>,
    running: Option<(String, Receiver<Result<String, String>>)>,
}
impl PluginState {
    pub fn new(root: PathBuf) -> Self {
        Self {
            store: Store::load(root),
            selected: None,
            path: String::new(),
            preview: None,
            message: String::new(),
            drafts: HashMap::new(),
            token: String::new(),
            pending_remove: None,
            running: None,
        }
    }
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.token.clear();
        }
        self.selected = Some(id.into());
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some((id, rx)) = &self.running {
            match rx.try_recv() {
                Ok(result) => {
                    self.drafts.entry(id.clone()).or_default().output =
                        result.unwrap_or_else(|e| format!("失败：{e}"));
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "执行线程已退出".into();
                    self.running = None;
                }
                _ => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        ui.heading(egui::RichText::new("插件与连接器").size(28.0));
        ui.label(
            "安装工具定义，按需连接本机或远程服务。已启用工具会自动进入分类、收藏与 Ctrl K 搜索。",
        );
        ui.add_space(12.0);
        if self.selected.is_some() && ui.button("← 返回插件管理").clicked() {
            self.selected = None;
            self.token.clear();
        }
        if let Some(id) = self.selected.clone() {
            if let Some((_, tool)) = self.store.tools().into_iter().find(|(key, _)| key == &id) {
                ui.heading(&tool.name);
                ui.label(&tool.description);
                let draft = self.drafts.entry(id.clone()).or_insert_with(|| Draft {
                    model: tool.model.clone(),
                    ..Default::default()
                });
                if let Adapter::Http { url, method, .. } = &tool.adapter {
                    ui.label(format!("请求目标：{method} {url}"));
                    ui.label(
                        "仅点击运行时发送输入；请求超时 30 秒。令牌不写入配置，切换工具时清除。",
                    );
                    ui.horizontal(|ui| {
                        ui.label("模型名称");
                        ui.text_edit_singleline(&mut draft.model);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bearer 令牌（可选）");
                        ui.add(egui::TextEdit::singleline(&mut self.token).password(true));
                        if ui.small_button("清除令牌").clicked() {
                            self.token.clear();
                        }
                    });
                } else {
                    ui.label("本地配方 · 无网络请求");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.strong("输入");
                    if ui
                        .add_enabled(draft.input.is_empty(), egui::Button::new("填入示例"))
                        .clicked()
                    {
                        draft.input = tool.sample.clone();
                    }
                    if ui.button("清空输入与结果").clicked() {
                        draft.input.clear();
                        draft.output.clear();
                    }
                });
                ui.add_sized(
                    [ui.available_width(), 160.0],
                    egui::TextEdit::multiline(&mut draft.input).font(egui::TextStyle::Monospace),
                );
                if ui
                    .add_enabled(self.running.is_none(), egui::Button::new("运行工具"))
                    .clicked()
                {
                    let (input, model, token) =
                        (draft.input.clone(), draft.model.clone(), self.token.clone());
                    let (tx, rx) = mpsc::channel();
                    self.running = Some((id, rx));
                    std::thread::spawn(move || {
                        let result = plugins::execute(&tool, &input, &model, &token)
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
                    ui.strong("结果");
                    if ui
                        .add_enabled(!draft.output.is_empty(), egui::Button::new("复制结果"))
                        .clicked()
                    {
                        ui.ctx().copy_text(draft.output.clone());
                    }
                });
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
            self.token.clear();
        }
        ui.strong(format!("已安装 {} 个插件", self.store.packages.len()));
        for package in self.store.packages.clone() {
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
                            if let Err(e) = self.store.set_enabled(&m.id, !package.enabled) {
                                self.message = e.to_string();
                            }
                            self.token.clear();
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
                                if let Err(e) = self.store.uninstall(&m.id) {
                                    self.message = e.to_string();
                                }
                                self.drafts
                                    .retain(|id, _| !id.starts_with(&format!("plugin:{}/", m.id)));
                                self.token.clear();
                                self.pending_remove = None;
                            }
                            if ui.button("取消").clicked() {
                                self.pending_remove = None;
                            }
                        });
                    }
                    if package.enabled {
                        for t in &m.tools {
                            if ui.button(format!("打开 {} →", t.name)).clicked() {
                                self.select(&format!("plugin:{}/{}", m.id, t.id));
                            }
                        }
                    }
                });
            ui.add_space(8.0);
        }
        if self.running.is_some() {
            ui.label("有工具正在运行，完成后可停用或卸载。");
        }
    }
}
