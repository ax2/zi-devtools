//! Explicit, bounded UTF-8 file exchange. Markdown stays plain source text.
use super::*;
use anyhow::Context;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Clone)]
pub(super) struct Export {
    pub title: String,
    pub body: String,
    pub filename: String,
}

const MAX_BULK_FILES: usize = 32;
const MAX_BULK_BYTES: usize = 1024 * 1024;

pub(super) struct BatchImportReview {
    pub expected: Vec<Item>,
    pub records: Vec<Item>,
    pub total_bytes: usize,
}

#[derive(Clone)]
pub(super) struct BatchExportRecord {
    pub title: String,
    pub body: String,
    pub filename: String,
}

pub(super) struct BatchExportReview {
    pub records: Vec<BatchExportRecord>,
    pub total_bytes: usize,
}

fn batch_export_review(items: &[Item]) -> Result<BatchExportReview> {
    ensure!(!items.is_empty(), "当前筛选没有可导出的备忘");
    ensure!(
        items.len() <= MAX_BULK_FILES,
        "一次最多导出 32 条，请缩小搜索范围"
    );
    let mut total_bytes = 0usize;
    let records = items
        .iter()
        .map(|item| {
            item.validate()?;
            ensure!(
                item.revision > 0 && item.schedule.is_none() && !item.trash,
                "批量导出只接受已保存的正常备忘"
            );
            total_bytes = total_bytes
                .checked_add(item.body.len())
                .context("导出正文总量溢出")?;
            ensure!(
                total_bytes <= MAX_BULK_BYTES,
                "当前结果正文总量超过 1 MiB，请缩小搜索范围"
            );
            let mut stem: String = item
                .title
                .chars()
                .take(40)
                .map(|c| {
                    if c.is_control() || "<>:\"/\\|?*".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            stem = stem.trim().trim_end_matches('.').to_owned();
            if stem.is_empty() {
                stem = "未命名备忘".into();
            }
            let suffix: String = item
                .id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect();
            let suffix = if suffix.is_empty() {
                "memo".into()
            } else {
                suffix
            };
            Ok(BatchExportRecord {
                title: item.title.clone(),
                body: item.body.clone(),
                filename: format!("{stem}-{suffix}.md"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut filenames = HashSet::new();
    ensure!(
        records
            .iter()
            .all(|record| filenames.insert(record.filename.to_lowercase())),
        "导出文件名冲突，请修改标题或记录标识后重试"
    );
    Ok(BatchExportReview {
        records,
        total_bytes,
    })
}

fn write_batch(parent: &Path, review: &BatchExportReview) -> Result<(PathBuf, usize, usize)> {
    ensure!(
        review.records.len() <= MAX_BULK_FILES && !review.records.is_empty(),
        "批量导出记录数量无效"
    );
    ensure!(review.total_bytes <= MAX_BULK_BYTES, "批量导出超过 1 MiB");
    let meta = std::fs::symlink_metadata(parent).context("无法读取所选目标文件夹")?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "请选择普通文件夹，不能是文件或符号链接"
    );
    let name = format!(
        "Zi-备忘导出-{}-{}",
        Local::now().format("%Y%m%d-%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let stage = parent.join(format!(".{name}.partial"));
    std::fs::create_dir(&stage).context("无法创建新的批量导出暂存目录")?;
    for record in &review.records {
        let target = stage.join(&record.filename);
        if let Err(error) = write_note(&target, &record.body) {
            anyhow::bail!(
                "批量导出未完成；已写入的暂存文件保留在 {}，请检查后自行处理：{error:#}",
                stage.display()
            );
        }
    }
    let output = parent.join(&name);
    std::fs::rename(&stage, &output).with_context(|| {
        format!(
            "批量文件已写入暂存目录 {}，但无法发布到目标目录；请检查并手动处理",
            stage.display()
        )
    })?;
    Ok((output, review.records.len(), review.total_bytes))
}

pub(super) fn read_notes(paths: &[PathBuf]) -> Result<(Vec<Item>, usize)> {
    ensure!(!paths.is_empty(), "请选择要导入的文件");
    ensure!(paths.len() <= MAX_BULK_FILES, "一次最多选择 32 个文件");
    let mut records = Vec::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for path in paths {
        let item = read_note(path)?;
        total_bytes = total_bytes
            .checked_add(item.body.len())
            .context("导入文件总量溢出")?;
        ensure!(
            total_bytes <= MAX_BULK_BYTES,
            "所选文件正文总量超过 1 MiB，请分批导入"
        );
        records.push(item);
    }
    Ok((records, total_bytes))
}

fn text_extension(path: &Path) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    ensure!(
        ["md", "markdown", "txt"]
            .iter()
            .any(|s| extension.eq_ignore_ascii_case(s)),
        "请选择 .md、.markdown 或 .txt 文件"
    );
    Ok(())
}

pub(super) fn read_note(path: &Path) -> Result<Item> {
    text_extension(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "请选择普通文本文件，不能是目录或符号链接"
    );
    ensure!(
        metadata.len() <= (MAX_BODY + 3) as u64,
        "文件超过 128 KiB，请先缩小内容"
    );
    let file = File::open(path).context("无法读取所选文件")?;
    ensure!(file.metadata()?.is_file(), "所选路径不再是普通文件");
    let mut bytes = Vec::new();
    file.take((MAX_BODY + 4) as u64).read_to_end(&mut bytes)?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    ensure!(bytes.len() <= MAX_BODY, "文件超过 128 KiB，请先缩小内容");
    let body =
        std::str::from_utf8(bytes).context("文件不是 UTF-8；请先用文件编码工具转换后导入")?;
    ensure!(!body.contains('\0'), "文件包含空字节，请确认是文本文件");
    let mut item = Item::new(None);
    item.title = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>()
        .trim()
        .to_owned();
    if item.title.is_empty() {
        item.title = "导入的备忘".into();
    }
    item.body = body.to_owned();
    item.validate()?;
    Ok(item)
}

pub(super) fn write_note(path: &Path, body: &str) -> Result<usize> {
    text_extension(path)?;
    ensure!(body.len() <= MAX_BODY, "正文超过 128 KiB，请先缩小内容");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("无法创建导出文件；已有文件不会覆盖，请使用新文件名并检查目录权限")?;
    if let Err(error) = file
        .write_all(body.as_bytes())
        .and_then(|_| file.sync_all())
    {
        // Do not retry or truncate. A failed write may have left a partial new file.
        return Err(error).context("导出未完成，目标可能包含部分正文；请检查后选择新的文件名重试");
    }
    Ok(body.len())
}

impl State {
    pub(super) fn finish_import(&mut self, item: Item) {
        self.edit(item);
        self.calendar = false;
        self.trash = false;
        self.query.clear();
        self.message = "文件已读入备忘草稿，原文件不变；确认正文后点击保存到本机。".into();
        self.error = false;
    }
    pub(super) fn import_file(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(!self.has_unsaved(), "请先保存或放弃当前编辑，再导入文件");
        ensure!(
            self.purge_review.is_none()
                && self.export_review.is_none()
                && self.backup_review.is_none()
                && self.ics_review.is_none(),
            "请先关闭当前确认窗口"
        );
        self.start_file(move || read_note(&path).map(Reply::Imported));
        Ok(())
    }
    pub(super) fn import_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.backup_available()?;
        ensure!(!paths.is_empty(), "请选择要导入的文件");
        ensure!(paths.len() <= MAX_BULK_FILES, "一次最多选择 32 个文件");
        let database = self.path.clone();
        self.start_file(move || {
            let (records, total_bytes) = read_notes(&paths)?;
            let expected = store::load(&database)?;
            Ok(Reply::BatchImportReady(Box::new(BatchImportReview {
                expected,
                records,
                total_bytes,
            })))
        });
        Ok(())
    }
    pub(super) fn confirm_batch_import(&mut self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none() && !self.has_unsaved(),
            "正在读写记录或有未保存编辑，请稍后重试"
        );
        let Some(backup::Review::Import(review)) = self.backup_review.as_ref() else {
            anyhow::bail!("请先预览批量导入内容");
        };
        let expected = review.expected.clone();
        let incoming = review.records.clone();
        let path = self.path.clone();
        self.backup_review = None;
        self.start_file(move || {
            store::insert_batch(&path, &expected, &incoming).map(Reply::BatchImported)
        });
        Ok(())
    }
    pub(super) fn review_batch_export(&mut self) -> Result<()> {
        self.backup_available()?;
        ensure!(!self.calendar && !self.trash, "请切换到备忘列表后批量导出");
        let items = self
            .listed_indices()
            .into_iter()
            .map(|index| self.items[index].clone())
            .collect::<Vec<_>>();
        self.backup_review = Some(backup::Review::BatchExport(batch_export_review(&items)?));
        Ok(())
    }
    pub(super) fn export_batch(&mut self, parent: PathBuf) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none() && !self.has_unsaved(),
            "正在读写记录或有未保存编辑，请稍后重试"
        );
        let Some(backup::Review::BatchExport(review)) = self.backup_review.as_ref() else {
            anyhow::bail!("请先预览批量导出内容");
        };
        let review = BatchExportReview {
            records: review.records.clone(),
            total_bytes: review.total_bytes,
        };
        self.backup_review = None;
        self.start_file(move || {
            write_batch(&parent, &review)
                .map(|(path, count, bytes)| Reply::BatchExported(path, count, bytes))
        });
        Ok(())
    }
    pub(super) fn start_file(&mut self, work: impl FnOnce() -> Result<Reply> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.file_operation = true;
        self.message = "正在处理所选文件…".into();
        self.error = false;
        std::thread::spawn(move || {
            let _ = tx.send(work().map_err(|e| format!("{e:#}")));
        });
    }
    pub(super) fn review_export(&mut self) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        ensure!(
            self.purge_review.is_none()
                && self.backup_review.is_none()
                && self.ics_review.is_none(),
            "请先关闭其他确认窗口"
        );
        let item = self.draft.as_ref().context("请先打开备忘或日程")?;
        ensure!(!item.trash, "请先从回收站恢复记录，再导出正文");
        ensure!(
            item.body.len() <= MAX_BODY,
            "正文超过 128 KiB，请先缩小内容"
        );
        let stem: String = item
            .title
            .chars()
            .take(60)
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        self.export_review = Some(Export {
            title: item.title.clone(),
            body: item.body.clone(),
            filename: format!("{}-正文.md", stem.trim().trim_end_matches('.')),
        });
        Ok(())
    }
    pub(super) fn export_file(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            self.loaded && self.pending.is_none(),
            "正在读写本地记录，请稍后重试"
        );
        let reviewed = self.export_review.as_ref().context("请先预览导出正文")?;
        // The chosen path is explicit consent to export this reviewed snapshot.
        text_extension(&path)?;
        let body = reviewed.body.clone();
        self.export_review = None;
        self.start_file(move || write_note(&path, &body).map(|bytes| Reply::Exported(path, bytes)));
        Ok(())
    }
    pub(super) fn file_toolbar(&mut self, ui: &mut egui::Ui) {
        if self.calendar {
            self.ics_buttons(ui);
        } else {
            ui.horizontal_wrapped(|ui| {
                #[cfg(windows)]
                if ui
                    .add_enabled(
                        self.loaded && self.pending.is_none() && !self.has_unsaved(),
                        egui::Button::new("导入 Markdown / 文本…"),
                    )
                    .on_hover_text(
                        "读取 UTF-8 文件为新备忘草稿，最多 128 KiB；不修改原文件，也不自动保存。",
                    )
                    .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Markdown / 文本", &["md", "markdown", "txt"])
                        .pick_file()
                    && let Err(error) = self.import_file(path)
                {
                    self.message = format!("{error:#}");
                    self.error = true;
                }
                #[cfg(windows)]
                if ui
                    .add_enabled(
                        self.loaded
                            && self.pending.is_none()
                            && !self.has_unsaved()
                            && !self.trash
                            && !self.listed_indices().is_empty(),
                        egui::Button::new("批量导出当前结果…"),
                    )
                    .on_hover_text("导出当前搜索/置顶筛选结果中的备忘；预览后写入所选文件夹下的新子文件夹，不覆盖已有内容。")
                    .clicked()
                    && let Err(error) = self.review_batch_export()
                {
                    self.message = format!("{error:#}");
                    self.error = true;
                }
                #[cfg(windows)]
                if ui
                    .add_enabled(
                        self.loaded && self.pending.is_none() && !self.has_unsaved(),
                        egui::Button::new("批量导入…"),
                    )
                    .on_hover_text(
                        "多选最多 32 个 UTF-8 Markdown / 文本文件，预览后一次性保存为备忘。",
                    )
                    .clicked()
                    && let Some(paths) = rfd::FileDialog::new()
                        .add_filter("Markdown / 文本", &["md", "markdown", "txt"])
                        .pick_files()
                    && let Err(error) = self.import_files(paths)
                {
                    self.message = format!("{error:#}");
                    self.error = true;
                }
                ui.small(if self.pending.is_none() && self.has_unsaved() {
                    "先保存或放弃当前编辑，才能导入新备忘。"
                } else {
                    "文件 → 新备忘草稿 · UTF-8 · 最大 128 KiB"
                });
            });
        }
        self.backup_buttons(ui);
    }
    pub(super) fn export_ui(&mut self, ctx: &egui::Context) {
        let Some(reviewed) = self.export_review.as_ref() else {
            return;
        };
        let mut cancel = false;
        #[cfg(feature = "ui-preview")]
        let mut cancel_rect = None;
        #[cfg(windows)]
        let mut save = false;
        egui::Modal::new(egui::Id::new("planner-export")).show(ctx, |ui| {
            ui.set_width(480.0_f32.min((ctx.screen_rect().width() - 48.0).max(180.0)));
            ui.heading("导出正文");
            ui.label(format!("{} · {} 字节", reviewed.title, reviewed.body.len()));
            ui.label("导出下面的正文快照，包含未保存的编辑。标题、日程日期和提醒设置不写入文件；这不是完整备份。");
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("export-body").max_height(220.0).show(ui, |ui| { ui.monospace(&reviewed.body); });
            ui.add_space(8.0);
            ui.small("UTF-8，不添加标题或 BOM，不转换换行；只创建新文件，已有文件不会覆盖。");
            ui.horizontal_wrapped(|ui| {
                let response = ui.button("取消");
                cancel = response.clicked();
                #[cfg(feature = "ui-preview")]
                { cancel_rect = Some(response.rect); }
                #[cfg(windows)]
                { save = ui.button("选择路径并导出…").clicked(); }
            });
        });
        #[cfg(feature = "ui-preview")]
        {
            self.preview_export_cancel_rect = cancel_rect;
        }
        cancel |= ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel {
            self.export_review = None;
        }
        #[cfg(windows)]
        if !cancel && save {
            let name = self.export_review.as_ref().unwrap().filename.clone();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Markdown / 文本", &["md", "markdown", "txt"])
                .set_file_name(name)
                .save_file()
                && let Err(error) = self.export_file(path)
            {
                self.export_review = None;
                self.message = format!("{error:#}");
                self.error = true;
            }
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_file_exchange(&mut self, export: bool) {
        self.preview(false, false);
        let mut item = Item::new(None);
        item.title = "本周项目计划".into();
        item.body = "# 本周项目计划\n\n- 完善备忘与日程\n- 将诊断结果保存为备忘\n- 导出 Markdown，在编辑器中继续整理\n\n## 资料\n\n正文保留原始 Markdown，不自动执行链接或脚本。\n".into();
        self.finish_import(item);
        if export {
            self.review_export().unwrap();
        }
        self.focus_editor = false;
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_files_smoke(&mut self, phase: u8) -> bool {
        match phase {
            0 => {
                self.preview(false, false);
                self.path = std::env::temp_dir()
                    .join(format!("zi-planner-files-ui-{}", uuid::Uuid::new_v4()))
                    .join("planner.sqlite3");
                std::fs::create_dir_all(self.path.parent().unwrap()).unwrap();
                let source = self.path.with_file_name("输入.md");
                std::fs::write(&source, "# 本机资料\r\n\n中文 😀").unwrap();
                self.import_file(source).unwrap();
            }
            1 => {
                assert!(self.pending.is_none() && !self.error && self.has_unsaved());
                assert_eq!(self.draft.as_ref().unwrap().title, "输入");
                self.review_export().unwrap();
            }
            2 => {
                assert!(self.export_review.is_none());
                assert!(!self.path.with_file_name("输出.md").exists());
                self.review_export().unwrap();
                self.export_file(self.path.with_file_name("输出.md"))
                    .unwrap();
            }
            3 => {
                if self.pending.is_some() {
                    return false;
                }
                assert!(!self.error && self.has_unsaved() && !self.path.exists());
                let source = self.path.with_file_name("输入.md");
                let target = self.path.with_file_name("输出.md");
                assert_eq!(
                    std::fs::read_to_string(&source).unwrap(),
                    "# 本机资料\r\n\n中文 😀"
                );
                assert_eq!(
                    std::fs::read(&source).unwrap(),
                    std::fs::read(&target).unwrap()
                );
                assert_eq!(self.draft.as_ref().unwrap().body, "# 本机资料\r\n\n中文 😀");
                std::fs::remove_file(source).unwrap();
                std::fs::remove_file(target).unwrap();
                std::fs::remove_dir(self.path.parent().unwrap()).unwrap();
                println!(
                    "PASS planner file exchange: native app async import, actual preview cancel, exact UTF-8 export, source and unsaved draft preserved; no database write"
                );
            }
            _ => unreachable!(),
        }
        true
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_import(&mut self) {
        self.preview(false, false);
        let mut first = Item::new(None);
        first.title = "项目计划.md".into();
        first.body = "# 本周计划\n\n- 完成复核\n- 整理资料".into();
        let mut second = Item::new(None);
        second.title = "会议纪要.txt".into();
        second.body = "讨论了本机保存与跨工具检索。".into();
        self.backup_review = Some(backup::Review::Import(BatchImportReview {
            expected: self.items.clone(),
            total_bytes: first.body.len() + second.body.len(),
            records: vec![first, second],
        }));
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_export(&mut self) {
        self.preview(false, false);
        self.review_batch_export()
            .expect("preview fixture has bounded saved memos");
    }
}
