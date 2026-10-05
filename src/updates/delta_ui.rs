use super::size;
use super::{
    delta::{self, Header, Kind},
    delta_files,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Create,
    Rebuild,
}
#[derive(Clone)]
struct Preview {
    header: Header,
    bytes: Arc<[u8]>,
    package: bool,
}
enum Completion {
    Preview(Preview),
    Saved(PathBuf),
}
struct Job {
    receiver: mpsc::Receiver<Result<Completion, String>>,
    cancel: Arc<AtomicBool>,
}
pub struct State {
    mode: Mode,
    source: String,
    target: String,
    source_version: String,
    target_version: String,
    patch: String,
    preview: Option<Preview>,
    pending: Option<Job>,
    unsaved: bool,
    message: String,
    error: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Create,
            source: String::new(),
            target: String::new(),
            source_version: String::new(),
            target_version: env!("CARGO_PKG_VERSION").into(),
            patch: String::new(),
            preview: None,
            pending: None,
            unsaved: false,
            message: String::new(),
            error: false,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(job) = &self.pending {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn has_work(&self) -> bool {
        self.pending.is_some() || self.unsaved
    }
    pub fn discard(&mut self) {
        if let Some(job) = &self.pending {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.preview = None;
        self.unsaved = false;
    }
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        task: impl FnOnce(&AtomicBool) -> Result<Completion, String> + Send + 'static,
    ) {
        if self.pending.is_some() {
            return;
        }
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("update-package".into())
            .spawn(move || {
                let result = task(&worker_cancel);
                let _ = tx.send(result);
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some(Job { receiver, cancel });
                self.message = "后台处理中…".into();
                self.error = false;
            }
            Err(_) => {
                self.message = "无法启动后台处理".into();
                self.error = true;
            }
        }
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let completion = self
            .pending
            .as_ref()
            .and_then(|job| match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("后台任务未完成".into())),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(completion) = completion {
            let cancelled = self
                .pending
                .take()
                .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
            match completion {
                Ok(Completion::Saved(path)) => {
                    self.unsaved = false;
                    self.message = format!("已另存新文件：{}", path.display());
                    self.error = false;
                }
                Ok(Completion::Preview(preview)) if !cancelled => {
                    self.preview = Some(preview);
                    self.unsaved = true;
                    self.message = "预览就绪，内容只在内存中；需要时另存新文件。".into();
                    self.error = false;
                }
                Ok(_) => {
                    self.message = "操作已取消，原有预览保留。".into();
                    self.error = false;
                }
                Err(error) => {
                    self.message = error;
                    self.error = true;
                }
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn start(&mut self, ctx: &egui::Context) {
        let mode = self.mode;
        let source = PathBuf::from(self.source.trim());
        let target = PathBuf::from(self.target.trim());
        let patch = PathBuf::from(self.patch.trim());
        let source_version = self.source_version.trim().to_owned();
        let target_version = self.target_version.trim().to_owned();
        let has_source = !self.source.trim().is_empty();
        self.spawn(ctx, move |cancel| {
            let result = (|| -> anyhow::Result<Preview> {
                if mode == Mode::Create {
                    let source = delta_files::read(&source, false, cancel)?;
                    let target = delta_files::read(&target, false, cancel)?;
                    let (header, bytes) =
                        delta::create(&source, &target, &source_version, &target_version, cancel)?;
                    // Validate generated reconstruction before offering a package for export.
                    let (_, rebuilt) = delta::reconstruct(Some(&source), &bytes, cancel)?;
                    anyhow::ensure!(rebuilt == target, "生成包的自检失败");
                    Ok(Preview {
                        header,
                        bytes: bytes.into(),
                        package: true,
                    })
                } else {
                    let bytes = delta_files::read(&patch, true, cancel)?;
                    let (header, _) = delta::inspect(&bytes, cancel)?;
                    let source = if header.kind == Kind::Delta && has_source {
                        Some(delta_files::read(&source, false, cancel)?)
                    } else {
                        None
                    };
                    let (header, bytes) = delta::reconstruct(source.as_deref(), &bytes, cancel)?;
                    Ok(Preview {
                        header,
                        bytes: bytes.into(),
                        package: false,
                    })
                }
            })();
            result
                .map(Completion::Preview)
                .map_err(|error| error.to_string())
        });
    }
    fn save(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let dialog = rfd::FileDialog::new().set_file_name(if preview.package {
            "update.zidelta"
        } else {
            "reconstructed.exe"
        });
        if let Some(path) = dialog.save_file() {
            self.spawn(ctx, move |cancel| {
                delta_files::save_new(&path, &preview.bytes, cancel)
                    .map(Completion::Saved)
                    .map_err(|error| error.to_string())
            });
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("更新包生成与重建");
        ui.label("本机处理 · 不联网 · 不安装 · 不覆盖原文件");
        ui.label(
            "当前格式未签名：摘要校验只能确认内容一致，不能证明发布者身份。重建文件不会被执行。",
        );
        ui.add_space(12.0);
        let key = (
            self.mode,
            self.source.clone(),
            self.target.clone(),
            self.patch.clone(),
            self.source_version.clone(),
            self.target_version.clone(),
        );
        ui.add_enabled_ui(self.pending.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, Mode::Create, "生成更新包");
                ui.selectable_value(&mut self.mode, Mode::Rebuild, "核对并重建");
            });
            path_row(ui, "源文件", &mut self.source);
            if self.mode == Mode::Create {
                path_row(ui, "目标文件", &mut self.target);
                ui.horizontal(|ui| {
                    ui.label("源版本");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.source_version).desired_width(180.0),
                    );
                    ui.label("目标版本");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.target_version).desired_width(180.0),
                    );
                });
                ui.weak(
                    "版本号为你填写的标签，要求目标 SemVer 更高；不会读取 EXE 或 MSI 的内部版本。",
                );
            } else {
                path_row(ui, "更新包", &mut self.patch);
                ui.weak(
                    "差分包必须提供匹配源文件；完整内容包可不选源文件。单个源／目标最大128 MiB。",
                );
            }
        });
        let changed = key
            != (
                self.mode,
                self.source.clone(),
                self.target.clone(),
                self.patch.clone(),
                self.source_version.clone(),
                self.target_version.clone(),
            );
        if changed {
            self.preview = None;
            self.unsaved = false;
            self.message.clear();
        }
        ui.add_space(10.0);
        let ready = if self.mode == Mode::Create {
            !self.source.trim().is_empty()
                && !self.target.trim().is_empty()
                && !self.source_version.trim().is_empty()
                && !self.target_version.trim().is_empty()
        } else {
            !self.patch.trim().is_empty()
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.pending.is_none() && ready,
                    egui::Button::new(if self.mode == Mode::Create {
                        "生成并自检预览"
                    } else {
                        "校验并重建预览"
                    }),
                )
                .clicked()
            {
                self.start(ctx);
            }
            if self.pending.is_some() {
                ui.spinner();
                if ui.button("取消任务").clicked()
                    && let Some(job) = &self.pending
                {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
            if ui
                .add_enabled(
                    self.pending.is_none() && self.preview.is_some(),
                    egui::Button::new("另存新文件…"),
                )
                .clicked()
            {
                self.save(ctx);
            }
            if ui
                .add_enabled(
                    self.pending.is_none() && self.preview.is_some(),
                    egui::Button::new("清除预览"),
                )
                .clicked()
            {
                self.preview = None;
                self.unsaved = false;
            }
        });
        if !self.message.is_empty() {
            ui.label(egui::RichText::new(&self.message).color(if self.error {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            }));
        }
        if let Some(preview) = &self.preview {
            ui.add_space(12.0);
            ui.separator();
            let h = &preview.header;
            ui.label(
                egui::RichText::new(format!("{} → {}", h.source_version, h.target_version))
                    .strong(),
            );
            ui.label(match h.kind {
                Kind::Delta => "内容：差分复制与新增数据",
                Kind::Full => "内容：压缩完整文件（差分没有更小）",
            });
            ui.label(format!(
                "源 {} · 目标 {} · 当前输出 {}",
                size(h.source_size),
                size(h.target_size),
                size(preview.bytes.len() as u64)
            ));
            ui.label(format!(
                "源块复用 {} · 新数据 {}",
                size(h.copied_bytes),
                size(h.literal_bytes)
            ));
            ui.weak("SHA-256（源 / 目标）");
            ui.label(&h.source_sha256);
            ui.label(&h.target_sha256);
            ui.horizontal(|ui| {
                if ui.small_button("复制源摘要").clicked() {
                    ui.ctx().copy_text(h.source_sha256.clone());
                }
                if ui.small_button("复制目标摘要").clicked() {
                    ui.ctx().copy_text(h.target_sha256.clone());
                }
            });
            ui.label(if preview.package {
                "生成的包已在内存中重建并逐字节核对目标，可另存 .zidelta。"
            } else {
                "重建内容已通过目标长度和 SHA-256 校验，可另存新文件。"
            });
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self, rebuild: bool) {
        self.mode = if rebuild { Mode::Rebuild } else { Mode::Create };
        self.source = "C:\\Example\\previous.exe".into();
        self.target = "C:\\Example\\next.exe".into();
        self.patch = "C:\\Example\\update.zidelta".into();
        self.source_version = "0.81.0".into();
        self.target_version = "0.82.0".into();
        let bytes = (0..65536).map(|i| (i * 17) as u8).collect::<Vec<_>>();
        let mut target = bytes.clone();
        target.extend_from_slice(b"fixture");
        let (header, patch) =
            delta::create(&bytes, &target, "0.81.0", "0.82.0", &AtomicBool::new(false)).unwrap();
        self.preview = Some(Preview {
            header,
            bytes: if rebuild { target.into() } else { patch.into() },
            package: !rebuild,
        });
        self.unsaved = false;
        self.message = "合成界面示例；未读取或保存上方示例路径。".into();
    }
}
fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width((ui.available_width() - 96.0).max(120.0)),
        );
        if ui.button("选择…").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_file()
        {
            *value = path.to_string_lossy().into_owned();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview() -> Preview {
        let (header, bytes) =
            delta::create(b"old", b"new", "1.0.0", "2.0.0", &AtomicBool::new(false)).unwrap();
        Preview {
            header,
            bytes: bytes.into(),
            package: true,
        }
    }
    fn deliver(state: &mut State, result: Result<Completion, String>, cancel: bool) {
        let (tx, receiver) = mpsc::channel();
        tx.send(result).unwrap();
        state.pending = Some(Job {
            receiver,
            cancel: Arc::new(AtomicBool::new(cancel)),
        });
        state.poll(&egui::Context::default());
    }
    #[test]
    fn failure_and_cancelled_late_preview_preserve_work_but_committed_save_is_kept() {
        let mut state = State::default();
        let original = preview();
        state.preview = Some(original.clone());
        state.unsaved = true;
        deliver(&mut state, Err("fixture failure".into()), false);
        assert!(state.has_work());
        assert!(Arc::ptr_eq(
            &state.preview.as_ref().unwrap().bytes,
            &original.bytes
        ));
        deliver(&mut state, Ok(Completion::Preview(preview())), true);
        assert!(state.has_work());
        assert!(Arc::ptr_eq(
            &state.preview.as_ref().unwrap().bytes,
            &original.bytes
        ));
        deliver(
            &mut state,
            Ok(Completion::Saved(PathBuf::from("committed.zidelta"))),
            true,
        );
        assert!(!state.has_work());
        assert!(state.message.contains("committed.zidelta"));
    }
    #[test]
    fn background_task_protects_exit_and_drop_signals_cancellation() {
        let mut state = State::default();
        let (_tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        state.pending = Some(Job {
            receiver,
            cancel: cancel.clone(),
        });
        assert!(state.has_work());
        assert!(state.busy());
        drop(state);
        assert!(cancel.load(Ordering::Relaxed));
    }
}
