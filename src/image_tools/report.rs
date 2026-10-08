//! Frozen, bounded image-information reports; no file read, encoding or implicit writes.
use super::{
    relay::{self, Origin, Source},
    *,
};
use std::{collections::BTreeMap, sync::Weak};

enum Key {
    Image(Weak<DynamicImage>),
    Rgba(Weak<image::RgbaImage>),
    Encoded(Weak<Vec<u8>>),
}
impl Key {
    fn from(source: &Source) -> Self {
        match source {
            Source::Image(v) => Self::Image(Arc::downgrade(v)),
            Source::Rgba(v) => Self::Rgba(Arc::downgrade(v)),
            Source::Encoded(v) => Self::Encoded(Arc::downgrade(v)),
        }
    }
    fn matches(&self, source: &Source) -> bool {
        match (self, source) {
            (Self::Image(old), Source::Image(v)) => {
                old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, v))
            }
            (Self::Rgba(old), Source::Rgba(v)) => {
                old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, v))
            }
            (Self::Encoded(old), Source::Encoded(v)) => {
                old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, v))
            }
            _ => false,
        }
    }
}
struct Record {
    key: Key,
    origins: Vec<Origin>,
    text: String,
}
struct Pending {
    id: &'static str,
    key: Key,
    origins: Vec<Origin>,
    rx: mpsc::Receiver<Result<String, String>>,
}
#[derive(Default)]
pub(super) struct State {
    records: BTreeMap<&'static str, Record>,
    pending: Option<Pending>,
    message: String,
    error: bool,
}
fn build(source: Source, origins: Vec<Origin>, id: &str) -> Result<String> {
    let encoding = match &source {
        Source::Encoded(bytes) => {
            ensure!(bytes.len() <= MAX_OUTPUT_BYTES, "编码结果超限");
            let format = image::guess_format(bytes)?;
            let name = match format {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpeg",
                ImageFormat::WebP => "webp",
                _ => bail!("不支持的编码格式"),
            };
            Some(serde_json::json!({"format": name, "bytes":bytes.len()}))
        }
        _ => None,
    };
    let prepared = relay::prepare(source, origins, id)?;
    let image = &prepared.image;
    let transparent = image.pixels().any(|(_, _, pixel)| pixel.0[3] < 255);
    let text = serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion":1,"material":"image-information","snapshotKind":prepared.kind,
        "width":image.width(),"height":image.height(),
        "pixels":u64::from(image.width())*u64::from(image.height()),
        "hasAlphaChannel":image.color().has_alpha(),"hasTransparentPixels":transparent,
        "encoding":encoding,
        "origins":prepared.origins.iter().map(|o|serde_json::json!({"toolId":o.id,"version":o.version,"utc":o.utc})).collect::<Vec<_>>(),
        "notes":"冻结内存图片信息，不含文件路径。不读取或保存文件，不自动执行接收工具。encoding=null表示没有已有编码结果，不能据此估算最终文件大小。"
    }))?;
    ensure!(text.len() <= 48 * 1024, "图片报告超限");
    Ok(text)
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    fn valid(&self, source: &Source, origins: &[Origin], id: &str) -> Option<&str> {
        self.records
            .get(id)
            .filter(|r| r.key.matches(source) && r.origins == origins)
            .map(|r| r.text.as_str())
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        source: Source,
        origins: Vec<Origin>,
        id: &'static str,
    ) {
        if self.busy() {
            return;
        }
        let key = Key::from(&source);
        let captured = origins.clone();
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        self.pending = Some(Pending {
            id,
            key,
            origins,
            rx,
        });
        self.message = "正在生成冻结图片报告…".into();
        self.error = false;
        std::thread::spawn(move || {
            let result = build(source, captured, id)
                .map_err(|_| "报告准备失败：格式、容量或来源不支持，原工作保留".to_string());
            let _ = tx.send(result);
            wake.request_repaint();
        });
    }
    fn poll(&mut self, ctx: &egui::Context, current: Option<(Source, Vec<Origin>, &'static str)>) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let result = match pending.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("报告线程结束，原工作保留".into()),
        };
        let pending = self.pending.take().unwrap();
        if !current.as_ref().is_some_and(|(source, origins, id)| {
            *id == pending.id && pending.key.matches(source) && *origins == pending.origins
        }) {
            self.message = "图片或工具已改变，已忽略过期报告；原工作保留。".into();
            self.error = false;
            return;
        }
        match result {
            Ok(text) => {
                self.records.insert(
                    pending.id,
                    Record {
                        key: pending.key,
                        origins: pending.origins,
                        text,
                    },
                );
                self.message = "报告已生成，可通过顶部“发送结果到工具…”继续处理。".into();
                self.error = false
            }
            Err(error) => {
                self.message = error;
                self.error = true
            }
        }
    }
}
impl super::State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_batch_report_fixture(&mut self) {
        self.batch.preview_fixture();
        self.mode = Mode::Batch;
    }

    #[cfg(feature = "ui-preview")]
    pub fn preview_image_report_assert(&self) -> String {
        let text = self.image_report().expect("current image report");
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        if self.mode == Mode::Batch {
            assert_eq!(value["material"], "image-batch-report");
            assert_eq!(value["phase"], "preflight");
            assert_eq!(value["counts"]["total"], 3);
            assert_eq!(value["counts"]["saved"], 0);
            assert!(value["items"][0]["outputBytes"].is_null());
            assert!(!text.contains("Pictures"));
            return text.to_owned();
        }
        assert_eq!(value["width"], 720);
        assert_eq!(value["height"], 405);
        assert_eq!(value["encoding"]["format"], "jpeg");
        assert_eq!(
            value["encoding"]["bytes"],
            self.encoded.as_ref().unwrap().len()
        );
        assert_eq!(value["hasTransparentPixels"], false);
        assert!(!text.contains("Pictures") && !text.contains("sample.png"));
        text.to_string()
    }
    pub(crate) fn image_report(&self) -> Option<&str> {
        if self.mode == Mode::Batch {
            return self.batch.report();
        }
        let (source, origins, id) = self.relay_source()?;
        self.reports.valid(&source, &origins, id)
    }
    pub(super) fn poll_image_report(&mut self, ctx: &egui::Context) {
        let current = self.relay_source();
        self.reports.poll(ctx, current);
    }
    pub(super) fn image_report_ui(&mut self, ui: &mut egui::Ui) {
        if self.mode == Mode::Batch {
            self.batch.report_ui(ui);
            return;
        }
        self.poll_image_report(ui.ctx());
        let source = self.relay_source();
        let available = source.is_some();
        ui.horizontal_wrapped(|ui| {
            let response = ui.add_enabled(
                source.is_some() && !self.reports.busy() && !self.relay_active(),
                egui::Button::new("生成图片信息报告"),
            );
            #[cfg(feature = "ui-preview")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("image-report-generate"), response.rect)
            });
            if response.clicked()
                && let Some((source, origins, id)) = source
            {
                self.reports.start(ui.ctx(), source, origins, id)
            }
            ui.small(if available {
                "尺寸、透明度与已有编码大小 → JSON / 备忘等；不自动保存。"
            } else {
                "单图、已生成编辑结果或截图选区支持报告；请先准备图片。"
            });
        });
        if self.reports.error {
            ui.colored_label(ui.visuals().error_fg_color, &self.reports.message);
        } else if !self.reports.message.is_empty() {
            ui.small(&self.reports.message);
        }
        let current = self.image_report().is_some();
        if let Some(record) = self.reports.records.get(self.active_tool_id()) {
            if !current {
                ui.small("图片已改变，旧报告保留但不可发送；请重新生成。");
            }
            egui::CollapsingHeader::new(if current {
                "图片信息 JSON 报告"
            } else {
                "旧图片信息 JSON 报告（非当前图片）"
            })
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(170.0)
                    .show(ui, |ui| {
                        ui.monospace(&record.text);
                    });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoded_report_uses_actual_dimensions_size_and_alpha_without_paths() {
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            7,
            5,
            image::Rgba([1, 2, 3, 0]),
        ));
        for (format, name) in [(ImageFormat::Png, "png"), (ImageFormat::WebP, "webp")] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            let bytes = bytes.into_inner();
            let size = bytes.len();
            let report: serde_json::Value = serde_json::from_str(
                &build(Source::Encoded(Arc::new(bytes)), vec![], "image-tools").unwrap(),
            )
            .unwrap();
            assert_eq!(report["width"], 7);
            assert_eq!(report["height"], 5);
            assert_eq!(report["encoding"]["bytes"], size);
            assert_eq!(report["encoding"]["format"], name);
            assert_eq!(report["hasTransparentPixels"], true);
            assert_eq!(report["origins"][0]["toolId"], "image-tools");
            assert!(report.get("path").is_none());
        }
    }
    #[test]
    fn unencoded_selection_never_estimates_size_and_bad_data_is_rejected() {
        let source = Source::Rgba(Arc::new(image::RgbaImage::from_pixel(
            4,
            3,
            image::Rgba([1, 2, 3, 255]),
        )));
        let report: serde_json::Value =
            serde_json::from_str(&build(source, vec![], "screenshot-workbench").unwrap()).unwrap();
        assert!(report["encoding"].is_null());
        assert_eq!(report["pixels"], 12);
        assert_eq!(report["hasTransparentPixels"], false);
        assert!(
            build(
                Source::Encoded(Arc::new(vec![1, 2, 3])),
                vec![],
                "image-tools"
            )
            .is_err()
        );
    }
    #[test]
    fn stale_delivery_and_expired_keys_preserve_completed_report() {
        let ctx = egui::Context::default();
        let image = Arc::new(DynamicImage::new_rgba8(2, 2));
        let source = Source::Image(image.clone());
        let mut state = State::default();
        state.records.insert(
            "image-tools",
            Record {
                key: Key::from(&source),
                origins: vec![],
                text: "old".into(),
            },
        );
        let (tx, rx) = mpsc::channel();
        tx.send(Ok("new".into())).unwrap();
        state.pending = Some(Pending {
            id: "image-tools",
            key: Key::from(&source),
            origins: vec![],
            rx,
        });
        state.poll(&ctx, Some((source.clone(), vec![], "screenshot-workbench")));
        assert_eq!(state.valid(&source, &[], "image-tools"), Some("old"));
        let (tx, rx) = mpsc::channel();
        tx.send(Err("fixture failure".into())).unwrap();
        state.pending = Some(Pending {
            id: "image-tools",
            key: Key::from(&source),
            origins: vec![],
            rx,
        });
        state.poll(&ctx, Some((source.clone(), vec![], "image-tools")));
        assert_eq!(state.valid(&source, &[], "image-tools"), Some("old"));
        assert!(state.error && !state.busy());
        let replacement = Source::Image(Arc::new(DynamicImage::new_rgba8(2, 2)));
        assert!(state.valid(&replacement, &[], "image-tools").is_none());
        drop(source);
        drop(image);
        let Source::Image(v) = replacement else {
            unreachable!()
        };
        assert!(!state.records["image-tools"].key.matches(&Source::Image(v)));
    }
}
