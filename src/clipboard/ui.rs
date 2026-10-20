use super::{History, OcrRegion};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct State {
    history: History,
    images_enabled: bool,
    files_enabled: bool,
    html_enabled: bool,
    rtf_enabled: bool,
    image_preview: Option<Preview>,
    ocr_region_selecting: bool,
    handoff_ocr: bool,
    image_relay: Option<std::sync::Arc<super::Picture>>,
    file_relay: Option<Vec<std::path::PathBuf>>,
    file_relay_wait_notified: bool,
    policy: super::policy::Editor,
    excluded_count: u64,
    search: String,
    grouped_by_label: bool,
    editing_label: Option<u64>,
    label_draft: String,
    batch_label_draft: String,
    selected: Vec<u64>,
    selected_files: Vec<u64>,
    format: usize,
    template: Option<String>,
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
    #[cfg(windows)]
    ocr_languages: Vec<super::ocr::RecognizerLanguage>,
    #[cfg(windows)]
    ocr_languages_job: Option<OcrLanguagesJob>,
    #[cfg(windows)]
    ocr_languages_error: String,
    #[cfg(windows)]
    ocr_language_tag: Option<String>,
    #[cfg(windows)]
    ocr_task: crate::tasks::Job,
    #[cfg(test)]
    test_label_filter_rects: Vec<(String, egui::Rect)>,
    #[cfg(test)]
    test_clear_search_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_add_filtered_texts_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_add_filtered_texts_enabled: Option<bool>,
    #[cfg(test)]
    test_batch_label_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_format_combo_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_markdown_format_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_template_format_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_records_format_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_template_editor_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_group_mode_rect: Option<egui::Rect>,
    #[cfg(test)]
    test_group_headers: Vec<String>,
    #[cfg(test)]
    test_group_header_rects: Vec<(String, egui::Rect)>,
    #[cfg(test)]
    test_file_relay_rect: Option<egui::Rect>,
    #[cfg(all(test, windows))]
    test_ocr_image_rect: Option<egui::Rect>,
    #[cfg(all(test, windows))]
    test_ocr_region_toggle_rect: Option<egui::Rect>,
    #[cfg(all(test, windows))]
    test_ocr_language_rect: Option<egui::Rect>,
    #[cfg(all(test, windows))]
    test_ocr_language_option_rects: Vec<(String, egui::Rect)>,
}
type PreviewReceipt = Result<(egui::ColorImage, Vec<u8>), String>;
type OcrReceipt = Result<String, String>;
#[cfg(windows)]
type OcrLanguagesReceipt = Result<Vec<super::ocr::RecognizerLanguage>, String>;
#[cfg(windows)]
type OcrLanguagesJob = (u64, std::sync::mpsc::Receiver<OcrLanguagesReceipt>);
struct Preview {
    cancelled: bool,
    id: u64,
    picture: std::sync::Arc<super::Picture>,
    texture: Option<egui::TextureHandle>,
    dib: Option<Vec<u8>>,
    error: String,
    job: Option<std::sync::mpsc::Receiver<PreviewReceipt>>,
    ocr_job: Option<std::sync::mpsc::Receiver<OcrReceipt>>,
    ocr_text: Option<String>,
    ocr_error: String,
    ocr_region: Option<OcrRegion>,
    ocr_drag_start: Option<egui::Pos2>,
}
fn ocr_region_from_drag(
    image_rect: egui::Rect,
    image_size: [u32; 2],
    start: egui::Pos2,
    end: egui::Pos2,
) -> Option<OcrRegion> {
    if image_rect.width() <= 0.0
        || image_rect.height() <= 0.0
        || image_size[0] == 0
        || image_size[1] == 0
    {
        return None;
    }
    let selection = egui::Rect::from_two_pos(start, end).intersect(image_rect);
    if selection.width() <= 0.0 || selection.height() <= 0.0 {
        return None;
    }
    let x = |position: f32| {
        (((position - image_rect.left()) / image_rect.width()) * image_size[0] as f32)
            .floor()
            .clamp(0.0, image_size[0] as f32) as u32
    };
    let y = |position: f32| {
        (((position - image_rect.top()) / image_rect.height()) * image_size[1] as f32)
            .floor()
            .clamp(0.0, image_size[1] as f32) as u32
    };
    let right = |position: f32| {
        (((position - image_rect.left()) / image_rect.width()) * image_size[0] as f32)
            .ceil()
            .clamp(0.0, image_size[0] as f32) as u32
    };
    let bottom = |position: f32| {
        (((position - image_rect.top()) / image_rect.height()) * image_size[1] as f32)
            .ceil()
            .clamp(0.0, image_size[1] as f32) as u32
    };
    let left = x(selection.left());
    let top = y(selection.top());
    let width = right(selection.right()).saturating_sub(left);
    let height = bottom(selection.bottom()).saturating_sub(top);
    (width >= 2 && height >= 2).then_some(OcrRegion {
        x: left,
        y: top,
        width,
        height,
    })
}
fn screen_rect_for_ocr_region(
    image_rect: egui::Rect,
    image_size: [u32; 2],
    region: OcrRegion,
) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            image_rect.left() + image_rect.width() * region.x as f32 / image_size[0] as f32,
            image_rect.top() + image_rect.height() * region.y as f32 / image_size[1] as f32,
        ),
        egui::pos2(
            image_rect.left()
                + image_rect.width() * (region.x + region.width) as f32 / image_size[0] as f32,
            image_rect.top()
                + image_rect.height() * (region.y + region.height) as f32 / image_size[1] as f32,
        ),
    )
}
fn matches_query(entry: &super::Entry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || entry.text.to_lowercase().contains(&query)
        || entry
            .rich_html
            .as_deref()
            .is_some_and(|html| contains_lowercase_match(html, &query))
        || entry.source.to_lowercase().contains(&query)
        || entry
            .labels
            .iter()
            .any(|label| label.to_lowercase().contains(&query))
        || entry
            .file_references
            .iter()
            .any(|path| path.to_lowercase().contains(&query))
}

fn contains_lowercase_match(haystack: &str, lowercase_needle: &str) -> bool {
    let needle = lowercase_needle.chars().collect::<Vec<_>>();
    if needle.is_empty() {
        return true;
    }
    let mut prefix = vec![0usize; needle.len()];
    let mut matched = 0usize;
    for index in 1..needle.len() {
        while matched > 0 && needle[index] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
            prefix[index] = matched;
        }
    }
    matched = 0;
    for character in haystack.chars().flat_map(char::to_lowercase) {
        while matched > 0 && character != needle[matched] {
            matched = prefix[matched - 1];
        }
        if character == needle[matched] {
            matched += 1;
            if matched == needle.len() {
                return true;
            }
        }
    }
    false
}

fn add_filtered_text_entries(
    entries: &[super::Entry],
    ordered_indices: &[usize],
    selected: &mut Vec<u64>,
    query: &str,
) -> usize {
    let mut added = 0;
    for &index in ordered_indices {
        let Some(entry) = entries.get(index) else {
            continue;
        };
        if entry.image.is_some()
            || !entry.file_references.is_empty()
            || entry.text.is_empty()
            || !matches_query(entry, query)
        {
            continue;
        }
        if !selected.contains(&entry.id) {
            selected.push(entry.id);
            added += 1;
        }
    }
    added
}

#[derive(Debug, PartialEq, Eq)]
struct LabelGroup {
    label: Option<String>,
    indices: Vec<usize>,
}

#[derive(Default)]
struct EntryActions {
    pin_changed: bool,
    copy: Option<String>,
    copy_html: Option<(String, String)>,
    copy_rtf: Option<(String, Vec<u8>)>,
    copy_files: Option<Vec<String>>,
    delete: Option<u64>,
    open_image: Option<u64>,
    open_label_editor: Option<u64>,
    add_label: Option<(u64, String)>,
    remove_label: Option<(u64, String)>,
    filter_label: Option<String>,
}

fn group_history_by_label(
    entries: &[super::Entry],
    ordered_indices: &[usize],
    query: &str,
) -> Vec<LabelGroup> {
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    let mut unlabelled = Vec::new();
    for &index in ordered_indices {
        let entry = &entries[index];
        if !matches_query(entry, query) {
            continue;
        }
        if entry.labels.is_empty() {
            unlabelled.push(index);
            continue;
        }
        let mut added_to_entry = Vec::new();
        for label in &entry.labels {
            let key = label.to_lowercase();
            if added_to_entry.contains(&key) {
                continue;
            }
            added_to_entry.push(key.clone());
            if let Some((_, _, members)) = groups
                .iter_mut()
                .find(|(_, group_key, _)| *group_key == key)
            {
                members.push(index);
            } else {
                groups.push((label.clone(), key, vec![index]));
            }
        }
    }
    groups.sort_by(|left, right| left.1.cmp(&right.1));
    let mut result = groups
        .into_iter()
        .map(|(label, _, indices)| LabelGroup {
            label: Some(label),
            indices,
        })
        .collect::<Vec<_>>();
    if !unlabelled.is_empty() {
        result.push(LabelGroup {
            label: None,
            indices: unlabelled,
        });
    }
    result
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
    #[cfg(windows)]
    pub(crate) fn task_snapshot(&self) -> Option<crate::tasks::Row> {
        self.ocr_task
            .snapshot("super-clipboard", "剪贴板图片 OCR", false)
    }
    fn policy_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("应用排除 · 采集前过滤").default_open(false).show(ui, |ui| {
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
            ui.small("保存包含文本、图片、标签、来源、时间和置顶状态，保护范围为当前Windows用户；同用户程序仍可能访问。不开启云同步。");
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
        self.selected_files.clear();
        self.output.clear();
        self.cancel_image_preview();
        self.image_relay = None;
        self.storage.reload(ctx);
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.poll_image(ctx);
        #[cfg(windows)]
        {
            let before = self.storage.restored_revision;
            self.storage.poll(ctx, &mut self.history);
            if before != self.storage.restored_revision {
                self.cancel_image_preview();
                self.image_relay = None;
                self.selected.clear();
                self.editing_label = None;
                self.label_draft.clear();
                self.output.clear();
            }
        }
        #[cfg(not(windows))]
        let _ = ctx;
        #[cfg(windows)]
        if let Some(listener) = &self.listener {
            for event in listener.rx.try_iter().take(64) {
                match event.event {
                    super::native::Event::Text(text, source)
                        if !self.paused && !text.is_empty() =>
                    {
                        match self.history.insert(text, source) {
                            Err(e) => self.message = e,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::Image(image, source)
                        if !self.paused && self.images_enabled =>
                    {
                        match self.history.insert_image(image, source) {
                            Err(error) => self.message = error,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::Files(paths, source)
                        if !self.paused && self.files_enabled =>
                    {
                        match self.history.insert_files(paths, source) {
                            Err(error) => self.message = error,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::RichText(text, html, source)
                        if !self.paused && self.html_enabled =>
                    {
                        match self.history.insert_rich_text(text, html, source) {
                            Err(error) => self.message = error,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::RichRtf(text, rtf, source)
                        if !self.paused && self.rtf_enabled =>
                    {
                        match self.history.insert_rich_rtf(text, rtf, source) {
                            Err(error) => self.message = error,
                            Ok(()) => self.storage.changed(),
                        }
                    }
                    super::native::Event::RichFormats(text, html, rtf, source)
                        if !self.paused && self.html_enabled && self.rtf_enabled =>
                    {
                        match self.history.insert_rich_formats(text, html, rtf, source) {
                            Err(error) => self.message = error,
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
                self.message = "通知队列达到事件或字节上限，部分条目未采集".into();
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
        self.selected_files.retain(|id| {
            self.history
                .entries
                .iter()
                .any(|entry| entry.id == *id && !entry.file_references.is_empty())
        });
    }
    fn cancel_image_preview(&mut self) {
        self.handoff_ocr = false;
        self.ocr_region_selecting = false;
        if self
            .image_preview
            .as_ref()
            .is_some_and(|p| p.job.is_some() || p.ocr_job.is_some())
        {
            self.image_preview.as_mut().unwrap().cancelled = true;
        } else {
            self.image_preview = None;
        }
    }
    fn set_ocr_region(&mut self, region: Option<OcrRegion>) {
        let Some(preview) = self.image_preview.as_mut() else {
            return;
        };
        if preview.ocr_region == region {
            return;
        }
        preview.ocr_region = region;
        preview.ocr_text = None;
        preview.ocr_error.clear();
        self.handoff_ocr = false;
        self.message = if region.is_some() {
            "已选择局部 OCR 区域；请手动开始识别".into()
        } else {
            "已清除局部 OCR 区域；下次将识别整图".into()
        };
    }
    #[cfg(windows)]
    fn load_ocr_languages(&mut self, ctx: &egui::Context, id: u64) {
        self.ocr_languages_error.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(super::ocr::available_languages());
            wake.request_repaint();
        });
        self.ocr_languages_job = Some((id, rx));
    }
    #[cfg(windows)]
    fn poll_ocr_languages(&mut self, ctx: &egui::Context) {
        let Some((id, job)) = self.ocr_languages_job.as_ref() else {
            return;
        };
        let id = *id;
        let result = match job.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                None
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("OCR语言查询后台任务意外结束".into()))
            }
        };
        let Some(result) = result else {
            return;
        };
        self.ocr_languages_job = None;
        if !self
            .image_preview
            .as_ref()
            .is_some_and(|preview| preview.id == id && !preview.cancelled)
        {
            return;
        }
        match result {
            Ok(languages) => {
                self.ocr_languages = languages;
                self.ocr_languages_error.clear();
            }
            Err(error) => self.ocr_languages_error = error,
        }
    }
    #[cfg(windows)]
    fn set_ocr_language(&mut self, language_tag: Option<String>) {
        if self.ocr_language_tag == language_tag {
            return;
        }
        self.ocr_language_tag = language_tag;
        if let Some(preview) = self.image_preview.as_mut() {
            preview.ocr_text = None;
            preview.ocr_error.clear();
        }
        self.handoff_ocr = false;
        self.message = match self.ocr_language_tag.as_deref() {
            Some(tag) => format!("识别语言已切换为 {tag}；请手动开始 OCR"),
            None => "已切换为 Windows 系统首选语言；请手动开始 OCR".into(),
        };
    }
    fn open_image(&mut self, ctx: &egui::Context, id: u64) {
        if self
            .image_preview
            .as_ref()
            .is_some_and(|p| p.job.is_some() || p.ocr_job.is_some())
        {
            self.message = "图片预览或 OCR 正在后台处理，请等待完成或取消".into();
            return;
        }
        let Some(picture) = self
            .history
            .entries
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.image.clone())
        else {
            return;
        };
        self.handoff_ocr = false;
        self.ocr_region_selecting = false;
        #[cfg(windows)]
        {
            self.ocr_languages.clear();
            self.ocr_languages_error.clear();
            self.ocr_language_tag = None;
        }
        let source = picture.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                Ok((source.thumbnail()?, super::media::to_dib(&source)?))
            })();
            let _ = tx.send(result.map_err(|_| "图片预览准备失败，历史保留".into()));
            wake.request_repaint();
        });
        self.image_preview = Some(Preview {
            cancelled: false,
            id,
            picture,
            texture: None,
            dib: None,
            error: String::new(),
            job: Some(rx),
            ocr_job: None,
            ocr_text: None,
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        #[cfg(windows)]
        self.load_ocr_languages(ctx, id);
    }
    fn poll_image(&mut self, ctx: &egui::Context) {
        let Some(preview) = &self.image_preview else {
            return;
        };
        let valid_source = self.history.entries.iter().any(|entry| {
            entry.id == preview.id
                && entry
                    .image
                    .as_ref()
                    .is_some_and(|image| std::sync::Arc::ptr_eq(image, &preview.picture))
        });
        if !valid_source {
            self.cancel_image_preview();
            if self.image_preview.is_none() {
                return;
            }
        }
        #[cfg(windows)]
        self.poll_ocr_languages(ctx);
        if let Some(job) = self
            .image_preview
            .as_ref()
            .and_then(|preview| preview.job.as_ref())
        {
            match job.try_recv() {
                Ok(result) => {
                    let preview = self.image_preview.as_mut().unwrap();
                    preview.job = None;
                    if !preview.cancelled {
                        match result {
                            Ok((image, dib)) => {
                                preview.texture = Some(ctx.load_texture(
                                    "clipboard-image-preview",
                                    image,
                                    egui::TextureOptions::LINEAR,
                                ));
                                preview.dib = Some(dib);
                            }
                            Err(error) => preview.error = error,
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
                Err(_) => {
                    self.image_preview.as_mut().unwrap().job = None;
                    self.image_preview.as_mut().unwrap().error = "图片准备线程结束".into();
                }
            }
        }
        if let Some(job) = self
            .image_preview
            .as_ref()
            .and_then(|preview| preview.ocr_job.as_ref())
        {
            match job.try_recv() {
                Ok(result) => {
                    let preview = self.image_preview.as_mut().unwrap();
                    preview.ocr_job = None;
                    if preview.cancelled {
                        self.ocr_task
                            .finish(crate::tasks::Phase::Cancelled, "预览已关闭；识别结果已丢弃");
                    } else {
                        match result {
                            Ok(text) => {
                                preview.ocr_text = Some(text);
                                preview.ocr_error.clear();
                                self.message = "本机 OCR 已完成；结果仅在当前会话中暂存".into();
                                self.ocr_task.finish(
                                    crate::tasks::Phase::Done,
                                    "识别完成；正文仅保留在剪贴板预览中",
                                );
                            }
                            Err(error) => {
                                preview.ocr_error = error;
                                self.ocr_task.finish(
                                    crate::tasks::Phase::Failed,
                                    "OCR 失败；请打开剪贴板预览查看原因",
                                );
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
                Err(_) => {
                    let preview = self.image_preview.as_mut().unwrap();
                    preview.ocr_job = None;
                    preview.ocr_error = "OCR后台任务意外结束".into();
                    self.ocr_task.finish(
                        crate::tasks::Phase::Failed,
                        "OCR 后台任务意外结束；请打开剪贴板预览查看原因",
                    );
                }
            }
        }
        if self.image_preview.as_ref().is_some_and(|preview| {
            preview.cancelled && preview.job.is_none() && preview.ocr_job.is_none()
        }) {
            self.image_preview = None;
            self.handoff_ocr = false;
        }
    }
    pub fn take_image_relay(&mut self) -> Option<std::sync::Arc<super::Picture>> {
        self.image_relay.take()
    }
    pub fn take_file_relay(&mut self) -> Option<Vec<std::path::PathBuf>> {
        self.file_relay_wait_notified = false;
        self.file_relay.take()
    }
    pub fn notify_file_relay_waiting(&mut self) -> bool {
        if self.file_relay.is_some() && !self.file_relay_wait_notified {
            self.file_relay_wait_notified = true;
            self.message = "文件导入正在进行；路径接力已保留，完成后会打开确认页。".into();
            true
        } else {
            false
        }
    }
    fn image_ui(&mut self, ui: &mut egui::Ui) {
        let Some(preview) = &self.image_preview else {
            return;
        };
        let preview_id = preview.id;
        let preview_picture = preview.picture.clone();
        if preview.cancelled {
            ui.small("上一图片预览已取消，后台释放中");
            return;
        }
        ui.separator();
        ui.heading("图片预览");
        ui.small(format!(
            "{} × {} · PNG {:.1}KiB · ID {}",
            preview.picture.width,
            preview.picture.height,
            preview.picture.png.len() as f64 / 1024.0,
            preview.id
        ));
        if preview.job.is_some() {
            ui.spinner();
            ui.label("后台准备预览和复制格式");
        }
        let mut region_update = None;
        let mut region_invalid = false;
        let mut drag_start_update = None;
        if let Some(texture) = &preview.texture {
            let size = texture.size_vec2();
            let sense = if self.ocr_region_selecting {
                egui::Sense::drag()
            } else {
                egui::Sense::hover()
            };
            let (rect, response) = ui.allocate_exact_size(size, sense);
            #[cfg(all(test, windows))]
            {
                self.test_ocr_image_rect = Some(rect);
            }
            for y in 0..=(size.y as usize / 12) {
                for x in 0..=(size.x as usize / 12) {
                    let cell = egui::Rect::from_min_size(
                        rect.min + egui::vec2(x as f32 * 12.0, y as f32 * 12.0),
                        egui::vec2(12.0, 12.0),
                    )
                    .intersect(rect);
                    ui.painter().rect_filled(
                        cell,
                        0.0,
                        if (x + y) % 2 == 0 {
                            egui::Color32::from_gray(160)
                        } else {
                            egui::Color32::from_gray(200)
                        },
                    );
                }
            }
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            let mut drag_start = preview.ocr_drag_start;
            let mut drag_preview = None;
            if self.ocr_region_selecting {
                if response.drag_started() {
                    drag_start = response
                        .interact_pointer_pos()
                        .filter(|position| rect.contains(*position));
                    drag_start_update = Some(drag_start);
                }
                if response.dragged()
                    && let (Some(start), Some(current)) =
                        (drag_start, response.interact_pointer_pos())
                {
                    drag_preview = Some(egui::Rect::from_two_pos(start, current).intersect(rect));
                }
                if response.drag_stopped() {
                    let end = response
                        .interact_pointer_pos()
                        .or_else(|| ui.ctx().input(|input| input.pointer.interact_pos()));
                    if let (Some(start), Some(end)) = (drag_start, end) {
                        region_update = ocr_region_from_drag(
                            rect,
                            [preview.picture.width, preview.picture.height],
                            start,
                            end,
                        );
                        region_invalid = region_update.is_none();
                    }
                    drag_start_update = Some(None);
                }
            }
            let selected = drag_preview.or_else(|| {
                preview.ocr_region.map(|region| {
                    screen_rect_for_ocr_region(
                        rect,
                        [preview.picture.width, preview.picture.height],
                        region,
                    )
                })
            });
            if let Some(selected) = selected {
                ui.painter().rect_filled(
                    selected,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(30, 130, 240, 56),
                );
                ui.painter().rect_stroke(
                    selected,
                    0.0,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(45, 145, 255)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if !preview.error.is_empty() {
            ui.label(&preview.error);
        }
        let mut close = false;
        let mut copy = false;
        let mut relay = false;
        let mut save = false;
        let mut set_ocr_handoff = false;
        let mut clear_ocr_handoff = false;
        let mut toggle_region_selection = false;
        let mut clear_region = false;
        #[cfg(windows)]
        let mut selected_language = self.ocr_language_tag.clone();
        #[cfg(windows)]
        let mut ocr_language_changed = false;
        #[cfg(windows)]
        let mut refresh_ocr_languages = false;
        ui.horizontal_wrapped(|ui| {
            copy = ui
                .add_enabled(
                    preview.dib.is_some(),
                    egui::Button::new("复制图片（PNG + 位图）"),
                )
                .clicked();
            relay = ui
                .add_enabled(
                    preview.texture.is_some(),
                    egui::Button::new("发送到图片工具…"),
                )
                .clicked();
            save = ui.button("另存透明PNG…").clicked();
            close = ui.button("关闭预览").clicked();
        });
        ui.small("图片单独预览，不参加文本组合；发送后还需预览确认目标，来源历史保留。只缓存当前一张预览。");
        ui.separator();
        ui.heading("本机文字识别");
        #[cfg(windows)]
        ui.horizontal_wrapped(|ui| {
            let response = ui.add_enabled(
                preview.texture.is_some() && preview.ocr_job.is_none(),
                egui::Button::selectable(
                    self.ocr_region_selecting,
                    if self.ocr_region_selecting {
                        "取消框选"
                    } else {
                        "框选局部区域…"
                    },
                ),
            );
            #[cfg(test)]
            {
                self.test_ocr_region_toggle_rect = Some(response.rect);
            }
            toggle_region_selection = response.clicked();
            if let Some(region) = preview.ocr_region {
                ui.small(format!(
                    "局部识别 · x{} y{} · {}×{}",
                    region.x, region.y, region.width, region.height
                ));
                clear_region = ui
                    .add_enabled(preview.ocr_job.is_none(), egui::Button::new("清除区域"))
                    .clicked();
            } else {
                ui.small("未框选时识别整张图片");
            }
        });
        if self.ocr_region_selecting {
            ui.small(
                "请在上方图片内按住鼠标并拖动；松开后会显示识别区域。小于 2×2 像素的选区会被忽略。",
            );
        }
        #[cfg(windows)]
        {
            let language_loading = self
                .ocr_languages_job
                .as_ref()
                .is_some_and(|(id, _)| *id == preview_id);
            let selected_label = selected_language.as_deref().map_or_else(
                || "系统首选语言（自动）".to_owned(),
                |tag| {
                    self.ocr_languages
                        .iter()
                        .find(|language| language.tag.eq_ignore_ascii_case(tag))
                        .map_or_else(
                            || format!("不可用语言 · {tag}"),
                            |language| format!("{} · {}", language.display_name, language.tag),
                        )
                },
            );
            #[cfg(test)]
            let mut language_option_rects = Vec::new();
            ui.horizontal_wrapped(|ui| {
                ui.label("识别语言");
                ui.add_enabled_ui(preview.ocr_job.is_none(), |ui| {
                    let combo =
                        egui::ComboBox::from_id_salt(("clipboard-ocr-language", preview_id))
                            .selected_text(selected_label)
                            .show_ui(ui, |ui| {
                                let system_option = ui.selectable_label(
                                    selected_language.is_none(),
                                    "系统首选语言（自动）",
                                );
                                #[cfg(test)]
                                language_option_rects.push(("system".into(), system_option.rect));
                                if system_option.clicked() && selected_language.is_some() {
                                    selected_language = None;
                                    ocr_language_changed = true;
                                }
                                for language in &self.ocr_languages {
                                    let tag = language.tag.clone();
                                    let option = ui.selectable_label(
                                        selected_language.as_deref().is_some_and(|selected| {
                                            selected.eq_ignore_ascii_case(&tag)
                                        }),
                                        format!("{} · {}", language.display_name, language.tag),
                                    );
                                    #[cfg(test)]
                                    language_option_rects.push((tag.clone(), option.rect));
                                    if option.clicked()
                                        && selected_language.as_deref() != Some(tag.as_str())
                                    {
                                        selected_language = Some(tag);
                                        ocr_language_changed = true;
                                    }
                                }
                            });
                    #[cfg(not(test))]
                    let _ = combo;
                    #[cfg(test)]
                    {
                        self.test_ocr_language_rect = Some(combo.response.rect);
                        self.test_ocr_language_option_rects = language_option_rects;
                    }
                });
                refresh_ocr_languages = ui
                    .add_enabled(!language_loading, egui::Button::new("刷新语言"))
                    .clicked();
                if language_loading {
                    ui.spinner();
                    ui.small("正在读取本机语言包");
                }
            });
            if !self.ocr_languages_error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &self.ocr_languages_error);
            } else if !language_loading && self.ocr_languages.is_empty() {
                ui.small("未检测到可用 OCR 语言包；可继续尝试系统首选语言，或在 Windows 设置中安装语言功能。");
            } else {
                ui.small("列表仅显示本机已安装的 Windows OCR 语言包；语言选择仅在当前会话有效。");
            }
        }
        #[cfg(windows)]
        let recognize = ui
            .add_enabled(
                preview.ocr_job.is_none() && !self.ocr_region_selecting,
                egui::Button::new(if preview.ocr_text.is_some() {
                    "重新识别"
                } else {
                    "识别图片文字"
                }),
            )
            .clicked();
        #[cfg(windows)]
        {
            if preview.ocr_job.is_some() {
                ui.spinner();
                ui.label("正在使用 Windows OCR 识别…");
            }
        }
        #[cfg(not(windows))]
        {
            ui.add_enabled(false, egui::Button::new("识别图片文字（仅Windows）"));
        }
        #[cfg(not(windows))]
        let recognize = false;
        ui.small("默认使用 Windows 系统首选语言，也可选择本机已安装的 OCR 语言；可框选或识别整图。图片不上传，结果不写入历史或磁盘。");
        if !preview.ocr_error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &preview.ocr_error);
        }
        if let Some(text) = &preview.ocr_text {
            ui.label(format!("识别结果 · {} 字符", text.chars().count()));
            let mut display = text.clone();
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 150.0],
                        egui::TextEdit::multiline(&mut display)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
            if self.handoff_ocr {
                clear_ocr_handoff = ui.button("取消 OCR 接力选择").clicked();
                ui.small("当前顶部接力栏将发送这份 OCR 结果；不会自动执行或保存。");
            } else {
                set_ocr_handoff = ui.button("设为当前接力结果…").clicked();
            }
        }
        if relay {
            self.image_relay = Some(preview.picture.clone());
        }
        if copy {
            let dib = preview.dib.as_ref().unwrap();
            #[cfg(windows)]
            {
                self.message = match &self.listener {
                    Some(listener) => listener.copy_picture(&preview.picture, dib),
                    None => super::native::copy_picture(&preview.picture, dib),
                }
                .map_or_else(
                    |error| error,
                    |_| "已复制PNG与位图；本工具输出不回流采集".into(),
                );
            }
            #[cfg(not(windows))]
            {
                let _ = dib;
                self.message = "图片复制当前仅支持Windows".into();
            }
        }
        if save
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name("clipboard-image.png")
                .save_file()
        {
            use std::io::Write;
            let result = (|| -> anyhow::Result<()> {
                anyhow::ensure!(
                    path.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("png")),
                    "请选择.png路径"
                );
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                if let Err(error) = file
                    .write_all(&preview.picture.png)
                    .and_then(|_| file.sync_all())
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error.into());
                }
                Ok(())
            })();
            self.message = result.map_or_else(
                |_| "另存失败，目标已存在或不可写；历史保留".into(),
                |_| "已另存透明PNG".into(),
            );
        }
        if let Some(drag_start) = drag_start_update
            && let Some(preview) = self.image_preview.as_mut()
        {
            preview.ocr_drag_start = drag_start;
        }
        if toggle_region_selection {
            self.ocr_region_selecting = !self.ocr_region_selecting;
            if self.ocr_region_selecting {
                self.handoff_ocr = false;
                if let Some(preview) = self.image_preview.as_mut() {
                    preview.ocr_text = None;
                    preview.ocr_error.clear();
                }
            }
        }
        if let Some(region) = region_update {
            self.ocr_region_selecting = false;
            self.set_ocr_region(Some(region));
        } else if region_invalid {
            self.ocr_region_selecting = false;
            self.message = "局部区域太小或无效，请重新框选；原图片未改变".into();
        }
        if clear_region {
            self.ocr_region_selecting = false;
            self.set_ocr_region(None);
        }
        #[cfg(windows)]
        {
            if ocr_language_changed {
                self.set_ocr_language(selected_language.clone());
            }
            if refresh_ocr_languages {
                self.load_ocr_languages(ui.ctx(), preview_id);
            }
        }
        if recognize {
            #[cfg(windows)]
            {
                let region = self
                    .image_preview
                    .as_ref()
                    .filter(|preview| preview.id == preview_id)
                    .and_then(|preview| preview.ocr_region);
                self.start_ocr(
                    ui.ctx(),
                    preview_id,
                    preview_picture,
                    region,
                    self.ocr_language_tag.clone(),
                );
            }
        }
        if set_ocr_handoff {
            self.handoff_ocr = true;
            self.message = "OCR结果已设为当前接力来源；请使用顶部接力栏选择目标".into();
        }
        if clear_ocr_handoff {
            self.handoff_ocr = false;
            self.message = "已取消 OCR 接力选择".into();
        }
        if close {
            #[cfg(windows)]
            {
                self.ocr_languages_job = None;
                self.ocr_languages.clear();
                self.ocr_languages_error.clear();
                self.ocr_language_tag = None;
            }
            if self
                .image_preview
                .as_ref()
                .is_some_and(|p| p.job.is_some() || p.ocr_job.is_some())
            {
                self.image_preview.as_mut().unwrap().cancelled = true;
                self.handoff_ocr = false;
            } else {
                self.image_preview = None;
                self.handoff_ocr = false;
            }
        }
    }
    #[cfg(windows)]
    fn start_ocr(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        picture: std::sync::Arc<super::Picture>,
        region: Option<OcrRegion>,
        language_tag: Option<String>,
    ) {
        let Some(preview) = self
            .image_preview
            .as_mut()
            .filter(|preview| preview.id == id)
        else {
            return;
        };
        if preview.ocr_job.is_some() {
            return;
        }
        preview.ocr_text = None;
        preview.ocr_error.clear();
        self.handoff_ocr = false;
        self.ocr_task.begin();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = super::ocr::recognize(&picture, region, language_tag);
            let _ = tx.send(result);
            wake.request_repaint();
        });
        preview.ocr_job = Some(rx);
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

    fn copy_rich_html(&mut self, text: String, html: String) {
        #[cfg(windows)]
        {
            self.message = if let Some(listener) = &self.listener {
                match listener.copy_html(&text, &html) {
                    Ok(()) => "已复制富文本和纯文本回退；不会重复采集本工具输出".into(),
                    Err(error) => error,
                }
            } else {
                match super::native::copy_html_once(&text, &html) {
                    Ok(()) => "已复制富文本和纯文本回退".into(),
                    Err(error) => error,
                }
            };
        }
        #[cfg(not(windows))]
        {
            let _ = (text, html);
            self.message = "HTML富文本复制当前仅支持Windows".into();
        }
    }

    fn copy_rich_rtf(&mut self, text: String, rtf: Vec<u8>) {
        #[cfg(windows)]
        {
            self.message = if let Some(listener) = &self.listener {
                match listener.copy_rtf(&text, &rtf) {
                    Ok(()) => "已复制RTF富文本和纯文本回退；不会重复采集本工具输出".into(),
                    Err(error) => error,
                }
            } else {
                match super::native::copy_rtf_once(&text, &rtf) {
                    Ok(()) => "已复制RTF富文本和纯文本回退".into(),
                    Err(error) => error,
                }
            };
        }
        #[cfg(not(windows))]
        {
            let _ = (text, rtf);
            self.message = "RTF富文本复制当前仅支持Windows".into();
        }
    }

    fn history_entry_ui(&mut self, ui: &mut egui::Ui, index: usize, actions: &mut EntryActions) {
        let entry = &mut self.history.entries[index];
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut selected = self.selected.contains(&entry.id);
                if ui
                    .add_enabled(
                        entry.image.is_none()
                            && entry.file_references.is_empty()
                            && !entry.text.is_empty(),
                        egui::Checkbox::new(
                            &mut selected,
                            if entry.rich_html.is_some() {
                                "选择纯文本"
                            } else {
                                "选择文本"
                            },
                        ),
                    )
                    .changed()
                {
                    if selected {
                        if !self.selected.contains(&entry.id) {
                            self.selected.push(entry.id);
                        }
                    } else {
                        self.selected.retain(|id| *id != entry.id);
                    }
                }
                if !entry.file_references.is_empty() {
                    let mut selected_file = self.selected_files.contains(&entry.id);
                    if ui
                        .checkbox(
                            &mut selected_file,
                            format!("选择{}个文件", entry.file_references.len()),
                        )
                        .changed()
                    {
                        if selected_file {
                            if !self.selected_files.contains(&entry.id) {
                                self.selected_files.push(entry.id);
                            }
                        } else {
                            self.selected_files.retain(|id| *id != entry.id);
                        }
                    }
                }
                if ui
                    .checkbox(&mut entry.pinned, "置顶保留")
                    .on_hover_text(
                        "置顶不自动过期；取消置顶后，已过期记录将在下一次期限检查时删除。更改后随本机历史快照保存。",
                    )
                    .changed()
                {
                    actions.pin_changed = true;
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
                if let Some(image) = &entry.image {
                    ui.small(format!(
                        "图片 · {}×{} · PNG {:.1}KiB",
                        image.width,
                        image.height,
                        image.png.len() as f64 / 1024.0
                    ));
                    if ui.button("查看图片").clicked() {
                        actions.open_image = Some(entry.id);
                    }
                }
                if entry.image.is_none()
                    && entry.file_references.is_empty()
                    && !entry.text.is_empty()
                    && ui.button(if entry.rich_html.is_some() || entry.rich_rtf.is_some() { "复制纯文本" } else { "复制" }).clicked()
                {
                    actions.copy = Some(entry.text.clone());
                }
                if let Some(html) = &entry.rich_html {
                    ui.small(format!("HTML富文本 · {:.1} KiB", html.len() as f64 / 1024.0));
                    if ui.button("复制富文本").clicked() {
                        actions.copy_html = Some((entry.text.clone(), html.clone()));
                    }
                }
                if let Some(rtf) = &entry.rich_rtf {
                    ui.small(format!("RTF富文本 · {:.1} KiB", rtf.len() as f64 / 1024.0));
                    if ui.button("复制RTF富文本").clicked() {
                        actions.copy_rtf = Some((entry.text.clone(), rtf.clone()));
                    }
                }
                if !entry.file_references.is_empty() {
                    ui.small(format!("文件引用 · {}项", entry.file_references.len()))
                        .on_hover_text(entry.file_references.join("\n"));
                    if ui.button("复制路径列表").clicked() {
                        actions.copy_files = Some(entry.file_references.clone());
                    }
                }
                for label in &entry.labels {
                    let filter_button = ui.small_button(format!("#{label}"));
                    #[cfg(test)]
                    self.test_label_filter_rects
                        .push((label.clone(), filter_button.rect));
                    if filter_button
                        .on_hover_text("按此标签筛选历史")
                        .clicked()
                    {
                        actions.filter_label = Some(label.clone());
                    }
                    if ui.small_button("×").on_hover_text("移除标签").clicked() {
                        actions.remove_label = Some((entry.id, label.clone()));
                    }
                }
                if ui.small_button("+ 标签").clicked() {
                    actions.open_label_editor = Some(entry.id);
                }
                if ui.button("删除").clicked() {
                    actions.delete = Some(entry.id);
                }
            });
            if self.editing_label == Some(entry.id) {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.label_draft)
                            .desired_width(140.0)
                            .hint_text("最多32字"),
                    );
                    if ui.small_button("添加").clicked() {
                        actions.add_label = Some((entry.id, self.label_draft.clone()));
                    }
                    if ui.small_button("取消").clicked() {
                        self.editing_label = None;
                        self.label_draft.clear();
                    }
                });
            }
            if entry.image.is_none() && entry.file_references.is_empty() {
                let preview: String = entry.text.chars().take(180).collect();
                ui.label(preview);
                if entry.text.chars().count() > 180 {
                    ui.small("长文本已截断；下方组合预览可查看完整结果");
                }
                if entry.text.is_empty() && (entry.rich_html.is_some() || entry.rich_rtf.is_some()) {
                    ui.small(if entry.rich_html.is_some() && entry.rich_rtf.is_some() {
                        "该条目没有纯文本回退；仅可分别复制HTML或RTF富文本"
                    } else if entry.rich_html.is_some() {
                        "该条目没有纯文本回退；仅可复制HTML富文本"
                    } else {
                        "该条目没有纯文本回退；仅可复制RTF富文本"
                    });
                }
            }
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        #[cfg(test)]
        {
            self.test_label_filter_rects.clear();
            self.test_clear_search_rect = None;
            self.test_add_filtered_texts_rect = None;
            self.test_add_filtered_texts_enabled = None;
            self.test_format_combo_rect = None;
            self.test_markdown_format_rect = None;
            self.test_template_format_rect = None;
            self.test_records_format_rect = None;
            self.test_template_editor_rect = None;
            self.test_group_mode_rect = None;
            self.test_file_relay_rect = None;
            self.test_group_headers.clear();
            self.test_group_header_rects.clear();
        }
        #[cfg(windows)]
        if self.storage.restoring() {
            ui.heading("正在恢复本机保护历史");
            ui.spinner();
            ui.label("完成前暂不修改历史或开启采集。");
            return;
        }
        ui.heading("超级剪贴板");
        ui.label(format!(
            "v{} · 开发中：文本、图片、富文本、Windows本机OCR与跨工具确认接力",
            super::catalog_tool_version()
        ));
        ui.label("主动开启后采集新复制的文本；可另行开启本机保护保存，重启恢复历史但不自动采集。图片、文件引用、HTML和RTF富文本须分别主动开启；文件只记录路径，不读取文件。");
        #[cfg(windows)]
        ui.horizontal_wrapped(|ui| {
            if self.listener.is_none() {
                if ui
                    .add_enabled(
                        !self.storage.busy() && self.policy.ready,
                        egui::Button::new("开启历史采集"),
                    )
                    .clicked()
                {
                    match super::native::Listener::start(
                        ui.ctx().clone(),
                        self.policy.applied.clone(),
                    ) {
                        Ok(listener) => {
                            listener.set_images(self.images_enabled);
                            listener.set_files(self.files_enabled);
                            listener.set_html(self.html_enabled);
                            listener.set_rtf(self.rtf_enabled);
                            self.listener = Some(listener);
                            self.paused = false;
                            self.message = "已开启；仅采集后续复制的内容".into();
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
        if ui
            .checkbox(
                &mut self.images_enabled,
                "同时采集图片（PNG / 常见24、32位DIB，默认关闭）",
            )
            .changed()
        {
            #[cfg(windows)]
            if let Some(listener) = &self.listener {
                listener.set_images(self.images_enabled);
            }
            self.message = "图片采集范围已修改，清除排队通知，不补采当前剪贴板".into();
        }
        if ui
            .checkbox(
                &mut self.files_enabled,
                "同时采集文件引用（只记录路径，不读取文件；默认关闭）",
            )
            .on_hover_text("文件路径可能包含个人信息；只有勾选后才读取CF_HDROP路径列表")
            .changed()
        {
            #[cfg(windows)]
            if let Some(listener) = &self.listener {
                listener.set_files(self.files_enabled);
            }
            self.message = "文件引用采集范围已修改；不追溯先前剪贴板内容".into();
        }
        if ui
            .checkbox(
                &mut self.html_enabled,
                "同时采集HTML富文本（默认关闭，最多2 MiB）",
            )
            .on_hover_text(
                "只读取HTML片段；应用内不渲染。采集后可保留并复制格式，另含最多1 MiB纯文本回退",
            )
            .changed()
        {
            #[cfg(windows)]
            if let Some(listener) = &self.listener {
                listener.set_html(self.html_enabled);
            }
            self.message = "HTML富文本采集范围已修改；清除排队通知，不补采当前剪贴板".into();
        }
        if ui
            .checkbox(
                &mut self.rtf_enabled,
                "同时采集RTF富文本（默认关闭，最多2 MiB）",
            )
            .on_hover_text(
                "仅保留Windows RTF正文原始字节，不解析或渲染；剪贴板同时提供HTML和RTF时HTML优先；可复制纯文本回退或RTF",
            )
            .changed()
        {
            #[cfg(windows)]
            if let Some(listener) = &self.listener {
                listener.set_rtf(self.rtf_enabled);
            }
            self.message = "RTF富文本采集范围已修改；清除排队通知，不补采当前剪贴板".into();
        }
        if self.clear_confirm {
            ui.horizontal_wrapped(|ui| {
                ui.label("删除全部会话历史（含置顶）？");
                if ui.button("确认清空").clicked() {
                    self.history.entries.clear();
                    self.selected.clear();
                    self.selected_files.clear();
                    self.editing_label = None;
                    self.label_draft.clear();
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
        ui.small("当前最多500条、总32MiB；文本单条1MiB，文件引用最多64项/条且路径总量1MiB，图片最多400万像素/PNG16MiB。文件引用只保存路径，不读取文件；来源可能未知，必要时请暂停。采集类型默认关闭；本机历史保存须主动开启。");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("搜索文本、来源、标签、文件路径、HTML源码或RTF回退"),
            );
            if !self.search.is_empty() {
                let clear_search = ui.small_button("清除");
                #[cfg(test)]
                {
                    self.test_clear_search_rect = Some(clear_search.rect);
                }
                if clear_search.clicked() {
                    self.search.clear();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("历史视图");
            ui.selectable_value(&mut self.grouped_by_label, false, "列表");
            let _grouped = ui
                .selectable_value(&mut self.grouped_by_label, true, "按标签分组")
                .on_hover_text("多标签条目会出现在每个对应分组中，选择状态仍按条目去重");
            #[cfg(test)]
            {
                self.test_group_mode_rect = Some(_grouped.rect);
            }
        });
        let mut indices: Vec<usize> = (0..self.history.entries.len()).collect();
        indices.sort_by_key(|i| !self.history.entries[*i].pinned);
        let filtered_text_count = self
            .history
            .entries
            .iter()
            .filter(|entry| {
                entry.image.is_none()
                    && entry.file_references.is_empty()
                    && matches_query(entry, &self.search)
            })
            .count();
        let addable_text_count = self
            .history
            .entries
            .iter()
            .filter(|entry| {
                entry.image.is_none()
                    && entry.file_references.is_empty()
                    && matches_query(entry, &self.search)
                    && !self.selected.contains(&entry.id)
            })
            .count();
        let _add_filtered = ui
            .add_enabled(
                addable_text_count > 0,
                egui::Button::new(format!("加入未选文本 ({addable_text_count})")),
            )
            .on_hover_text(
                "将搜索结果中匹配的文本追加到组合选择；已选项不会重复，筛选外的选择保留",
            );
        ui.small(format!(
            "筛选命中{filtered_text_count}条文本，已有{}条在选择中",
            filtered_text_count.saturating_sub(addable_text_count)
        ));
        #[cfg(test)]
        {
            self.test_add_filtered_texts_rect = Some(_add_filtered.rect);
            self.test_add_filtered_texts_enabled = Some(_add_filtered.enabled());
        }
        if _add_filtered.clicked() {
            let added = add_filtered_text_entries(
                &self.history.entries,
                &indices,
                &mut self.selected,
                &self.search,
            );
            self.message = if added == 0 {
                "筛选中的文本已全部在选择中".into()
            } else {
                format!("已加入{added}条筛选文本；此前其他选择保留")
            };
        }
        let selected_file_paths = self
            .selected_files
            .iter()
            .filter_map(|id| self.history.entries.iter().find(|entry| entry.id == *id))
            .flat_map(|entry| entry.file_references.iter().cloned())
            .collect::<Vec<_>>();
        if !selected_file_paths.is_empty() {
            let relay = ui
                .button(format!(
                    "送入文件工具确认 · {}项",
                    selected_file_paths.len()
                ))
                .on_hover_text("只发送路径；将在现有文件确认页选择工具，不自动读取或处理");
            #[cfg(test)]
            {
                self.test_file_relay_rect = Some(relay.rect);
            }
            if relay.clicked() {
                let mut seen = std::collections::HashSet::new();
                let paths = selected_file_paths
                    .iter()
                    .filter(|path| seen.insert(path.to_lowercase()))
                    .cloned()
                    .collect::<Vec<_>>();
                if paths.len() > super::FILE_REFERENCE_LIMIT {
                    self.message = format!(
                        "一次最多送入{}个文件引用，请减少选择",
                        super::FILE_REFERENCE_LIMIT
                    );
                } else {
                    let total_bytes = paths.iter().map(String::len).sum::<usize>();
                    if total_bytes > super::FILE_REFERENCE_BYTE_LIMIT {
                        self.message = "一次接力路径总量超过1 MiB，请减少选择".into();
                    } else {
                        self.file_relay =
                            Some(paths.into_iter().map(std::path::PathBuf::from).collect());
                        self.message = "已加入文件接力队列；尚未读取或处理".into();
                    }
                }
            }
        }
        let selected_text_ids = self
            .selected
            .iter()
            .copied()
            .filter(|id| {
                self.history.entries.iter().any(|entry| {
                    entry.id == *id && entry.image.is_none() && entry.file_references.is_empty()
                })
            })
            .collect::<Vec<_>>();
        #[cfg(test)]
        let mut batch_label_rect = None;
        if selected_text_ids.len() >= 2 {
            let mut apply_batch_label = false;
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.batch_label_draft)
                        .hint_text("给已选文本统一加标签"),
                );
                let button = ui
                    .add_enabled(
                        !self.batch_label_draft.trim().is_empty(),
                        egui::Button::new(format!("批量添加到{}条", selected_text_ids.len())),
                    )
                    .on_hover_text("先预检所有选中记录；标签数量或历史容量超限时全部不修改");
                #[cfg(test)]
                {
                    batch_label_rect = Some(button.rect);
                }
                apply_batch_label = button.clicked();
            });
            if apply_batch_label {
                match self
                    .history
                    .add_label_to_many(&selected_text_ids, &self.batch_label_draft)
                {
                    Ok(0) => self.message = "所选文本已有该标签".into(),
                    Ok(count) => {
                        self.changed();
                        self.message = format!("已为{count}条文本添加标签");
                        self.batch_label_draft.clear();
                    }
                    Err(error) => self.message = error,
                }
            }
        }
        #[cfg(test)]
        {
            self.test_batch_label_rect = batch_label_rect;
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.separator();
        let mut actions = EntryActions::default();
        let groups = self
            .grouped_by_label
            .then(|| group_history_by_label(&self.history.entries, &indices, &self.search));
        #[cfg(test)]
        if let Some(groups) = &groups {
            self.test_group_headers = groups
                .iter()
                .map(|group| group.label.clone().unwrap_or_else(|| "未加标签".into()))
                .collect();
        }
        #[cfg(test)]
        let mut group_header_rects = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("clipboard-history-list")
            .max_height(if self.image_preview.is_some() {
                180.0
            } else {
                340.0
            })
            .show(ui, |ui| {
                if self.history.entries.is_empty() {
                    ui.label("开启采集后，在任意应用复制文本；也可先试用合成示例。");
                }
                if let Some(groups) = groups {
                    if groups.is_empty() {
                        ui.label("没有匹配的历史条目。");
                    }
                    for group in groups {
                        let title = group
                            .label
                            .as_ref()
                            .map_or_else(|| "未加标签".to_owned(), |label| format!("#{label}"));
                        let header = format!("{title} · {} 条", group.indices.len());
                        let _response = egui::CollapsingHeader::new(header)
                            .id_salt(("clipboard-label-group", title.as_str()))
                            .default_open(false)
                            .show(ui, |ui| {
                                for i in group.indices {
                                    self.history_entry_ui(ui, i, &mut actions);
                                }
                            });
                        #[cfg(test)]
                        group_header_rects.push((title, _response.header_response.rect));
                    }
                } else {
                    for i in indices {
                        if matches_query(&self.history.entries[i], &self.search) {
                            self.history_entry_ui(ui, i, &mut actions);
                        }
                    }
                }
            });
        #[cfg(test)]
        {
            self.test_group_header_rects = group_header_rects;
        }
        let EntryActions {
            pin_changed,
            copy,
            copy_html,
            copy_rtf,
            copy_files,
            delete,
            open_image,
            open_label_editor,
            add_label,
            remove_label,
            filter_label,
        } = actions;
        if let Some(id) = open_label_editor {
            self.grouped_by_label = false;
            self.editing_label = Some(id);
            self.label_draft.clear();
        }
        if let Some(id) = open_image {
            self.open_image(ui.ctx(), id);
        }
        self.image_ui(ui);
        if pin_changed {
            self.changed();
        }
        if let Some(id) = delete {
            self.changed();
            self.history.entries.retain(|e| e.id != id);
            self.selected.retain(|x| *x != id);
            self.selected_files.retain(|x| *x != id);
            if self.editing_label == Some(id) {
                self.editing_label = None;
                self.label_draft.clear();
            }
        }
        if let Some((id, label)) = remove_label {
            if self.history.remove_label(id, &label) {
                self.changed();
            }
        }
        if let Some((id, label)) = add_label {
            match self.history.add_label(id, &label) {
                Ok(true) => {
                    self.changed();
                    self.message = "标签已添加".into();
                }
                Ok(false) => self.message = "该标签已存在".into(),
                Err(error) => self.message = error,
            }
            self.editing_label = None;
            self.label_draft.clear();
        }
        if let Some(label) = filter_label {
            self.search = label.clone();
            self.message = format!("已按标签筛选：#{label}");
        }
        if let Some(text) = copy {
            self.copy(ui, text);
        }
        if let Some((text, html)) = copy_html {
            self.copy_rich_html(text, html);
        }
        if let Some((text, rtf)) = copy_rtf {
            self.copy_rich_rtf(text, rtf);
        }
        if let Some(paths) = copy_files {
            self.copy(ui, paths.join("\n"));
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("组合预览 · {}项", self.selected.len())).strong());
            let _format_response = egui::ComboBox::from_id_salt("clipboard-format")
                .selected_text(
                    [
                        "段落",
                        "编号",
                        "JSON数组",
                        "CSV单列",
                        "Markdown清单",
                        "自定义模板",
                        "JSON记录",
                    ][self.format],
                )
                .show_ui(ui, |ui| {
                    for (i, label) in [
                        "段落",
                        "编号",
                        "JSON数组",
                        "CSV单列",
                        "Markdown清单",
                        "自定义模板",
                        "JSON记录",
                    ]
                    .iter()
                    .enumerate()
                    {
                        let _response = ui.selectable_value(&mut self.format, i, *label);
                        #[cfg(test)]
                        if i == 4 {
                            self.test_markdown_format_rect = Some(_response.rect);
                        }
                        #[cfg(test)]
                        if i == 5 {
                            self.test_template_format_rect = Some(_response.rect);
                        }
                        #[cfg(test)]
                        if i == 6 {
                            self.test_records_format_rect = Some(_response.rect);
                        }
                    }
                });
            #[cfg(test)]
            {
                self.test_format_combo_rect = Some(_format_response.response.rect);
            }
            if ui.button("取消全部选择").clicked() {
                self.selected.clear();
            }
        });
        if self.format == 5 {
            let template = self
                .template
                .get_or_insert_with(|| super::DEFAULT_TEMPLATE.to_owned());
            ui.horizontal_wrapped(|ui| {
                ui.label("模板");
                if ui.button("恢复默认模板").clicked() {
                    *template = super::DEFAULT_TEMPLATE.to_owned();
                }
            });
            let _editor = ui.add(
                egui::TextEdit::multiline(template)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("{{index}}. {{text}}"),
            );
            #[cfg(test)]
            {
                self.test_template_editor_rect = Some(_editor.rect);
            }
            ui.small("变量：{{index}} 序号、{{text}} 正文、{{source}} 来源、{{time}} 时间、{{labels}} 标签。模板仅保留在当前会话；必须包含 {{text}}，先预览再手动复制。");
        }
        let combined = if self.format == 5 {
            self.history.combined_template(
                &self.selected,
                self.template.as_deref().unwrap_or(super::DEFAULT_TEMPLATE),
            )
        } else {
            self.history.combined(&self.selected, self.format)
        };
        match combined {
            Ok(mut text) => {
                self.output.clone_from(&text);
                ui.small("顺序按勾选先后；取消后重选可改变次序。历史原文不变。可通过上方接力入口发送备忘/数据等工具。");
                if self.format == 3 {
                    ui.small("CSV对可能的公式加前置单引号，降低电子表格将其识别为公式的风险。");
                } else if self.format == 4 {
                    ui.small("Markdown清单：每项加无序列表标记，多行内容缩进到同一项。");
                } else if self.format == 5 {
                    ui.small("模板按所选顺序逐条展开；正文里的变量标记按原文保留，不会二次解析。");
                } else if self.format == 6 {
                    ui.small("JSON记录含所选文本、来源应用文件名、复制时间与标签；先检查预览，再复制或接力。");
                }
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
        if ui.button("载入合成图片示例（不读取系统剪贴板）").clicked() {
            self.add_picture_example(ui.ctx());
        }
        if ui.button("载入合成示例（不读取系统剪贴板）").clicked() {
            self.preview_fixture();
        }
    }
    pub fn transfer_text(&self) -> Option<(String, &str)> {
        if self.handoff_ocr
            && let Some(preview) = &self.image_preview
            && let Some(text) = &preview.ocr_text
            && !text.is_empty()
        {
            #[cfg(windows)]
            let language = self.ocr_language_tag.as_deref().unwrap_or("系统首选语言");
            #[cfg(not(windows))]
            let language = "系统首选语言";
            return Some((
                format!(
                    "剪贴板 v{} · OCR{} · {} #{}{}",
                    super::catalog_tool_version(),
                    if preview.ocr_region.is_some() {
                        "局部"
                    } else {
                        "整图"
                    },
                    language,
                    preview.id,
                    self.handoff_source_summary(&[preview.id]),
                ),
                text,
            ));
        }
        if self.output.is_empty() {
            None
        } else {
            Some((
                format!(
                    "剪贴板 v{} · [{}]{}{}",
                    super::catalog_tool_version(),
                    self.selected
                        .iter()
                        .take(8)
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                    if self.selected.len() > 8 {
                        format!("+{}", self.selected.len() - 8)
                    } else {
                        String::new()
                    },
                    self.handoff_source_summary(&self.selected),
                ),
                &self.output,
            ))
        }
    }
    fn handoff_source_summary(&self, ids: &[u64]) -> String {
        let mut sources = Vec::<String>::new();
        for id in ids {
            let Some(entry) = self.history.entries.iter().find(|entry| entry.id == *id) else {
                continue;
            };
            let basename = entry.source.rsplit(['\\', '/']).next().unwrap_or_default();
            let name = basename
                .chars()
                .filter(|character| !character.is_control())
                .take(48)
                .collect::<String>();
            let name = if name.trim().is_empty() {
                "未知来源".to_owned()
            } else {
                name
            };
            if !sources
                .iter()
                .any(|source| source.eq_ignore_ascii_case(&name))
            {
                sources.push(name);
            }
        }
        if sources.is_empty() {
            return String::new();
        }
        let summary = sources
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        if sources.len() > 4 {
            format!(" · {summary} 等{}个应用", sources.len())
        } else {
            format!(" · {summary}")
        }
    }
    fn add_picture_example(&mut self, ctx: &egui::Context) {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(600, 240, |x, y| {
            let dx = x as i32 - 300;
            let dy = y as i32 - 120;
            image::Rgba([
                ((x / 3) + 30) as u8,
                ((y / 2) + 70) as u8,
                180,
                if dx * dx / 9 + dy * dy < 12500 {
                    if x < 200 { 120 } else { 255 }
                } else {
                    0
                },
            ])
        }));
        let value = match super::Picture::from_image(image) {
            Ok(value) => std::sync::Arc::new(value),
            Err(_) => {
                self.message = "图片示例准备失败".into();
                return;
            }
        };
        match self.history.insert_image(value, "合成图片示例".into()) {
            Ok(()) => {
                self.changed();
                self.open_image(ctx, self.history.entries[0].id);
            }
            Err(error) => self.message = error,
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_fixture(&mut self, ctx: &egui::Context) {
        self.add_picture_example(ctx);
        self.selected.clear();
        self.output.clear();
        self.message = "合成透明图片示例；不读取系统剪贴板或真实历史".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ocr_fixture(&mut self, ctx: &egui::Context) {
        self.preview_image_fixture(ctx);
        if let Some(preview) = self.image_preview.as_mut() {
            preview.ocr_text = Some(
                "这是合成 OCR 结果，仅用于预览界面布局。\n实际文字由 Windows 本机识别引擎生成。"
                    .into(),
            );
            self.handoff_ocr = true;
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ocr_region_fixture(&mut self, ctx: &egui::Context) {
        self.preview_image_fixture(ctx);
        if let Some(preview) = self.image_preview.as_mut() {
            preview.ocr_region = Some(OcrRegion {
                x: 90,
                y: 48,
                width: 330,
                height: 132,
            });
            preview.ocr_text = Some(
                "教程步骤一：在图片中框选需要识别的部分。\n此处展示的是合成区域与合成文本。".into(),
            );
            self.handoff_ocr = true;
        }
        self.message = "合成局部 OCR 示例；不访问系统剪贴板或调用识别服务".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_ocr_language_fixture(&mut self, ctx: &egui::Context) {
        self.preview_ocr_fixture(ctx);
        #[cfg(windows)]
        {
            self.ocr_languages_job = None;
            self.ocr_languages = vec![
                super::ocr::RecognizerLanguage {
                    tag: "en-US".into(),
                    display_name: "English (United States)".into(),
                },
                super::ocr::RecognizerLanguage {
                    tag: "zh-CN".into(),
                    display_name: "Chinese (Simplified)".into(),
                },
                super::ocr::RecognizerLanguage {
                    tag: "ja-JP".into(),
                    display_name: "Japanese".into(),
                },
            ];
            self.ocr_language_tag = Some("zh-CN".into());
        }
        self.message = "合成 OCR 语言选择示例；语言选项和识别文字均为演示数据".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_provenance_handoff_fixture(&mut self) {
        self.history = super::History::default();
        self.selected.clear();
        for (text, source) in [
            ("教程标题", "editor.exe"),
            ("参考链接 https://example.test", "browser.exe"),
        ] {
            self.history.insert(text.into(), source.into()).unwrap();
            self.selected.push(self.history.entries[0].id);
        }
        self.output = "教程标题\n参考链接 https://example.test".into();
        self.handoff_ocr = false;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_image_ready(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<std::sync::Arc<super::Picture>> {
        self.poll_image(ctx);
        let preview = self.image_preview.as_ref()?;
        if preview.texture.is_some() && preview.dib.is_some() {
            let source = self
                .history
                .entries
                .iter()
                .find(|e| e.id == preview.id)
                .unwrap()
                .image
                .as_ref()
                .unwrap();
            assert!(std::sync::Arc::ptr_eq(source, &preview.picture));
            assert!(
                preview
                    .picture
                    .image()
                    .unwrap()
                    .to_rgba8()
                    .pixels()
                    .any(|p| p.0[3] == 0)
            );
            Some(preview.picture.clone())
        } else {
            None
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
                "第二步：多片段可按选择顺序组合为Markdown清单、JSON数组等格式。",
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
        for (source, label) in [
            ("示例编辑器", "教程"),
            ("示例终端", "JSON"),
            ("示例浏览器", "教程"),
        ] {
            if let Some(id) = self
                .history
                .entries
                .iter()
                .find(|entry| entry.source == source)
                .map(|entry| entry.id)
            {
                let _ = self.history.add_label(id, label);
            }
        }
        self.format = 4;
        self.output = self
            .history
            .combined(&self.selected, self.format)
            .unwrap_or_default();
        self.message = "合成示例不读取系统剪贴板；采集状态见上方".into();
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_json_records_fixture(&mut self) {
        self.preview_fixture();
        self.format = 6;
        self.output = self
            .history
            .combined(&self.selected, self.format)
            .expect("synthetic JSON record fixture is valid");
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_grouped_fixture(&mut self) {
        self.preview_fixture();
        self.batch_label_draft = "本轮整理".into();
        let tutorial_id = self
            .history
            .entries
            .iter()
            .find(|entry| entry.source == "示例编辑器")
            .map(|entry| entry.id)
            .expect("clipboard preview contains a tutorial entry");
        self.history.add_label(tutorial_id, "JSON").unwrap();
        self.history
            .insert("尚未加标签的示例片段".into(), "示例草稿".into())
            .unwrap();
        self.grouped_by_label = true;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_template_fixture(&mut self) {
        self.preview_fixture();
        self.format = 5;
        self.template = Some("{{index}} · {{source}} [{{labels}}]\n{{text}}".into());
        self.grouped_by_label = false;
        if let Some(tutorial_id) = self
            .history
            .entries
            .iter()
            .find(|entry| entry.source == "示例编辑器")
            .map(|entry| entry.id)
        {
            self.selected = vec![tutorial_id];
        }
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_file_reference_fixture(&mut self) {
        self.preview_fixture();
        self.files_enabled = true;
        self.history
            .insert_files(
                vec![
                    r"C:\教程示例\订单模板.csv".into(),
                    r"C:\教程示例\产品图片.png".into(),
                ],
                "合成文件选择".into(),
            )
            .expect("synthetic file-reference preview fits history bounds");
        let id = self.history.entries[0].id;
        self.selected_files = vec![id];
        self.message = "合成路径示例；文件内容尚未读取，可继续送入确认页选择工具。".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_html_fixture(&mut self) {
        self.preview_fixture();
        self.html_enabled = true;
        self.history
            .insert_rich_text(
                "教程标题与正文".into(),
                "<article><h1>教程标题</h1><p><strong>正文</strong>包含链接与格式。</p></article>"
                    .into(),
                "合成网页编辑器".into(),
            )
            .expect("synthetic HTML preview fits history bounds");
        self.message = "合成HTML片段；应用内不渲染，可分别复制纯文本或富文本。".into();
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_rtf_fixture(&mut self) {
        self.history = History::default();
        self.selected.clear();
        self.selected_files.clear();
        self.output.clear();
        self.format = 0;
        self.template = None;
        self.grouped_by_label = false;
        self.html_enabled = true;
        self.rtf_enabled = true;
        self.history
            .insert_rich_formats(
                "RTF示例：粗体、颜色与段落".into(),
                "<article><h1>格式示例</h1><p><strong>粗体</strong>与<span style=\"color:#1e5ab4\">颜色</span>。</p></article>".into(),
                br#"{\rtf1\ansi\deff0{\colortbl;\red30\green90\blue180;}\pard RTF example: {\b bold} and {\cf1 color}.\par Next paragraph.}"#.to_vec(),
                "合成文档编辑器".into(),
            )
            .expect("synthetic HTML and RTF preview fits history bounds");
        self.message = "同一合成条目同时保留HTML与RTF；应用内均不解析或渲染，可分别复制。".into();
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

    fn render(state: &mut State, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1125.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| state.ui(ui));
        });
    }

    fn click(state: &mut State, ctx: &egui::Context, position: egui::Pos2) {
        render(
            state,
            ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        render(
            state,
            ctx,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    #[cfg(windows)]
    fn drag(state: &mut State, ctx: &egui::Context, start: egui::Pos2, end: egui::Pos2) {
        render(
            state,
            ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        render(state, ctx, vec![egui::Event::PointerMoved(end)]);
        render(
            state,
            ctx,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }

    #[test]
    fn ocr_drag_maps_preview_coordinates_to_original_pixels_and_clips_edges() {
        let image_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(480.0, 240.0));
        let region = ocr_region_from_drag(
            image_rect,
            [600, 300],
            egui::pos2(220.0, 98.0),
            egui::pos2(460.0, 194.0),
        )
        .unwrap();
        assert_eq!(
            region,
            OcrRegion {
                x: 150,
                y: 60,
                width: 300,
                height: 120,
            }
        );
        let reverse = ocr_region_from_drag(
            image_rect,
            [600, 300],
            egui::pos2(460.0, 194.0),
            egui::pos2(220.0, 98.0),
        );
        assert_eq!(reverse, Some(region));
        let clipped = ocr_region_from_drag(
            image_rect,
            [600, 300],
            egui::pos2(50.0, 30.0),
            egui::pos2(220.0, 98.0),
        )
        .unwrap();
        assert_eq!(
            clipped,
            OcrRegion {
                x: 0,
                y: 0,
                width: 150,
                height: 60,
            }
        );
        assert!(
            ocr_region_from_drag(
                image_rect,
                [600, 300],
                egui::pos2(220.0, 98.0),
                egui::pos2(220.2, 98.2),
            )
            .is_none()
        );
    }

    #[test]
    fn changing_ocr_region_invalidates_previous_result_and_handoff() {
        let mut state = State::default();
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(16, 12)).unwrap(),
        );
        state
            .history
            .insert_image(picture.clone(), "fixture.exe".into())
            .unwrap();
        state.image_preview = Some(Preview {
            cancelled: false,
            id: state.history.entries[0].id,
            picture,
            texture: None,
            dib: None,
            error: String::new(),
            job: None,
            ocr_job: None,
            ocr_text: Some("stale full-image result".into()),
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        state.handoff_ocr = true;

        state.set_ocr_region(Some(OcrRegion {
            x: 4,
            y: 3,
            width: 8,
            height: 6,
        }));

        let preview = state.image_preview.as_ref().unwrap();
        assert_eq!(preview.ocr_region.unwrap().width, 8);
        assert!(preview.ocr_text.is_none());
        assert!(!state.handoff_ocr);
        assert!(state.transfer_text().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn changing_ocr_language_invalidates_previous_result_and_handoff() {
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        );
        let mut state = State {
            image_preview: Some(Preview {
                cancelled: false,
                id: 1,
                picture,
                texture: None,
                dib: None,
                error: String::new(),
                job: None,
                ocr_job: None,
                ocr_text: Some("old language result".into()),
                ocr_error: "old error".into(),
                ocr_region: None,
                ocr_drag_start: None,
            }),
            ocr_language_tag: Some("zh-CN".into()),
            handoff_ocr: true,
            ..Default::default()
        };

        state.set_ocr_language(Some("en-US".into()));

        assert_eq!(state.ocr_language_tag.as_deref(), Some("en-US"));
        assert!(state.image_preview.as_ref().unwrap().ocr_text.is_none());
        assert!(state.image_preview.as_ref().unwrap().ocr_error.is_empty());
        assert!(!state.handoff_ocr);
        assert!(state.transfer_text().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn ocr_language_picker_selects_an_installed_language_from_the_preview_ui() {
        let ctx = egui::Context::default();
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(20, 12)).unwrap(),
        );
        let texture = ctx.load_texture(
            "clipboard-language-picker-test",
            picture.thumbnail().unwrap(),
            egui::TextureOptions::LINEAR,
        );
        let mut state = State {
            ocr_languages: vec![
                super::super::ocr::RecognizerLanguage {
                    tag: "en-US".into(),
                    display_name: "English (United States)".into(),
                },
                super::super::ocr::RecognizerLanguage {
                    tag: "zh-CN".into(),
                    display_name: "Chinese (Simplified)".into(),
                },
            ],
            image_preview: Some(Preview {
                cancelled: false,
                id: 1,
                picture,
                texture: Some(texture),
                dib: None,
                error: String::new(),
                job: None,
                ocr_job: None,
                ocr_text: Some("stale result".into()),
                ocr_error: String::new(),
                ocr_region: None,
                ocr_drag_start: None,
            }),
            handoff_ocr: true,
            ..Default::default()
        };
        render(&mut state, &ctx, vec![]);
        let picker = state.test_ocr_language_rect.unwrap().center();
        click(&mut state, &ctx, picker);
        render(&mut state, &ctx, vec![]);
        let option = state
            .test_ocr_language_option_rects
            .iter()
            .find(|(tag, _)| tag == "en-US")
            .map(|(_, rect)| rect.center())
            .expect("the installed English language appears in the picker");
        click(&mut state, &ctx, option);

        assert_eq!(state.ocr_language_tag.as_deref(), Some("en-US"));
        assert!(state.image_preview.as_ref().unwrap().ocr_text.is_none());
        assert!(!state.handoff_ocr);
    }

    #[cfg(windows)]
    #[test]
    fn late_ocr_language_list_for_another_preview_is_discarded() {
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = State {
            image_preview: Some(Preview {
                cancelled: false,
                id: 2,
                picture,
                texture: None,
                dib: None,
                error: String::new(),
                job: None,
                ocr_job: None,
                ocr_text: None,
                ocr_error: String::new(),
                ocr_region: None,
                ocr_drag_start: None,
            }),
            ocr_languages_job: Some((1, rx)),
            ..Default::default()
        };
        tx.send(Ok(vec![super::super::ocr::RecognizerLanguage {
            tag: "en-US".into(),
            display_name: "English (United States)".into(),
        }]))
        .unwrap();

        state.poll_ocr_languages(&egui::Context::default());

        assert!(state.ocr_languages.is_empty());
        assert!(state.ocr_languages_job.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn dragging_image_preview_selects_region_and_clears_old_handoff() {
        let ctx = egui::Context::default();
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(600, 300)).unwrap(),
        );
        let mut state = State::default();
        state
            .history
            .insert_image(picture.clone(), "fixture.exe".into())
            .unwrap();
        let texture = ctx.load_texture(
            "clipboard-region-test",
            picture.thumbnail().unwrap(),
            egui::TextureOptions::LINEAR,
        );
        state.image_preview = Some(Preview {
            cancelled: false,
            id: state.history.entries[0].id,
            picture,
            texture: Some(texture),
            dib: None,
            error: String::new(),
            job: None,
            ocr_job: None,
            ocr_text: Some("stale full-image result".into()),
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        state.handoff_ocr = true;
        render(&mut state, &ctx, vec![]);
        let toggle = state.test_ocr_region_toggle_rect.unwrap().center();
        click(&mut state, &ctx, toggle);
        assert!(state.ocr_region_selecting);
        assert!(state.image_preview.as_ref().unwrap().ocr_text.is_none());
        assert!(!state.handoff_ocr);
        render(&mut state, &ctx, vec![]);
        let image_rect = state.test_ocr_image_rect.unwrap();
        drag(
            &mut state,
            &ctx,
            egui::pos2(
                image_rect.left() + image_rect.width() * 0.25,
                image_rect.top() + image_rect.height() * 0.2,
            ),
            egui::pos2(
                image_rect.left() + image_rect.width() * 0.75,
                image_rect.top() + image_rect.height() * 0.8,
            ),
        );
        let preview = state.image_preview.as_ref().unwrap();
        assert_eq!(
            preview.ocr_region,
            Some(OcrRegion {
                x: 150,
                y: 60,
                width: 300,
                height: 180,
            })
        );
        assert!(!state.ocr_region_selecting);
    }

    #[cfg(feature = "ui-preview")]
    #[test]
    fn rich_format_preview_keeps_html_and_rtf_on_one_entry() {
        let mut state = State::default();
        assert!(!state.rtf_enabled);
        state.preview_rtf_fixture();
        assert!(state.rtf_enabled);
        assert_eq!(state.history.entries.len(), 1);
        let entry = &state.history.entries[0];
        assert_eq!(entry.text, "RTF示例：粗体、颜色与段落");
        assert!(
            entry
                .rich_html
                .as_deref()
                .is_some_and(|html| html.contains("<strong>粗体</strong>"))
        );
        assert!(
            entry
                .rich_rtf
                .as_deref()
                .is_some_and(|bytes| { bytes.starts_with(b"{\\rtf") && bytes.ends_with(b"}") })
        );
        let ctx = egui::Context::default();
        render(&mut state, &ctx, vec![]);
    }

    #[test]
    fn clicking_label_filters_history_and_clear_restores_all_entries() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        render(&mut state, &ctx, vec![]);
        let label_position = state
            .test_label_filter_rects
            .iter()
            .find(|(label, _)| label == "教程")
            .map(|(_, rect)| rect.center())
            .expect("preview contains a clickable tutorial label");

        click(&mut state, &ctx, label_position);
        assert_eq!(state.search, "教程");
        assert_eq!(
            state
                .history
                .entries
                .iter()
                .filter(|entry| matches_query(entry, &state.search))
                .count(),
            2
        );

        render(&mut state, &ctx, vec![]);
        let clear_position = state
            .test_clear_search_rect
            .expect("active search shows a clear button")
            .center();
        click(&mut state, &ctx, clear_position);
        assert!(state.search.is_empty());
        assert_eq!(state.history.entries.len(), 3);
    }

    #[test]
    fn selected_file_references_relay_paths_only_after_explicit_click() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state
            .history
            .insert_files(
                vec![r"C:\synthetic\tutorial.pdf".into()],
                "合成文件来源".into(),
            )
            .unwrap();
        state.selected_files.push(state.history.entries[0].id);

        render(&mut state, &ctx, vec![]);
        assert!(state.file_relay.is_none());
        let position = state
            .test_file_relay_rect
            .expect("selected file refs show an explicit handoff button")
            .center();
        click(&mut state, &ctx, position);

        assert!(state.notify_file_relay_waiting());
        assert!(!state.notify_file_relay_waiting());
        assert!(state.message.contains("路径接力已保留"));
        assert_eq!(
            state.take_file_relay(),
            Some(vec![std::path::PathBuf::from(r"C:\synthetic\tutorial.pdf")])
        );
        assert!(!state.notify_file_relay_waiting());
        assert_eq!(state.history.entries[0].file_references.len(), 1);
        assert!(state.message.contains("路径接力已保留"));
    }

    #[test]
    fn add_filtered_selection_button_appends_once_and_preserves_other_selection() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        let tutorial_id = state
            .history
            .entries
            .iter()
            .find(|entry| entry.source == "示例编辑器")
            .unwrap()
            .id;
        let json_id = state
            .history
            .entries
            .iter()
            .find(|entry| entry.source == "示例终端")
            .unwrap()
            .id;
        let second_tutorial_id = state
            .history
            .entries
            .iter()
            .find(|entry| entry.source == "示例浏览器")
            .unwrap()
            .id;
        state.search = "教程".into();
        state.selected = vec![tutorial_id, json_id];
        render(&mut state, &ctx, vec![]);
        let button_position = state
            .test_add_filtered_texts_rect
            .expect("filtered selection button is visible")
            .center();

        click(&mut state, &ctx, button_position);
        assert_eq!(
            state.selected,
            vec![tutorial_id, json_id, second_tutorial_id]
        );
        assert!(state.message.contains("已加入1条"));

        render(&mut state, &ctx, vec![]);
        assert_eq!(state.selected.len(), 3);
        assert_eq!(state.test_add_filtered_texts_enabled, Some(false));
    }

    #[test]
    fn filtered_selection_adds_only_matching_text_without_duplicates() {
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        );
        let mut history = super::super::History::default();
        history
            .insert("match text".into(), "fixture-app".into())
            .unwrap();
        let text_id = history.entries[0].id;
        history.insert_image(picture, "fixture-app".into()).unwrap();
        let mut selected = vec![999, text_id];

        let ordered = (0..history.entries.len()).collect::<Vec<_>>();
        let added =
            add_filtered_text_entries(&history.entries, &ordered, &mut selected, "fixture-app");
        assert_eq!(added, 0);
        assert_eq!(selected, vec![999, text_id]);
        assert_eq!(
            add_filtered_text_entries(&history.entries, &ordered, &mut selected, "match text"),
            0
        );
        assert_eq!(selected, vec![999, text_id]);
    }

    #[test]
    fn batch_label_button_labels_only_selected_text_entries() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        let selected_ids = state.selected.iter().copied().take(2).collect::<Vec<_>>();
        let untouched_id = state
            .history
            .entries
            .iter()
            .find(|entry| !selected_ids.contains(&entry.id))
            .unwrap()
            .id;
        state.selected = selected_ids.clone();
        state.batch_label_draft = "本轮整理".into();
        render(&mut state, &ctx, vec![]);
        let button_position = state
            .test_batch_label_rect
            .expect("multiple text selections show batch label action")
            .center();

        click(&mut state, &ctx, button_position);

        for id in selected_ids {
            let entry = state
                .history
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap();
            assert!(entry.labels.iter().any(|label| label == "本轮整理"));
        }
        let untouched = state
            .history
            .entries
            .iter()
            .find(|entry| entry.id == untouched_id)
            .unwrap();
        assert!(!untouched.labels.iter().any(|label| label == "本轮整理"));
        assert!(state.message.contains("已为2条文本"));
        assert!(state.batch_label_draft.is_empty());
    }

    #[test]
    fn format_picker_selects_markdown_and_previews_selected_texts() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        state.format = 2;
        render(&mut state, &ctx, vec![]);
        let combo_position = state.test_format_combo_rect.unwrap().center();
        click(&mut state, &ctx, combo_position);
        render(&mut state, &ctx, vec![]);
        let markdown_position = state
            .test_markdown_format_rect
            .expect("open format menu contains Markdown清单")
            .center();
        click(&mut state, &ctx, markdown_position);

        assert_eq!(state.format, 4);
        let preview = state
            .history
            .combined(&state.selected, state.format)
            .unwrap();
        assert!(preview.starts_with("- 教程第一步：准备示例数据\n  仅使用合成内容"));
        assert!(preview.contains("- {\"project\":\"Zi DevTools\",\"status\":\"demo\"}"));
    }

    #[test]
    fn format_picker_selects_structured_json_records_and_previews_provenance() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        state.format = 2;
        render(&mut state, &ctx, vec![]);
        let combo_position = state.test_format_combo_rect.unwrap().center();
        click(&mut state, &ctx, combo_position);
        render(&mut state, &ctx, vec![]);
        let records_position = state
            .test_records_format_rect
            .expect("open format menu contains JSON records")
            .center();
        click(&mut state, &ctx, records_position);

        assert_eq!(state.format, 6);
        let records: Vec<serde_json::Value> = serde_json::from_str(&state.output).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["order"], 1);
        assert_eq!(records[0]["source"], "示例编辑器");
        assert_eq!(records[0]["labels"], serde_json::json!(["教程"]));
        assert!(state.output.contains("第二步：多片段"));
    }

    #[test]
    fn format_picker_opens_session_template_editor_and_previews_expansion() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        state.format = 2;
        render(&mut state, &ctx, vec![]);
        let combo_position = state.test_format_combo_rect.unwrap().center();
        click(&mut state, &ctx, combo_position);
        render(&mut state, &ctx, vec![]);
        let template_position = state
            .test_template_format_rect
            .expect("open format menu contains the template format")
            .center();
        click(&mut state, &ctx, template_position);

        assert_eq!(state.format, 5);
        assert_eq!(
            state.template.as_deref(),
            Some(super::super::DEFAULT_TEMPLATE)
        );
        assert!(state.test_template_editor_rect.is_some());
        assert!(
            state
                .output
                .starts_with("1. 教程第一步：准备示例数据\n仅使用合成内容")
        );
        assert!(
            state
                .output
                .contains("2. {\"project\":\"Zi DevTools\",\"status\":\"demo\"}")
        );
    }

    #[test]
    fn history_search_matches_labels_case_insensitively() {
        let mut history = super::super::History::default();
        history
            .insert("sample body".into(), "fixture-app".into())
            .unwrap();
        let id = history.entries[0].id;
        history.add_label(id, "教程资料").unwrap();
        let entry = &history.entries[0];
        assert!(matches_query(entry, "教程"));
        assert!(matches_query(entry, "FIXTURE"));
        assert!(matches_query(entry, "sample body"));
        assert!(matches_query(entry, "   "));
        assert!(!matches_query(entry, "missing"));
    }

    #[test]
    fn rich_html_searches_fragment_and_text_selection_needs_plain_fallback() {
        assert!(contains_lowercase_match("<p>Tutorial</p>", "tutorial"));
        assert!(contains_lowercase_match("İstanbul", "i\u{307}stanbul"));
        assert!(!contains_lowercase_match("abc", "abd"));
        let mut history = super::super::History::default();
        history
            .insert_rich_text(
                String::new(),
                "<article><h1>教程片段</h1></article>".into(),
                "fixture.exe".into(),
            )
            .unwrap();
        let html_only = history.entries[0].id;
        assert!(matches_query(&history.entries[0], "教程片段"));
        assert!(history.combined(&[html_only], 0).is_err());
        assert!(
            history
                .combined_template(&[html_only], "{{source}} {{text}}")
                .is_err()
        );
        let mut selected = Vec::new();
        assert_eq!(
            add_filtered_text_entries(&history.entries, &[0], &mut selected, "教程片段"),
            0,
            "HTML-only entry cannot silently add an empty text value"
        );
        history
            .insert_rich_text(
                "纯文本回退".into(),
                "<p>rich fragment</p>".into(),
                "fixture.exe".into(),
            )
            .unwrap();
        let fallback = history.entries[0].id;
        assert_eq!(
            add_filtered_text_entries(&history.entries, &[0, 1], &mut selected, ""),
            1
        );
        assert_eq!(selected, vec![fallback]);
        assert!(!selected.contains(&html_only));
    }

    #[test]
    fn label_groups_are_sorted_search_aware_and_keep_multi_label_membership() {
        let entry = |id, text: &str, labels: &[&str]| super::super::Entry {
            id,
            text: text.into(),
            rich_html: None,
            rich_rtf: None,
            image: None,
            file_references: Vec::new(),
            source: "fixture.exe".into(),
            time: String::new(),
            pinned: false,
            labels: labels.iter().map(|label| (*label).into()).collect(),
            first_captured_utc: None,
            last_captured_utc: None,
        };
        let entries = vec![
            entry(1, "alpha", &["工作", "A"]),
            entry(2, "beta", &["a", "A"]),
            entry(3, "unlabelled", &[]),
            entry(4, "other", &["z"]),
        ];
        let groups = group_history_by_label(&entries, &[0, 1, 2, 3], "");
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].label.as_deref(), Some("A"));
        assert_eq!(groups[0].indices, vec![0, 1]);
        assert_eq!(groups[1].label.as_deref(), Some("z"));
        assert_eq!(groups[1].indices, vec![3]);
        assert_eq!(groups[2].label.as_deref(), Some("工作"));
        assert_eq!(groups[2].indices, vec![0]);
        assert_eq!(groups[3].label, None);
        assert_eq!(groups[3].indices, vec![2]);

        let filtered = group_history_by_label(&entries, &[0, 1, 2, 3], "beta");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label.as_deref(), Some("a"));
        assert_eq!(filtered[0].indices, vec![1]);
    }

    #[test]
    fn grouped_history_toggle_renders_tag_sections() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.preview_fixture();
        render(&mut state, &ctx, vec![]);
        let grouped_position = state
            .test_group_mode_rect
            .expect("grouping control is visible")
            .center();
        click(&mut state, &ctx, grouped_position);

        assert!(state.grouped_by_label);
        render(&mut state, &ctx, vec![]);
        assert!(state.test_group_headers.contains(&"教程".into()));
        assert!(state.test_group_headers.contains(&"JSON".into()));
        let tutorial_header = state
            .test_group_header_rects
            .iter()
            .find(|(title, _)| title == "#教程")
            .map(|(_, rect)| rect.center())
            .expect("tutorial group header is visible");
        click(&mut state, &ctx, tutorial_header);
        assert!(
            state
                .test_label_filter_rects
                .iter()
                .any(|(label, _)| label == "教程")
        );
    }

    #[test]
    fn restored_id_cannot_reuse_old_picture_preview_or_late_cancelled_texture() {
        let ctx = egui::Context::default();
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(4, 3)).unwrap(),
        );
        let mut state = State::default();
        state
            .history
            .insert_image(picture.clone(), "fixture".into())
            .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        state.image_preview = Some(Preview {
            cancelled: false,
            id: state.history.entries[0].id,
            picture: picture.clone(),
            texture: None,
            dib: None,
            error: String::new(),
            job: Some(rx),
            ocr_job: None,
            ocr_text: None,
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        // A restore may reuse the stable numeric ID for a different actual payload.
        state.history.entries[0].image = Some(std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        ));
        tx.send(Ok((
            picture.thumbnail().unwrap(),
            super::super::media::to_dib(&picture).unwrap(),
        )))
        .unwrap();
        state.poll_image(&ctx);
        assert!(state.image_preview.is_none());
        assert!(state.take_image_relay().is_none());
        state.history.entries[0].image = Some(picture.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        state.image_preview = Some(Preview {
            cancelled: false,
            id: state.history.entries[0].id,
            picture: picture.clone(),
            texture: None,
            dib: None,
            error: String::new(),
            job: Some(rx),
            ocr_job: None,
            ocr_text: None,
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        state.cancel_image_preview();
        tx.send(Ok((
            picture.thumbnail().unwrap(),
            super::super::media::to_dib(&picture).unwrap(),
        )))
        .unwrap();
        state.poll_image(&ctx);
        assert!(state.image_preview.is_none());
    }
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

    #[test]
    fn composition_handoff_tracks_catalog_version_and_selected_application_names() {
        let mut state = State::default();
        state
            .history
            .insert("first".into(), r"C:\Apps\editor.exe".into())
            .unwrap();
        let first = state.history.entries[0].id;
        state
            .history
            .insert("second".into(), r"C:\Apps\browser.exe".into())
            .unwrap();
        let second = state.history.entries[0].id;
        state.selected = vec![first, second];
        state.output = "first\nsecond".into();

        let (source, text) = state.transfer_text().unwrap();
        assert!(source.contains(&format!("v{}", super::super::catalog_tool_version())));
        assert!(source.contains(&format!("[{first},{second}]")));
        assert!(source.contains("editor.exe、browser.exe"));
        assert!(!source.contains(r"C:\Apps"));
        assert_eq!(text, "first\nsecond");
    }

    #[test]
    fn explicitly_selected_ocr_result_replaces_composition_only_for_handoff() {
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        );
        let mut state = State::default();
        state
            .history
            .insert_image(picture.clone(), "fixture".into())
            .unwrap();
        let id = state.history.entries[0].id;
        state.output = "existing composed text".into();
        state.image_preview = Some(Preview {
            cancelled: false,
            id,
            picture,
            texture: None,
            dib: None,
            error: String::new(),
            job: None,
            ocr_job: None,
            ocr_text: Some("recognized text".into()),
            ocr_error: String::new(),
            ocr_region: Some(OcrRegion {
                x: 1,
                y: 1,
                width: 1,
                height: 1,
            }),
            ocr_drag_start: None,
        });
        #[cfg(windows)]
        {
            state.ocr_language_tag = Some("zh-CN".into());
        }
        assert_eq!(state.transfer_text().unwrap().1, "existing composed text");
        state.handoff_ocr = true;
        let (source, text) = state.transfer_text().unwrap();
        assert!(source.contains("OCR局部"));
        assert!(source.contains(&format!("#{id}")));
        assert!(source.contains(&format!("v{}", super::super::catalog_tool_version())));
        #[cfg(windows)]
        assert!(source.contains("zh-CN"));
        assert!(source.contains("fixture"));
        assert_eq!(text, "recognized text");
        state.history.entries.clear();
        state.poll_image(&egui::Context::default());
        assert!(
            !state.handoff_ocr,
            "a removed source cannot remain selected"
        );
        assert_eq!(state.transfer_text().unwrap().1, "existing composed text");
    }

    #[cfg(windows)]
    #[test]
    fn clipboard_ocr_task_snapshot_contains_only_status_metadata() {
        let mut state = State::default();
        assert!(state.task_snapshot().is_none());

        state.ocr_task.begin();
        let running = state.task_snapshot().unwrap();
        assert_eq!(running.key, "super-clipboard");
        assert_eq!(running.title, "剪贴板图片 OCR");
        assert_eq!(running.phase, crate::tasks::Phase::Running);
        assert!(!running.cancellable);

        state.ocr_task.finish(
            crate::tasks::Phase::Done,
            "识别完成；正文仅保留在剪贴板预览中",
        );
        let done = state.task_snapshot().unwrap();
        assert_eq!(done.phase, crate::tasks::Phase::Done);
        assert_eq!(done.summary, "识别完成；正文仅保留在剪贴板预览中");
        assert!(!done.summary.contains("recognized text"));
        assert!(!done.cancellable);
    }

    #[test]
    fn removed_image_discards_late_ocr_result() {
        let picture = std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(2, 2)).unwrap(),
        );
        let mut state = State::default();
        state
            .history
            .insert_image(picture.clone(), "fixture".into())
            .unwrap();
        let id = state.history.entries[0].id;
        let (tx, rx) = std::sync::mpsc::channel();
        state.image_preview = Some(Preview {
            cancelled: false,
            id,
            picture: picture.clone(),
            texture: None,
            dib: None,
            error: String::new(),
            job: None,
            ocr_job: Some(rx),
            ocr_text: None,
            ocr_error: String::new(),
            ocr_region: None,
            ocr_drag_start: None,
        });
        state.handoff_ocr = true;
        state.history.entries[0].image = Some(std::sync::Arc::new(
            super::super::Picture::from_image(image::DynamicImage::new_rgba8(1, 1)).unwrap(),
        ));
        state.poll_image(&egui::Context::default());
        assert!(state.image_preview.as_ref().unwrap().cancelled);
        assert!(!state.handoff_ocr);
        tx.send(Ok("late text".into())).unwrap();
        state.poll_image(&egui::Context::default());
        assert!(state.image_preview.is_none());
        assert!(state.transfer_text().is_none());
    }
}
