use super::History;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct State {
    history: History,
    policy: super::policy::Editor,
    excluded_count: u64,
    search: String,
    selected: Vec<u64>,
    format: usize,
    message: String,
    output: String,
    clear_confirm: bool,
    paused: bool,
    retention_input: u16,
    retention_confirm: bool,
    last_retention_check: Option<std::time::Instant>,
    #[cfg(windows)]
    listener: Option<super::native::Listener>,
    #[cfg(windows)]
    storage: super::storage::Persistence,
    #[cfg(windows)]
    forget_confirm: bool,
    #[cfg(windows)]
    restore_confirm: bool,
}
impl State {
    pub fn new(policy_path: std::path::PathBuf) -> Self {
        Self {
            policy: super::policy::Editor::new(if cfg!(any(feature = "ui-preview", test)) {
                None
            } else {
                Some(policy_path)
            }),
            ..Default::default()
        }
    }
    fn policy_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("应用排除 · 采集前过滤").default_open(true).show(ui, |ui| {
            ui.label(format!("已应用{}个进程规则 · 已跳过{}次通知 · 未知来源{}", self.policy.applied.excluded_apps.len(), self.excluded_count, if self.policy.applied.exclude_unknown { "不采集" } else { "允许采集" }));
            ui.collapsing("查看当前生效的进程名单", |ui| {
                if self.policy.applied.excluded_apps.is_empty() { ui.label("当前不排除已知进程"); }
                else { egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| { for name in &self.policy.applied.excluded_apps { ui.label(name); } }); }
            });
            ui.small("按实际剪贴板所有者的进程文件名过滤，在读取文本正文前判断；不能保证识别所有密码或间接复制来源。每行一个.exe文件名，大小写不敏感，不填路径或通配符，最多64个。规则与历史保存独立。");
            ui.add(egui::TextEdit::multiline(&mut self.policy.input).desired_rows(2).desired_width(f32::INFINITY).hint_text("example.exe\neditor.exe"));
            ui.checkbox(&mut self.policy.unknown, "来源未知时不采集");
            if self.policy.dirty() { ui.label("规则草稿未应用，当前仍使用上方已应用规则"); }
            if self.policy.load_failed {
                ui.label("旧规则读取失败；首次处理前不能开启采集，仅本次应用不会修改旧文件。");
                ui.checkbox(&mut self.policy.replace_confirm, "允许保存时替换无法读取的旧规则文件");
            }
            ui.horizontal_wrapped(|ui| {
                for (remember,label) in [(false,"仅本次应用"),(true,"保存并应用规则")] {
                    if ui.add_enabled(!remember || self.policy.path.is_some(),egui::Button::new(label)).clicked() {
                        match self.policy.apply(remember) {
                            Ok(()) => {
                                #[cfg(windows)]
                                if let Some(listener)=&self.listener { listener.set_policy(self.policy.applied.clone()); }
                                self.message="已应用排除规则；旧排队事件清除，不补采当前内容".into();
                            }
                            Err(error) => self.policy.status=error,
                        }
                    }
                }
                if ui.button("撤回草稿").clicked() { self.policy.input=self.policy.applied.excluded_apps.join("\n");self.policy.unknown=self.policy.applied.exclude_unknown; }
                if ui.add_enabled(self.listener_is_off() && !self.policy.dirty(),egui::Button::new("重新读取保存规则")).clicked() { self.policy.reload(); }
            });
            ui.small(&self.policy.status);
            if let Some(path)=&self.policy.path { ui.small(format!("规则文件：{}",path.display())); }
        });
    }
    fn listener_is_off(&self) -> bool {
        #[cfg(windows)]
        {
            self.listener.is_none()
        }
        #[cfg(not(windows))]
        {
            true
        }
    }

    #[cfg(windows)]
    fn storage_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(format!("本机历史保存 · {}", if self.storage.enabled { "已开启" } else { "未开启" })).id_salt("clipboard-storage-options").default_open(false).show(ui,|ui|{
            ui.label(if self.storage.enabled {"已开启：后台保存；重启恢复历史，采集需另行开启。"} else {"未开启：历史只在会话内存中，退出后清空。"});
            ui.small("保存包含文本、来源、时间和置顶状态，保护范围为当前Windows用户；同用户程序仍可能访问。不开启云同步。");
            if let Some(path)=&self.storage.path {ui.small(format!("本机路径：{}",path.display()));}
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!self.storage.busy(),egui::Button::new(if self.storage.enabled {"立即保存 / 重试"} else {"开启本机保存与重启恢复"})).clicked(){self.storage.save_now(ui.ctx(),&self.history);}
                if ui.add_enabled(!self.storage.busy(),egui::Button::new("停止保存并删除本机历史…")).clicked(){self.forget_confirm=true;}
                if self.storage.busy(){ui.spinner();ui.label("本机读写中，退出前请等待");}
                else if self.storage.pending(){ui.label("尚有未保存的历史变化");}
            });
            if self.storage.load_failed && ui.add_enabled(!self.storage.busy(),egui::Button::new("重试读取此前历史")).clicked(){
                if self.history.entries.is_empty(){self.start_reload(ui.ctx());}else{self.restore_confirm=true;}
            }
            if self.restore_confirm {
                ui.label(format!("读取旧历史将替换当前{}条会话历史，并关闭采集。确认继续？",self.history.entries.len()));
                ui.horizontal(|ui|{if ui.button("确认读取并替换").clicked(){self.start_reload(ui.ctx());self.restore_confirm=false;}
                if ui.button("取消读取").clicked(){self.restore_confirm=false;}});
            }
            if !self.storage.error.is_empty(){ui.label(&self.storage.error);}
            if self.forget_confirm {
                ui.label("停止后会话历史仍可使用；删除此前保存的本机快照，重启不恢复。确认删除？");
                ui.horizontal(|ui|{
                    if ui.add_enabled(!self.storage.busy(),egui::Button::new("确认停止并删除")).clicked(){self.storage.forget(ui.ctx());self.forget_confirm=false;}
                    if ui.button("取消").clicked(){self.forget_confirm=false;}
                });
            }
        });
    }
    #[cfg(windows)]
    fn start_reload(&mut self, ctx: &egui::Context) {
        self.listener = None;
        self.selected.clear();
        self.output.clear();
        self.storage.reload(ctx);
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(windows)]
        {
            let before = self.storage.restored_revision;
            self.storage.poll(ctx, &mut self.history);
            if before != self.storage.restored_revision {
                self.selected.clear();
                self.output.clear();
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
        #[cfg(windows)]
        if let Some(listener) = &self.listener {
            for event in listener.rx.try_iter().take(64) {
                match event {
                    super::native::Event::Text(text, source)
                        if !self.paused && !text.is_empty() =>
                    {
                        match self.history.insert(text, source) {
                            Err(e) => self.message = e,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::Error(e) => self.message = e,
                    super::native::Event::Excluded => {
                        self.excluded_count = self.excluded_count.saturating_add(1)
                    }
                    _ => {}
                }
            }
            if listener.lost.swap(0, std::sync::atomic::Ordering::AcqRel) > 0 {
                self.message = "复制事件过快，部分条目未采集".into();
            }
        }
        if self.history.retention_days.is_some()
            && self
                .last_retention_check
                .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(60))
        {
            self.last_retention_check = Some(std::time::Instant::now());
            let removed = self.history.expire(chrono::Utc::now().timestamp());
            if removed > 0 {
                self.changed();
                self.message = format!("按保留期限清理了{removed}条；置顶及时间未知的记录保留");
            }
        }
        let selected_count = self.selected.len();
        self.selected
            .retain(|id| self.history.entries.iter().any(|e| e.id == *id));
        if self.selected.len() != selected_count {
            self.output.clear();
        }
    }
    fn changed(&mut self) {
        #[cfg(windows)]
        self.storage.changed();
    }
    pub fn saving(&self) -> bool {
        #[cfg(windows)]
        {
            self.storage.busy()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    fn retention_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.history.retention_days.map_or_else(|| "历史保留期限 · 未开启".to_string(), |days| format!("历史保留期限 · {days}天"))).id_salt("clipboard-retention-options").default_open(false).show(ui, |ui| {
            ui.label(self.history.retention_days.map_or_else(|| "自动清理未开启".to_string(), |days| format!("自动清理：最近复制满{days}天的非置顶记录")));
            ui.small("按24小时计算，最多约一分钟检查一次；置顶、UTC时间未知和未来时间记录保留。清理只删除历史，不改系统剪贴板。开启本机保存时同步保存策略和清理结果。旧快照不推断年份。");
            if self.retention_input == 0 { self.retention_input = self.history.retention_days.unwrap_or(30); }
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut self.retention_input).range(1..=3650).suffix(" 天"));
                let count = self.history.expired_count(self.retention_input, chrono::Utc::now().timestamp());
                ui.label(format!("当前将清理{count}条"));
                if ui.button("应用期限并清理…").clicked() { self.retention_confirm = true; }
                if self.history.retention_days.is_some() && ui.button("关闭自动清理").clicked() {
                    self.history.retention_days = None;
                    self.retention_confirm = false;
                    self.changed();
                }
            });
            if self.retention_confirm {
                let count = self.history.expired_count(self.retention_input, chrono::Utc::now().timestamp());
                ui.label(format!("确认开启{}天期限并删除当前{count}条历史？后续过期记录会自动删除，不能撤销。", self.retention_input));
                ui.horizontal(|ui| {
                    if ui.button("确认应用与清理").clicked() {
                        self.history.retention_days = Some(self.retention_input);
                        let count = self.history.expire(chrono::Utc::now().timestamp());
                        self.last_retention_check = Some(std::time::Instant::now());
                        self.selected.retain(|id| self.history.entries.iter().any(|entry| entry.id == *id));
                        self.output.clear();
                        self.changed();
                        self.retention_confirm = false;
                        self.message = format!("已应用期限，清理{count}条历史");
                    }
                    if ui.button("取消").clicked() { self.retention_confirm = false; }
                });
            }
        });
    }
    pub fn needs_clock(&self) -> bool {
        #[cfg(windows)]
        {
            self.storage.needs_clock() || self.history.retention_days.is_some()
        }
        #[cfg(not(windows))]
        {
            self.history.retention_days.is_some()
        }
    }
    pub fn has_pending(&self) -> bool {
        #[cfg(windows)]
        {
            self.storage.pending() || self.policy.dirty()
        }
        #[cfg(not(windows))]
        {
            self.policy.dirty()
        }
    }
    fn copy(&mut self, ui: &egui::Ui, text: String) {
        #[cfg(windows)]
        if let Some(listener) = &self.listener {
            self.message = match listener.copy(&text) {
                Ok(()) => "已复制；不会重复采集本工具输出".into(),
                Err(e) => e,
            };
            return;
        }
        ui.ctx().copy_text(text);
        self.message = "已交给系统剪贴板".into();
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(windows)]
        if self.storage.restoring() {
            ui.heading("正在恢复本机保护历史");
            ui.spinner();
            ui.label("完成前暂不修改历史或开启采集。");
            return;
        }
        ui.heading("超级剪贴板");
        ui.label("v0.4.0 · 开发中：文本历史 / 搜索与置顶 / 按选择顺序组合复制");
        ui.label("主动开启后采集新复制的文本；可另行开启本机保护保存，重启恢复历史但不自动采集。图片、富文本、文件引用尚待开发。");
        #[cfg(windows)]
        ui.horizontal_wrapped(|ui| {
            if self.listener.is_none() {
                if ui
                    .add_enabled(
                        !self.storage.busy() && self.policy.ready,
                        egui::Button::new("开启文本历史采集"),
                    )
                    .clicked()
                {
                    match super::native::Listener::start(
                        ui.ctx().clone(),
                        self.policy.applied.clone(),
                    ) {
                        Ok(listener) => {
                            self.listener = Some(listener);
                            self.paused = false;
                            self.message = "已开启；仅采集后续复制的文本".into();
                        }
                        Err(e) => self.message = e,
                    }
                }
            } else {
                if ui
                    .button(if self.paused {
                        "恢复采集"
                    } else {
                        "私密暂停"
                    })
                    .clicked()
                {
                    self.paused = !self.paused;
                    if let Some(listener) = &self.listener {
                        listener.set_paused(self.paused);
                    }
                    self.message = if self.paused {
                        "已暂停；现有历史仍可使用"
                    } else {
                        "已恢复；不补采暂停期间内容"
                    }
                    .into();
                }
                if ui.button("关闭采集").clicked() {
                    self.listener = None;
                    self.message = "已关闭采集；历史保存状态见下方".into();
                }
            }
            ui.label(format!(
                "{} 条 · {:.2} MiB / 32 MiB",
                self.history.entries.len(),
                self.history.bytes() as f32 / (1024.0 * 1024.0)
            ));
            if ui.button("清空历史…").clicked() {
                self.clear_confirm = true;
            }
        });
        #[cfg(not(windows))]
        ui.label("剪贴板采集当前仅支持Windows。");
        if self.clear_confirm {
            ui.horizontal_wrapped(|ui| {
                ui.label("删除全部会话历史（含置顶）？");
                if ui.button("确认清空").clicked() {
                    self.history.entries.clear();
                    self.selected.clear();
                    self.changed();
                    self.clear_confirm = false;
                }
                if ui.button("取消").clicked() {
                    self.clear_confirm = false;
                }
            });
        }
        #[cfg(windows)]
        self.storage_ui(ui);
        self.policy_ui(ui);
        self.retention_ui(ui);
        ui.small("当前最多500条、单条1 MiB。来源可能未知；应用排除见上方；不能保证识别敏感内容，必要时请暂停。默认不采集、不联网；本机保存须主动开启。");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("搜索文本或来源应用"));
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.separator();
        let query = self.search.to_lowercase();
        let mut pin_changed = false;
        let mut copy = None;
        let mut delete = None;
        egui::ScrollArea::vertical()
            .id_salt("clipboard-history-list")
            .max_height(340.0)
            .show(ui, |ui| {
                if self.history.entries.is_empty() {
                    ui.label("开启采集后，在任意应用复制文本；也可先试用合成示例。");
                }
                let mut indices: Vec<usize> = (0..self.history.entries.len()).collect();
                indices.sort_by_key(|i| !self.history.entries[*i].pinned);
                for i in indices {
                    let entry = &mut self.history.entries[i];
                    if !query.is_empty()
                        && !entry.text.to_lowercase().contains(&query)
                        && !entry.source.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut selected = self.selected.contains(&entry.id);
                            if ui.checkbox(&mut selected, "选择").changed() {
                                if selected {
                                    self.selected.push(entry.id);
                                } else {
                                    self.selected.retain(|id| *id != entry.id);
                                }
                            }
                            if ui.checkbox(&mut entry.pinned, "置顶保留").on_hover_text("置顶不自动过期；取消置顶后，已过期记录将在下一次期限检查时删除。").changed() {
                                pin_changed = true;
                            }
                            ui.small(format!("{} · {}", entry.source, entry.time))
                                .on_hover_text(format!(
                                    "首次UTC：{}\n最近UTC：{}",
                                    timestamp_label(entry.first_captured_utc),
                                    timestamp_label(entry.last_captured_utc)
                                ));
                            if entry.last_captured_utc.is_none() {
                                ui.small("旧记录 · 年份未知");
                            }
                            if ui.button("复制").clicked() {
                                copy = Some(entry.text.clone());
                            }
                            if ui.button("删除").clicked() {
                                delete = Some(entry.id);
                            }
                        });
                        let preview: String = entry.text.chars().take(180).collect();
                        ui.label(preview);
                        if entry.text.chars().count() > 180 {
                            ui.small("长文本已截断；下方组合预览可查看完整结果");
                        }
                    });
                }
            });
        if pin_changed {
            self.changed();
        }
        if let Some(id) = delete {
            self.changed();
            self.history.entries.retain(|e| e.id != id);
            self.selected.retain(|x| *x != id);
        }
        if let Some(text) = copy {
            self.copy(ui, text);
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("组合预览 · {}项", self.selected.len())).strong());
            egui::ComboBox::from_id_salt("clipboard-format")
                .selected_text(["段落", "编号", "JSON数组", "CSV单列"][self.format])
                .show_ui(ui, |ui| {
                    for (i, label) in ["段落", "编号", "JSON数组", "CSV单列"].iter().enumerate()
                    {
                        ui.selectable_value(&mut self.format, i, *label);
                    }
                });
            if ui.button("取消全部选择").clicked() {
                self.selected.clear();
            }
        });
        match self.history.combined(&self.selected, self.format) {
            Ok(mut text) => {
                self.output.clone_from(&text);
                ui.small("顺序按勾选先后；取消后重选可改变次序。CSV对可能的公式加前置单引号；历史原文不变。可通过上方接力入口发送备忘/数据等工具。");
                if ui.button("复制组合结果").clicked() {
                    self.copy(ui, text.clone());
                }
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            }
            Err(e) => {
                self.output.clear();
                ui.small(e);
            }
        }
        if ui.button("载入合成示例（不读取系统剪贴板）").clicked() {
            self.preview_fixture();
        }
    }
    pub fn transfer_text(&self) -> Option<(String, &str)> {
        if self.output.is_empty() {
            None
        } else {
            Some((
                format!("超级剪贴板0.4.0组合 · 条目{:?}", self.selected),
                &self.output,
            ))
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_policy_fixture(&mut self) {
        self.preview_fixture();
        self.policy.input = "synthetic-private.exe\nfixture-editor.exe".into();
        self.policy.unknown = true;
        self.policy.apply(false).unwrap();
        self.policy.input.push_str("\nnew-draft.exe");
        self.excluded_count = 12;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_retention_fixture(&mut self) {
        self.preview_fixture();
        self.history.retention_days = Some(30);
        self.retention_input = 7;
        let now = chrono::Utc::now().timestamp();
        for entry in &mut self.history.entries {
            entry.first_captured_utc = None;
            entry.time = if entry.pinned {
                "01-02 03:04:05".into()
            } else {
                chrono::DateTime::from_timestamp(now - 10 * 86400, 0)
                    .unwrap()
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            };
            entry.last_captured_utc = if entry.pinned {
                None
            } else {
                Some(now - 10 * 86400)
            };
        }
        self.retention_confirm = true;
        self.message = "合成示例：修改期限前预览，不读取或写入用户数据。".into();
    }
    #[cfg(all(windows, feature = "ui-preview"))]
    pub fn preview_storage_fixture(&mut self) {
        self.preview_fixture();
        self.storage.preview_enabled();
        self.message = "合成示例：本机保存已开启的界面；不读取或写入真实数据。".into();
    }
    pub fn preview_fixture(&mut self) {
        self.changed();
        self.selected.clear();
        self.output.clear();
        for (text, source) in [
            ("教程第一步：准备示例数据\n仅使用合成内容", "示例编辑器"),
            (
                "{\"project\":\"Zi DevTools\",\"status\":\"demo\"}",
                "示例终端",
            ),
            (
                "第二步：选择JSON数组组合，多片段可以保持原来的复制顺序。",
                "示例浏览器",
            ),
        ] {
            if let Err(e) = self.history.insert(text.into(), source.into()) {
                self.message = e;
                return;
            }
            self.selected.push(self.history.entries[0].id);
        }
        if let Some(entry) = self
            .history
            .entries
            .iter_mut()
            .find(|e| e.source == "示例编辑器")
        {
            entry.pinned = true;
        }
        self.format = 2;
        self.output = self
            .history
            .combined(&self.selected, self.format)
            .unwrap_or_default();
        self.message = "合成示例不读取系统剪贴板；采集状态见上方".into();
    }
}

fn timestamp_label(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
        .map_or_else(|| "未知".into(), |time| time.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_drafts_guard_exit_without_enabling_history_storage_or_capture() {
        let mut state = State::default();
        assert!(!state.has_pending());
        state.policy.input = "fixture.exe".into();
        assert!(state.has_pending());
        state.policy.apply(false).unwrap();
        assert!(!state.has_pending() && state.policy.ready);
        assert!(state.listener_is_off());
        #[cfg(windows)]
        assert!(!state.storage.enabled && state.storage.path.is_none());
        state.policy.unknown = true;
        assert!(state.has_pending());
        state.policy.input = state.policy.applied.excluded_apps.join("\n");
        state.policy.unknown = state.policy.applied.exclude_unknown;
        assert!(!state.has_pending());
    }
    #[test]
    fn retention_poll_expires_and_invalidates_selected_handoff_once() {
        let mut state = State::default();
        state.preview_fixture();
        let now = chrono::Utc::now().timestamp();
        for entry in &mut state.history.entries {
            entry.first_captured_utc = None;
            entry.last_captured_utc = Some(now - 2 * 86400);
        }
        state.history.retention_days = Some(1);
        assert!(state.needs_clock());
        state.poll(&egui::Context::default());
        assert_eq!(state.history.entries.len(), 1);
        assert!(state.history.entries[0].pinned);
        assert!(state.transfer_text().is_none());
        assert_eq!(state.selected, vec![state.history.entries[0].id]);
        let message = state.message.clone();
        state.poll(&egui::Context::default());
        assert_eq!(state.message, message);
    }
    #[test]
    fn demo_rejects_full_pinned_history_without_panicking_or_replacing_originals() {
        let mut state = State::default();
        for i in 0..super::super::ITEM_LIMIT {
            state
                .history
                .insert(format!("fixture-{i}"), "fixture".into())
                .unwrap();
            state.history.entries[0].pinned = true;
        }
        state.output = "stale".into();
        state.preview_fixture();
        assert_eq!(state.history.entries.len(), super::super::ITEM_LIMIT);
        assert!(
            state
                .history
                .entries
                .iter()
                .all(|e| e.text.starts_with("fixture-"))
        );
        assert!(state.transfer_text().is_none());
        assert!(!state.message.is_empty());
    }
    #[test]
    fn evicted_selection_invalidates_handoff_without_replacing_other_inputs() {
        let mut state = State::default();
        state.preview_fixture();
        assert!(state.transfer_text().is_some());
        let remove = state.selected[0];
        state.history.entries.retain(|e| e.id != remove);
        state.poll(&egui::Context::default());
        assert!(state.transfer_text().is_none());
        assert!(!state.selected.contains(&remove));
    }
}
