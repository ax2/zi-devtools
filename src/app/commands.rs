use super::*;
use crate::commands::{Action, Command, Match};

pub(super) struct Prefix {
    pub active: bool,
    sequence: String,
    message: String,
    last: Instant,
    had_focus: bool,
    draft: Option<crate::commands::Bindings>,
    query: String,
    selected: String,
    input: String,
    notice: String,
    #[cfg(feature = "ui-preview")]
    rects: [egui::Rect; 6],
}
impl Default for Prefix {
    fn default() -> Self {
        Self {
            active: false,
            sequence: String::new(),
            message: String::new(),
            last: Instant::now(),
            had_focus: false,
            draft: None,
            query: String::new(),
            selected: String::new(),
            input: String::new(),
            notice: String::new(),
            #[cfg(feature = "ui-preview")]
            rects: [egui::Rect::NOTHING; 6],
        }
    }
}
impl Prefix {
    pub fn open(&mut self) {
        self.active = true;
        self.sequence.clear();
        self.message.clear();
        self.last = Instant::now();
        self.had_focus = false;
    }
}
impl DevToolsApp {
    pub(super) fn commands_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("统一快捷指令");
        let version = catalog()
            .iter()
            .find(|entry| entry.id == "unified-shortcuts")
            .and_then(|entry| entry.version.as_deref())
            .unwrap_or("未声明");
        ui.label(format!(
            "v{version} · 开发中：全局前缀、自定义键序列与保存校验"
        ));
        ui.small("全局前缀在设置中修改。附加键逐次输入，支持 1–4 个字母；K 保留给全工具搜索。");
        if self.prefix.active && !self.quick_open {
            if let Some(id) = self.prefix_ui(ui, ctx)
                && let Some(entry) = self.entries("").iter().find(|e| e.id == id).cloned()
            {
                self.open_entry(&entry);
            }
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("在本窗口输入键序列").clicked() {
                self.prefix.open();
            }
            if ui.button("打开全局快捷面板").clicked() {
                self.open_quick(ctx);
                self.prefix.open();
            }
            if ui.button("修改全局前缀").clicked() {
                self.page = Page::Settings;
            }
        });
        ui.add_space(10.0);
        self.binding_editor(ui);
    }

    fn binding_editor(&mut self, ui: &mut egui::Ui) {
        let defaults = self.default_commands();
        let draft = self
            .prefix
            .draft
            .get_or_insert_with(|| self.preferences.command_bindings.clone());
        let mut rows = defaults.clone();
        for row in &mut rows {
            if let Some(sequence) = draft.get(&row.id) {
                row.sequence = sequence.clone();
            }
        }
        rows.extend(
            draft
                .iter()
                .take(4096)
                .filter(|(id, _)| !defaults.iter().any(|c| &c.id == *id))
                .map(|(id, sequence)| Command {
                    id: id.clone(),
                    title: format!("暂不可用 · {id}"),
                    sequence: sequence.clone(),
                    action: Action::Open(String::new()),
                }),
        );
        let (_, errors) = crate::commands::configured(&defaults, draft);
        let dirty = *draft != self.preferences.command_bindings;
        ui.horizontal_wrapped(|ui| {
            ui.label(if dirty {
                "● 有未保存的修改"
            } else {
                "✓ 已保存的配置"
            });
            let save = ui.add_enabled(dirty && errors.is_empty(), egui::Button::new("保存并生效"));
            #[cfg(feature = "ui-preview")]
            {
                self.prefix.rects[3] = save.rect;
            }
            if save.clicked() {
                match self
                    .preferences
                    .save_command_bindings(&self.preferences_path, draft.clone())
                {
                    Ok(()) => {
                        self.prefix.active = false;
                        self.prefix.sequence.clear();
                        self.prefix.notice = "已保存；下次打开快捷面板使用新绑定".into();
                    }
                    Err(e) => self.prefix.notice = format!("保存失败，当前绑定未改变：{e}"),
                }
            }
            let revert = ui.add_enabled(dirty, egui::Button::new("放弃修改"));
            #[cfg(feature = "ui-preview")]
            {
                self.prefix.rects[4] = revert.rect;
            }
            if revert.clicked() {
                *draft = self.preferences.command_bindings.clone();
                self.prefix.selected.clear();
                self.prefix.notice = "已撤回未保存修改".into();
            }
            let reset = ui.button("恢复默认（草稿）");
            #[cfg(feature = "ui-preview")]
            {
                self.prefix.rects[5] = reset.rect;
            }
            if reset.clicked() {
                draft.clear();
                self.prefix.selected.clear();
                self.prefix.notice = "默认绑定已放入草稿；保存后生效".into();
            }
        });
        if !self.prefix.notice.is_empty() {
            ui.label(&self.prefix.notice);
        }
        for error in errors.iter().take(5) {
            ui.colored_label(self.colors.amber, error);
        }
        if errors.len() > 5 {
            ui.label(format!("另有 {} 项冲突，请逐项修正", errors.len() - 5));
        }
        if !self.prefix.selected.is_empty() {
            let title = rows
                .iter()
                .find(|r| r.id == self.prefix.selected)
                .map(|r| r.title.as_str())
                .unwrap_or("暂不可用的工具");
            ui.separator();
            ui.label(format!("编辑：{title}"));
            ui.horizontal_wrapped(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.prefix.input)
                        .char_limit(64)
                        .desired_width(180.0)
                        .hint_text("例如 Q A；留空取消绑定"),
                );
                #[cfg(feature = "ui-preview")]
                {
                    self.prefix.rects[1] = input.rect;
                }
                #[cfg(not(feature = "ui-preview"))]
                let _ = input;
                let stage = ui.button("写入草稿");
                #[cfg(feature = "ui-preview")]
                {
                    self.prefix.rects[2] = stage.rect;
                }
                if stage.clicked() {
                    match crate::commands::normalize_sequence(&self.prefix.input) {
                        Ok(sequence) => {
                            draft.insert(self.prefix.selected.clone(), sequence.clone());
                            self.prefix.input = sequence;
                            self.prefix.notice = "修改已写入草稿，尚未生效".into();
                        }
                        Err(e) => self.prefix.notice = e,
                    }
                }
                if ui.button("使用此项默认").clicked() {
                    draft.remove(&self.prefix.selected);
                    self.prefix.selected.clear();
                }
            });
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut self.prefix.query)
                .hint_text("搜索工具名称或键序列")
                .char_limit(128)
                .desired_width(ui.available_width()),
        );
        ui.small(format!(
            "{} 个可用指令；未绑定工具可按 K 搜索。插件停用后保留绑定并预留键序列。",
            defaults.len()
        ));
        let query = self.prefix.query.to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("binding-list")
            .max_height(320.0)
            .show(ui, |ui| {
                for row in rows.iter().filter(|row| {
                    format!("{} {} {}", row.title, row.sequence, row.id)
                        .to_lowercase()
                        .contains(&query)
                }) {
                    ui.horizontal_wrapped(|ui| {
                        let bound = if row.sequence.is_empty() {
                            "未绑定"
                        } else {
                            &row.sequence
                        };
                        ui.monospace(bound);
                        let select = ui.add_enabled(
                            row.id != "search",
                            egui::Button::new(&row.title).selected(self.prefix.selected == row.id),
                        );
                        #[cfg(feature = "ui-preview")]
                        if row.id == "open:advanced-calculator" {
                            self.prefix.rects[0] = select.rect;
                        }
                        if select.clicked() {
                            self.prefix.selected = row.id.clone();
                            self.prefix.input = row.sequence.clone();
                        }
                        ui.small(if row.id == "search" {
                            "保留"
                        } else if !defaults.iter().any(|c| c.id == row.id) {
                            "暂不可用"
                        } else if draft.contains_key(&row.id) {
                            "自定义"
                        } else {
                            "默认"
                        });
                    });
                }
            });
        ui.small("空序列表示取消绑定；重复或前缀冲突会阻止保存。恢复默认和放弃修改都可先检查再保存。旧录屏直接热键继续可用。");
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_binding_fixture(&mut self) {
        self.prefix.query = "计算器".into();
        self.prefix.selected = "open:advanced-calculator".into();
        self.prefix.input = "Q A".into();
        self.prefix.draft = Some(self.preferences.command_bindings.clone());
        self.prefix
            .draft
            .as_mut()
            .unwrap()
            .insert(self.prefix.selected.clone(), "Q A".into());
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_binding_position(&self, index: usize) -> egui::Pos2 {
        self.prefix.rects[index].center()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_binding_check(&mut self, phase: u8) {
        let key = "open:advanced-calculator";
        match phase {
            0 => {
                self.page = Page::Commands;
                self.prefix.query = "计算器".into();
            }
            1 => {
                assert_eq!(self.prefix.selected, key);
            }
            2 => {
                assert_eq!(self.prefix.draft.as_ref().unwrap()[key], "Q A");
                assert!(!self.preferences.command_bindings.contains_key(key));
            }
            3 => {
                assert_eq!(self.preferences.command_bindings[key], "Q A");
                assert_eq!(
                    Preferences::load(&self.preferences_path).command_bindings[key],
                    "Q A"
                );
                self.prefix.open();
            }
            4 => {
                assert!(
                    !crate::commands::configured(
                        &self.default_commands(),
                        self.prefix.draft.as_ref().unwrap()
                    )
                    .1
                    .is_empty()
                );
                assert_eq!(self.preferences.command_bindings[key], "Q A");
            }
            5 => {
                assert_eq!(
                    self.prefix.draft.as_ref().unwrap(),
                    &self.preferences.command_bindings
                );
            }
            6 => {
                assert!(self.prefix.draft.as_ref().unwrap().is_empty());
                assert_eq!(self.preferences.command_bindings[key], "Q A");
            }
            7 => {
                assert!(self.preferences.command_bindings.is_empty());
                assert!(
                    Preferences::load(&self.preferences_path)
                        .command_bindings
                        .is_empty()
                );
                println!(
                    "PASS binding editor: native selection, text edit, stage, save/reload, conflict, discard, reset/save"
                );
            }
            8 => {
                assert_eq!(self.page, Page::Calculator);
                self.page = Page::Commands;
                self.prefix.input = "R".into();
            }
            _ => panic!("unknown phase"),
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_prefix_check(&mut self, phase: u8) {
        match phase {
            0 => {
                self.page = Page::Commands;
                self.quick_open = false;
                self.prefix.open();
            }
            1 => {
                assert_eq!(self.prefix.sequence, "R");
                assert_eq!(self.page, Page::Commands);
            }
            2 => {
                assert_eq!(self.page, Page::Recorder);
                self.page = Page::Commands;
                self.prefix.open();
            }
            3 => {
                assert_eq!(self.page, Page::Calculator);
                self.page = Page::Commands;
                self.prefix.open();
            }
            4 => {
                assert_eq!(self.prefix.sequence, "");
                assert!(self.prefix.message.contains("未绑定"));
            }
            5 => {
                assert!(!self.prefix.active);
                assert!(self.quick_open);
                println!(
                    "PASS prefix: R O opens recorder, C opens calculator, invalid keys do not execute, K restores tool search"
                );
            }
            6 => {
                assert_eq!(self.page, Page::Commands);
                assert!(self.prefix.sequence.is_empty());
                assert!(!self.prefix.active);
            }
            7 => {
                assert_eq!(self.page, Page::Commands);
                assert!(!self.prefix.active);
            }
            _ => panic!("unknown phase"),
        }
    }
    fn default_commands(&self) -> Vec<Command> {
        let mut result = crate::commands::controls();
        result.extend(
            self.entries("")
                .iter()
                .map(|entry| crate::commands::tool_command(&entry.id, &entry.title)),
        );
        result
    }
    fn command_entries(&self) -> Vec<Command> {
        crate::commands::configured(&self.default_commands(), &self.preferences.command_bindings).0
    }
    pub(super) fn prefix_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<String> {
        let commands = self.command_entries();
        let (_, binding_errors) = crate::commands::configured(
            &self.default_commands(),
            &self.preferences.command_bindings,
        );
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(i.raw.focused));
        if focused {
            self.prefix.had_focus = true;
        } else if self.prefix.had_focus {
            self.prefix.active = false;
            self.prefix.sequence.clear();
            self.quick_open = false;
            return None;
        }
        if ctx.input(|i| i.pointer.any_pressed() || i.raw_scroll_delta != egui::Vec2::ZERO) {
            self.prefix.last = Instant::now();
        }
        if self.prefix.last.elapsed() > Duration::from_secs(8) {
            self.prefix.active = false;
            self.quick_open = false;
            return None;
        }
        ctx.request_repaint_after(Duration::from_millis(150));
        let mut action = None;
        // Only the focused shortcut viewport receives sequences; ordinary typing is
        // never hooked globally. IME composition aborts the pending sequence.
        let events = ctx.input(|i| {
            if i.viewport().focused.unwrap_or(i.raw.focused) {
                i.events.clone()
            } else {
                Vec::new()
            }
        });
        if events.iter().any(|event|matches!(event,egui::Event::Ime(value) if !matches!(value,egui::ImeEvent::Disabled))) {
            self.prefix.sequence.clear(); self.prefix.active=false; self.prefix.message="输入法组合中不执行快捷序列".into(); self.quick_focus=true; return None;
        }
        for event in events {
            match event {
                egui::Event::Ime(egui::ImeEvent::Disabled) => {}
                egui::Event::Ime(_) => {
                    self.prefix.sequence.clear();
                    self.prefix.message = "输入法组合中不执行快捷序列；点击搜索工具".into();
                    self.prefix.active = false;
                    self.quick_focus = true;
                    return None;
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if !modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.command
                    && !modifiers.shift =>
                {
                    if matches!(key, egui::Key::Escape) {
                        self.prefix.active = false;
                        self.prefix.sequence.clear();
                        self.quick_open = false;
                        return None;
                    }
                    if key == egui::Key::Backspace {
                        self.prefix.sequence.clear();
                        self.prefix.last = Instant::now();
                        continue;
                    }
                    let key = format!("{key:?}");
                    if key.len() != 1 || !key.as_bytes()[0].is_ascii_alphabetic() {
                        continue;
                    }
                    if !self.prefix.sequence.is_empty() {
                        self.prefix.sequence.push(' ');
                    }
                    self.prefix.sequence.push_str(&key);
                    self.prefix.last = Instant::now();
                    match crate::commands::resolve(&commands, &self.prefix.sequence) {
                        Match::Run(index) => {
                            action = Some(commands[index].action.clone());
                            break;
                        }
                        Match::Pending => self.prefix.message.clear(),
                        Match::Invalid => {
                            self.prefix.message =
                                format!("{} 未绑定；按 K 搜索全部工具", self.prefix.sequence);
                            self.prefix.sequence.clear();
                        }
                    }
                }
                _ => {}
            }
        }
        ui.heading("快捷指令");
        if !binding_errors.is_empty() {
            ui.colored_label(
                self.colors.amber,
                "部分绑定冲突或无效，已停用相关序列；请在统一快捷指令中修正。K 搜索仍可用。",
            );
        }
        ui.small(format!(
            "{} → 松开修饰键 → 按下面的键；8 秒超时 / Esc 取消",
            self.preferences.hotkey.shortcut
        ));
        ui.label(if self.prefix.sequence.is_empty() {
            "等待附加键…".into()
        } else {
            format!("{} …", self.prefix.sequence)
        });
        if !self.prefix.message.is_empty() {
            ui.colored_label(self.colors.amber, &self.prefix.message);
        }
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - 30.0).max(60.0))
            .show(ui, |ui| {
                for command in commands.iter().filter(|c| {
                    !c.sequence.is_empty()
                        && (self.prefix.sequence.is_empty()
                            || c.sequence
                                .starts_with(&format!("{} ", self.prefix.sequence)))
                }) {
                    if ui
                        .add_sized(
                            [ui.available_width(), 30.0],
                            egui::Button::new(format!("{}    {}", command.sequence, command.title)),
                        )
                        .on_hover_text(&command.id)
                        .clicked()
                    {
                        action = Some(command.action.clone());
                    }
                }
            });
        ui.small("K 搜索覆盖所有内置和已启用插件工具；自定义绑定在统一快捷指令中编辑。");
        match action {
            Some(Action::Open(id)) => {
                self.prefix.active = false;
                Some(id)
            }
            Some(Action::Search) => {
                self.open_quick(ctx);
                None
            }
            Some(Action::RecordStart) => {
                self.prefix.active = false;
                self.quick_open = false;
                self.page = Page::Recorder;
                if !self.recorder.request_start(self.tray.is_some()) {
                    restore_main_window(self.window_handle, ctx);
                }
                None
            }
            Some(Action::RecordPause) => {
                self.recorder.toggle_pause();
                self.prefix.active = false;
                self.quick_open = false;
                None
            }
            Some(Action::RecordStop) => {
                self.recorder.request_stop();
                self.prefix.active = false;
                self.quick_open = false;
                None
            }
            None => None,
        }
    }
}
