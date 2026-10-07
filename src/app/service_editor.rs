use super::*;
use crate::config::ServiceSpec;
use std::collections::BTreeMap;

#[derive(serde::Serialize, serde::Deserialize)]
struct Advanced {
    #[serde(default)]
    stop_command: Option<String>,
    #[serde(default = "stop_timeout")]
    stop_timeout_ms: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    config_files: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, Option<String>>,
}
fn stop_timeout() -> u64 {
    5_000
}

pub(super) struct Editor {
    original: Option<ServiceSpec>,
    spec: ServiceSpec,
    repo: String,
    port: String,
    health: String,
    advanced: String,
    show_advanced: bool,
}
impl Editor {
    fn new(spec: Option<ServiceSpec>) -> Self {
        let value = spec.clone().unwrap_or(ServiceSpec {
            id: String::new(),
            name: String::new(),
            repo: PathBuf::new(),
            command: String::new(),
            stop_command: None,
            stop_timeout_ms: 5_000,
            description: String::new(),
            health_url: None,
            port: None,
            config_files: Vec::new(),
            env: BTreeMap::new(),
            tags: Vec::new(),
        });
        let advanced = Advanced {
            stop_command: value.stop_command.clone(),
            stop_timeout_ms: value.stop_timeout_ms,
            tags: value.tags.clone(),
            config_files: value.config_files.clone(),
            env: value.env.clone(),
        };
        Self {
            repo: value.repo.display().to_string(),
            port: value.port.map(|p| p.to_string()).unwrap_or_default(),
            health: value.health_url.clone().unwrap_or_default(),
            advanced: serde_yaml_ng::to_string(&advanced).unwrap_or_default(),
            original: spec,
            spec: value,
            show_advanced: false,
        }
    }
    fn value(&self) -> anyhow::Result<ServiceSpec> {
        let mut value = self.spec.clone();
        value.id = value.id.trim().to_owned();
        value.name = value.name.trim().to_owned();
        if value.name.is_empty() {
            value.name.clone_from(&value.id);
        }
        value.repo = PathBuf::from(self.repo.trim());
        value.port = if self.port.trim().is_empty() {
            None
        } else {
            Some(
                self.port
                    .trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("端口须为 1–65535"))?,
            )
        };
        value.health_url = (!self.health.trim().is_empty()).then(|| self.health.trim().to_owned());
        let advanced: Advanced = serde_yaml_ng::from_str(&self.advanced)
            .map_err(|e| anyhow::anyhow!("高级选项 YAML 无效：{e}"))?;
        value.stop_command = advanced.stop_command;
        value.stop_timeout_ms = advanced.stop_timeout_ms;
        value.tags = advanced.tags;
        value.config_files = advanced.config_files;
        value.env = advanced.env;
        crate::config::validate_service_edit(&value)?;
        Ok(value)
    }
}

#[derive(Default)]
pub(super) struct State {
    editor: Option<Editor>,
    delete: Option<ServiceSpec>,
    receiver: Option<Receiver<Result<(), String>>>,
    error: String,
    #[cfg(feature = "ui-preview")]
    pub rects: std::collections::HashMap<&'static str, (egui::Rect, egui::Rect)>,
}
impl State {
    pub fn has_work(&self) -> bool {
        self.editor.is_some() || self.delete.is_some() || self.receiver.is_some()
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fill(&mut self, spec: ServiceSpec) {
        let original = self.editor.as_ref().and_then(|e| e.original.clone());
        let mut editor = Editor::new(Some(spec));
        editor.original = original;
        self.editor = Some(editor);
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ready(&self) -> bool {
        self.editor.is_none() && self.receiver.is_none() && self.error.is_empty()
    }
    pub fn new_service(&mut self) {
        if self.receiver.is_none() {
            self.editor = Some(Editor::new(None));
            self.error.clear();
        }
    }
    pub fn edit(&mut self, manager: &ServiceManager, id: &str) {
        if self.receiver.is_some() {
            return;
        }
        match manager.service_spec(id) {
            Ok(spec) => {
                self.editor = Some(Editor::new(Some(spec)));
                self.error.clear();
            }
            Err(e) => self.error = e.to_string(),
        }
    }
    pub fn delete(&mut self, manager: &ServiceManager, id: &str) {
        if self.receiver.is_none() {
            self.delete = manager.service_spec(id).ok();
            self.error.clear();
        }
    }
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Err("服务配置后台操作中断，请重试".into())
            }
        };
        self.receiver = None;
        match &result {
            Ok(()) => {
                self.editor = None;
                self.delete = None;
                self.error.clear();
            }
            Err(e) => self.error.clone_from(e),
        }
        Some(result)
    }
    fn submit(
        &mut self,
        manager: Arc<ServiceManager>,
        ctx: egui::Context,
        original: Option<ServiceSpec>,
        spec: Option<ServiceSpec>,
    ) {
        let (tx, rx) = unbounded();
        self.receiver = Some(rx);
        self.error.clear();
        std::thread::spawn(move || {
            let result = match spec {
                Some(spec) => manager.save_service(original.as_ref(), spec),
                None => manager.delete_service(&original.expect("delete original")),
            };
            let _ = tx.send(result.map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
    }
    pub fn ui(&mut self, ctx: &egui::Context, manager: &Arc<ServiceManager>) {
        let busy = self.receiver.is_some();
        let mut save = false;
        let mut dismiss = false;
        if let Some(editor) = &mut self.editor {
            let mut open = true;
            egui::Window::new(if editor.original.is_some(){"编辑本地服务"}else{"新增本地服务"}).open(&mut open).collapsible(false).default_width(620.0).resizable(true).show(ctx,|ui| {
                ui.label("保存仅更新服务定义，不会自动运行命令。运行中的托管服务须先停止。");
                ui.add_enabled_ui(!busy,|ui| {
                    egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height((ctx.screen_rect().height()-240.0).max(160.0)).show(ui,|ui| {
                        ui.label("服务标识（保存后不可改名）");ui.add_enabled(editor.original.is_none(),egui::TextEdit::singleline(&mut editor.spec.id).char_limit(64).desired_width(f32::INFINITY));
                        ui.label("显示名称");ui.add(egui::TextEdit::singleline(&mut editor.spec.name).char_limit(160).desired_width(f32::INFINITY));
                        ui.label("工作目录");ui.horizontal(|ui| {ui.add(egui::TextEdit::singleline(&mut editor.repo).char_limit(2048).desired_width((ui.available_width()-80.0).max(80.0)));if ui.button("选择…").clicked()&&let Some(path)=rfd::FileDialog::new().pick_folder(){editor.repo=path.display().to_string();}});
                        ui.label("启动命令");ui.add(egui::TextEdit::multiline(&mut editor.spec.command).char_limit(16_384).desired_rows(2).desired_width(f32::INFINITY));
                        ui.label("描述");ui.add(egui::TextEdit::singleline(&mut editor.spec.description).char_limit(1000).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {ui.label("端口（可选）");ui.add(egui::TextEdit::singleline(&mut editor.port).char_limit(5).desired_width(90.0));});
                        ui.label("健康检查 HTTP/HTTPS 地址（可选）");ui.add(egui::TextEdit::singleline(&mut editor.health).char_limit(2048).desired_width(f32::INFINITY));
                        ui.separator();ui.checkbox(&mut editor.show_advanced,"展开高级选项（包含环境变量值，请勿截屏分享）");
                        if editor.show_advanced {ui.small("YAML：优雅停止命令/超时、标签、配置文件、环境变量；修改将随保存生效。");ui.add(egui::TextEdit::multiline(&mut editor.advanced).code_editor().char_limit(65_536).desired_rows(8).desired_width(f32::INFINITY));}
                    });
                    ui.separator();ui.horizontal(|ui|{let button=ui.button("保存定义"); #[cfg(feature = "ui-preview")] self.rects.insert("save",(button.rect,ui.clip_rect())); save=button.clicked();dismiss=ui.button("取消").clicked();});
                });
                if busy {ui.spinner();ui.label("正在保存…");}
                if !self.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&self.error);}
            });
            if !open && !busy {
                dismiss = true;
            }
        }
        if dismiss {
            self.editor = None;
            self.error.clear();
        }
        if save && let Some(editor) = &self.editor {
            match editor.value() {
                Ok(spec) => self.submit(
                    manager.clone(),
                    ctx.clone(),
                    editor.original.clone(),
                    Some(spec),
                ),
                Err(e) => self.error = e.to_string(),
            }
        }
        let mut confirm = false;
        let mut cancel = false;
        if let Some(spec) = &self.delete {
            egui::Window::new("删除服务定义").collapsible(false).resizable(false).show(ctx,|ui| {
                ui.label(format!("移除“{}”的服务定义？",spec.name));ui.label("仅移除配置与自动恢复意图；工作目录和日志保留。\n运行中的托管服务须先停止，外部进程不会被结束。");
                ui.add_enabled_ui(!busy,|ui| {ui.horizontal(|ui| {let button=ui.button("确认删除"); #[cfg(feature = "ui-preview")] self.rects.insert("confirm",(button.rect,ui.clip_rect())); confirm=button.clicked();cancel=ui.button("取消").clicked();});});
                if busy{ui.spinner();}
                if !self.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&self.error);}
            });
        }
        if confirm {
            self.submit(manager.clone(), ctx.clone(), self.delete.clone(), None);
        }
        if cancel {
            self.delete = None;
            self.error.clear();
        }
    }
}
