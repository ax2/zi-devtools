use super::History;
use eframe::egui::{self, RichText};

#[derive(Default)]
pub struct State {
    history: History,
    images_enabled: bool,
    image_preview: Option<Preview>,
    image_relay: Option<std::sync::Arc<super::Picture>>,
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
type PreviewReceipt = Result<(egui::ColorImage, Vec<u8>), String>;
struct Preview {
    cancelled: bool,
    id: u64,
    picture: std::sync::Arc<super::Picture>,
    texture: Option<egui::TextureHandle>,
    dib: Option<Vec<u8>>,
    error: String,
    job: Option<std::sync::mpsc::Receiver<PreviewReceipt>>,
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
        egui::CollapsingHeader::new("应用排除 · 采集前过滤").default_open(self.image_preview.is_none()).show(ui, |ui| {
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
            ui.small("保存包含文本、图片、来源、时间和置顶状态，保护范围为当前Windows用户；同用户程序仍可能访问。不开启云同步。");
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
    }
    fn cancel_image_preview(&mut self) {
        if self.image_preview.as_ref().is_some_and(|p| p.job.is_some()) {
            self.image_preview.as_mut().unwrap().cancelled = true;
        } else {
            self.image_preview = None;
        }
    }
    fn open_image(&mut self, ctx: &egui::Context, id: u64) {
        if self.image_preview.as_ref().is_some_and(|p| p.job.is_some()) {
            self.message = "图片预览准备中，请等待完成或取消".into();
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
        });
    }
    fn poll_image(&mut self, ctx: &egui::Context) {
        let Some(preview) = &mut self.image_preview else {
            return;
        };
        if !self.history.entries.iter().any(|entry| {
            entry.id == preview.id
                && entry
                    .image
                    .as_ref()
                    .is_some_and(|image| std::sync::Arc::ptr_eq(image, &preview.picture))
        }) {
            if preview.job.is_some() {
                preview.cancelled = true;
            } else {
                self.image_preview = None;
                return;
            }
        }
        let Some(job) = &preview.job else { return };
        let result = match job.try_recv() {
            Ok(value) => value,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(_) => Err("图片准备线程结束".into()),
        };
        preview.job = None;
        if preview.cancelled {
            self.image_preview = None;
            return;
        }
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
        };
    }
    pub fn take_image_relay(&mut self) -> Option<std::sync::Arc<super::Picture>> {
        self.image_relay.take()
    }
    fn image_ui(&mut self, ui: &mut egui::Ui) {
        let Some(preview) = &self.image_preview else {
            return;
        };
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
        if let Some(texture) = &preview.texture {
            let size = texture.size_vec2();
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
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
        }
        if !preview.error.is_empty() {
            ui.label(&preview.error);
        }
        let mut close = false;
        let mut copy = false;
        let mut relay = false;
        let mut save = false;
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
        if close {
            if self.image_preview.as_ref().is_some_and(|p| p.job.is_some()) {
                self.image_preview.as_mut().unwrap().cancelled = true;
            } else {
                self.image_preview = None;
            }
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
        ui.label("v0.5.0 · 开发中：文本与图片历史 / 搜索与置顶 / 组合与接力");
        ui.label("主动开启后采集新复制的文本；可另行开启本机保护保存，重启恢复历史但不自动采集。图片采集需另外开启；富文本与文件引用继续开发。");
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
        ui.small("当前最多500条，文本单条1MiB；图片最多400万像素、PNG16MiB，总32MiB同时计编码与RGBA容量。来源可能未知；应用排除见上方；不能保证识别敏感内容，必要时请暂停。默认不采集、不联网；本机保存须主动开启。");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("搜索文本或来源应用"));
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        ui.separator();
        let query = self.search.to_lowercase();
        let mut pin_changed = false;
        let mut copy = None;
        let mut delete = None;
        let mut open_image = None;
        egui::ScrollArea::vertical()
            .id_salt("clipboard-history-list")
            .max_height(if self.image_preview.is_some() {180.0}else{340.0})
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
                            if ui.add_enabled(entry.image.is_none(),egui::Checkbox::new(&mut selected, "选择文本")).changed() {
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
                            if let Some(image)=&entry.image {
                                ui.small(format!("图片 · {}×{} · PNG {:.1}KiB",image.width,image.height,image.png.len() as f64/1048576.0));
                                if ui.button("查看图片").clicked(){open_image=Some(entry.id);}
                            }
                            if entry.image.is_none() && ui.button("复制").clicked() {
                                copy = Some(entry.text.clone());
                            }
                            if ui.button("删除").clicked() {
                                delete = Some(entry.id);
                            }
                        });
                        if entry.image.is_none() {
                        let preview: String = entry.text.chars().take(180).collect();
                        ui.label(preview);
                        if entry.text.chars().count() > 180 {
                            ui.small("长文本已截断；下方组合预览可查看完整结果");
                        }
                        }
                    });
                }
            });
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
        if ui.button("载入合成图片示例（不读取系统剪贴板）").clicked() {
            self.add_picture_example(ui.ctx());
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
                format!("超级剪贴板0.5.0组合 · 条目{:?}", self.selected),
                &self.output,
            ))
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
}
