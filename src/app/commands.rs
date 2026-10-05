use super::*;
use crate::commands::{Action, Command, Match};

pub(super) struct Prefix {
    pub active: bool,
    sequence: String,
    message: String,
    last: Instant,
    had_focus: bool,
}
impl Default for Prefix {
    fn default() -> Self {
        Self {
            active: false,
            sequence: String::new(),
            message: String::new(),
            last: Instant::now(),
            had_focus: false,
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
            "v{version} · 开发中：共用命令身份、全局前缀与默认键序列"
        ));
        ui.small(
            "全局前缀在设置中修改；旧录屏直接热键保留。自定义附加键、导入导出与更多工具动作待续。",
        );
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
        ui.label("常用键序列；按 K 搜索全部工具，包括已启用插件。");
        let mut commands = self.command_entries();
        commands.retain(|command| !command.sequence.is_empty());
        commands.sort_by(|a, b| a.sequence.cmp(&b.sequence));
        for command in commands {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(if command.sequence.is_empty() {
                    "K 搜索"
                } else {
                    &command.sequence
                });
                ui.label(&command.title);
            });
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
    fn command_entries(&self) -> Vec<Command> {
        let mut result = crate::commands::controls();
        result.extend(
            self.entries("")
                .iter()
                .map(|entry| crate::commands::tool_command(&entry.id, &entry.title)),
        );
        result
    }
    pub(super) fn prefix_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<String> {
        let commands = self.command_entries();
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
        ui.small("K 搜索覆盖所有内置和已启用插件工具；逐工具自定义绑定待接入。");
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
