//! Typed, bounded in-memory image relay. No temporary files or implicit encoding.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub id: String,
    pub version: String,
    pub utc: String,
}
#[derive(Clone)]
pub(crate) enum Source {
    Image(Arc<DynamicImage>),
    Rgba(Arc<image::RgbaImage>),
    Encoded(Arc<Vec<u8>>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Convert,
    Edit,
}
impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::Convert => "图片转换",
            Self::Edit => "裁剪与标注",
        }
    }
}
pub(crate) struct Prepared {
    pub image: Arc<DynamicImage>,
    pub thumbnail: egui::ColorImage,
    pub origins: Vec<Origin>,
    pub kind: &'static str,
}
fn check(image: &DynamicImage) -> Result<()> {
    ensure!(
        image.width() > 0
            && image.height() > 0
            && image.width() <= 12000
            && image.height() <= 12000
            && u64::from(image.width()) * u64::from(image.height()) <= MAX_PIXELS,
        "接力图片尺寸超限"
    );
    ensure!(
        image.as_bytes().len() <= 192 * 1024 * 1024,
        "接力图像内存超限"
    );
    Ok(())
}
pub(super) fn origin(id: &str) -> Result<Origin> {
    let catalog: serde_json::Value = serde_json::from_str(include_str!("../../docs/tools.json"))?;
    let version = catalog["tools"]
        .as_array()
        .and_then(|tools| tools.iter().find(|tool| tool["id"].as_str() == Some(id)))
        .and_then(|tool| tool["tool_version"].as_str())
        .ok_or_else(|| anyhow::anyhow!("来源工具版本未知，未建立接力"))?;
    Ok(Origin {
        id: id.into(),
        version: version.into(),
        utc: chrono::Utc::now().to_rfc3339(),
    })
}
pub(super) fn prepare(source: Source, mut origins: Vec<Origin>, id: &str) -> Result<Prepared> {
    ensure!(
        origins.len() < 128,
        "接力链已达128步，请保留结果后建立新的链"
    );
    ensure!(
        origins
            .iter()
            .all(|o| o.id.len() <= 128 && o.version.len() <= 64 && o.utc.len() <= 64),
        "来源链字段超限"
    );
    let (image, kind) = match source {
        Source::Image(image) => {
            check(&image)?;
            (image, "原图")
        }
        Source::Rgba(image) => {
            ensure!(
                image.width() > 0
                    && image.height() > 0
                    && u64::from(image.width()) * u64::from(image.height()) <= MAX_PIXELS,
                "接力截图尺寸超限"
            );
            (
                Arc::new(DynamicImage::ImageRgba8((*image).clone())),
                "截图选区",
            )
        }
        Source::Encoded(bytes) => {
            ensure!(
                !bytes.is_empty() && bytes.len() <= MAX_OUTPUT_BYTES,
                "编码结果字节数超限"
            );
            let mut reader =
                ImageReader::new(Cursor::new(bytes.as_slice())).with_guessed_format()?;
            ensure!(
                matches!(
                    reader.format(),
                    Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
                ),
                "接力仅支持PNG/JPEG/WebP编码结果"
            );
            reader.limits(image_limits());
            let (width, height) = reader.into_dimensions()?;
            ensure!(
                width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
                "编码结果像素数超限"
            );
            let mut reader =
                ImageReader::new(Cursor::new(bytes.as_slice())).with_guessed_format()?;
            reader.limits(image_limits());
            (Arc::new(reader.decode()?), "当前编码结果")
        }
    };
    check(&image)?;
    origins.push(origin(id)?);
    let thumbnail = preview_image(&image);
    Ok(Prepared {
        image,
        thumbnail,
        origins,
        kind,
    })
}
pub(super) struct Transfer {
    pub source_id: &'static str,
    pub target: Target,
    pub replace: bool,
    pub cancelled: bool,
    pub error: String,
    pub prepared: Option<Prepared>,
    pub texture: Option<egui::TextureHandle>,
    receiver: Option<mpsc::Receiver<Result<Prepared, String>>>,
}
impl Transfer {
    pub fn start(
        ctx: &egui::Context,
        source: Source,
        origins: Vec<Origin>,
        id: &'static str,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let result = prepare(source, origins, id)
                .map_err(|_| "图片接力准备失败：格式、容量或来源不支持；原工作保留".to_string());
            let _ = tx.send(result);
            wake.request_repaint();
        });
        Self {
            source_id: id,
            target: if id == "image-tools" {
                Target::Edit
            } else {
                Target::Convert
            },
            replace: false,
            cancelled: false,
            error: String::new(),
            prepared: None,
            texture: None,
            receiver: Some(rx),
        }
    }
    pub fn busy(&self) -> bool {
        self.receiver.is_some()
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.receiver else { return };
        let result = match rx.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err("图片准备线程结束，原工作保留".into()),
        };
        self.receiver = None;
        if self.cancelled {
            return;
        }
        match result {
            Ok(value) => {
                self.texture = Some(ctx.load_texture(
                    "image-relay-preview",
                    value.thumbnail.clone(),
                    egui::TextureOptions::LINEAR,
                ));
                self.prepared = Some(value)
            }
            Err(error) => self.error = error,
        }
    }
}
impl State {
    pub fn start_clipboard_relay(&mut self, ctx: &egui::Context, png: Arc<Vec<u8>>) -> Result<()> {
        ensure!(self.relay.is_none(), "已有图片接力，请先确认或取消");
        self.relay = Some(Transfer::start(
            ctx,
            Source::Encoded(png),
            Vec::new(),
            "super-clipboard",
        ));
        Ok(())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_clipboard_relay_apply(
        &mut self,
        ctx: &egui::Context,
        png: &Arc<Vec<u8>>,
    ) -> bool {
        let Some(transfer) = self.relay.as_mut() else {
            return false;
        };
        transfer.poll(ctx);
        let Some(prepared) = &transfer.prepared else {
            return false;
        };
        assert_eq!(transfer.source_id, "super-clipboard");
        assert_eq!(prepared.origins[0].version, "0.5.0");
        let original = image::load_from_memory_with_format(png, ImageFormat::Png).unwrap();
        assert_eq!(prepared.image.to_rgba8(), original.to_rgba8());
        self.apply_relay(ctx).unwrap();
        assert_eq!(self.mode, Mode::Single);
        assert!(self.output.is_empty() && self.encoded.is_none());
        true
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_relay_fixture(&mut self, ctx: &egui::Context) {
        self.preview_fixture(ctx);
        self.editor.preview_fixture(ctx);
        self.screenshot.preview_fixture(ctx);
        self.mode = Mode::Screenshot;
        let (source, origins, id) = self.relay_source().unwrap();
        self.relay = Some(Transfer::start(ctx, source, origins, id));
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_relay_smoke(&mut self, ctx: &egui::Context, phase: u8) -> bool {
        if phase == 0 {
            self.preview_relay_fixture(ctx);
            return false;
        }
        let Some(transfer) = self.relay.as_mut() else {
            return false;
        };
        transfer.poll(ctx);
        if transfer.prepared.is_none() {
            return false;
        }
        if phase == 1 {
            let source = self.screenshot.relay_source().unwrap();
            let before = self.source.clone().unwrap();
            assert!(self.apply_relay(ctx).is_err());
            assert!(Arc::ptr_eq(self.source.as_ref().unwrap(), &before));
            self.relay.as_mut().unwrap().replace = true;
            self.apply_relay(ctx).unwrap();
            assert_eq!(self.source.as_ref().unwrap().to_rgba8(), *source);
            assert!(Arc::ptr_eq(
                &self.screenshot.relay_source().unwrap(),
                &source
            ));
            assert!(self.output.is_empty() && self.encoded.is_none());
            let (source, origins, id) = self.relay_source().unwrap();
            self.relay = Some(Transfer::start(ctx, source, origins, id));
            return true;
        }
        assert!(self.apply_relay(ctx).is_err());
        self.relay.as_mut().unwrap().replace = true;
        let source = self.source.clone().unwrap();
        self.apply_relay(ctx).unwrap();
        assert_eq!(self.mode, Mode::Editor);
        self.editor.verify_relay_source(&source, 2);
        assert!(Arc::ptr_eq(self.source.as_ref().unwrap(), &source));
        true
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_fixture(&mut self, ctx: &egui::Context) -> Arc<Vec<u8>> {
        self.preview_fixture(ctx);
        self.editor.preview_fixture(ctx);
        self.metadata.preview_fixture(ctx);
        self.mode = Mode::Metadata;
        self.metadata.preview_cleaned_snapshot()
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_ready(&self) -> bool {
        if let Some(transfer) = &self.relay {
            assert!(
                transfer.error.is_empty(),
                "fixture relay error: {}",
                transfer.error
            );
            transfer.prepared.is_some() && !transfer.busy()
        } else {
            false
        }
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_status(&self) -> String {
        self.relay
            .as_ref()
            .map(|t| {
                format!(
                    "prepared={} replace={} target={} error={}",
                    t.prepared.is_some(),
                    t.replace,
                    t.target.label(),
                    t.error
                )
            })
            .unwrap_or_else(|| "no transfer".into())
    }
    #[cfg(feature = "ui-preview")]
    pub fn preview_metadata_relay_check(&self, phase: u8, expected: &Arc<Vec<u8>>) {
        assert!(Arc::ptr_eq(
            expected,
            &self.metadata.preview_cleaned_snapshot()
        ));
        let clean = image::load_from_memory(expected).unwrap();
        match phase {
            1 | 2 => {
                assert_eq!(self.input, "C:\\Users\\demo\\Pictures\\sample.png");
                assert_eq!(self.output, "C:\\Users\\demo\\Pictures\\sample-edited.jpg");
                assert!(self.encoded.is_some());
                assert_eq!(self.editor.relay_target_state(), (false, true));
                if phase == 1 {
                    let transfer = self.relay.as_ref().unwrap();
                    assert!(!transfer.replace);
                    assert_eq!(transfer.source_id, "image-metadata");
                    assert_eq!(
                        transfer.prepared.as_ref().unwrap().image.to_rgba8(),
                        clean.to_rgba8()
                    );
                } else {
                    assert!(self.relay.is_none());
                }
            }
            3 => {
                assert!(self.relay.is_none());
                assert_eq!(self.mode, Mode::Single);
                assert_eq!(self.source.as_ref().unwrap().to_rgba8(), clean.to_rgba8());
                assert!(self.input.is_empty() && self.output.is_empty() && self.encoded.is_none());
                assert_eq!(self.origins[0].id, "image-metadata");
                assert_eq!(
                    self.origins[0].version,
                    origin("image-metadata").unwrap().version
                );
            }
            4 => {
                assert!(self.relay.is_none());
                assert_eq!(self.mode, Mode::Editor);
                let (source, origins, id) = self.editor.relay_source().unwrap();
                assert_eq!(id, "image-crop-annotate");
                let Source::Image(image) = source else {
                    panic!("received raw memory image")
                };
                assert_eq!(image.to_rgba8(), clean.to_rgba8());
                self.editor.verify_relay_source(&image, 1);
                assert_eq!(origins[0].id, "image-metadata");
                assert_eq!(
                    origins[0].version,
                    origin("image-metadata").unwrap().version
                );
            }
            _ => panic!("relay fixture phase"),
        }
    }
    pub fn relay_active(&self) -> bool {
        self.relay
            .as_ref()
            .is_some_and(|transfer| !transfer.cancelled)
    }
    pub(super) fn relay_source(&self) -> Option<(Source, Vec<Origin>, &'static str)> {
        match self.mode {
            Mode::Single if self.pending.is_none() => self
                .encoded
                .as_ref()
                .map(|bytes| {
                    (
                        Source::Encoded(bytes.clone()),
                        self.origins.clone(),
                        "image-tools",
                    )
                })
                .or_else(|| {
                    self.source.as_ref().map(|image| {
                        (
                            Source::Image(image.clone()),
                            self.origins.clone(),
                            "image-tools",
                        )
                    })
                }),
            Mode::Editor => self.editor.relay_source(),
            Mode::Metadata => self.metadata.relay_source(),
            Mode::Screenshot if !self.screenshot.busy() => self
                .screenshot
                .relay_source()
                .map(|image| (Source::Rgba(image), Vec::new(), "screenshot-workbench")),
            _ => None,
        }
    }
    fn target_state(&self, target: Target) -> (bool, bool) {
        match target {
            Target::Convert => (
                self.pending.is_some(),
                self.source.is_some() || self.encoded.is_some() || !self.input.is_empty(),
            ),
            Target::Edit => self.editor.relay_target_state(),
        }
    }
    fn apply_relay(&mut self, ctx: &egui::Context) -> Result<()> {
        let transfer = self
            .relay
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("接力已取消"))?;
        let prepared = transfer
            .prepared
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("图片还在准备中"))?;
        ensure!(!transfer.cancelled, "接力已取消");
        ensure!(
            !matches!(
                (transfer.source_id, transfer.target),
                ("image-tools", Target::Convert) | ("image-crop-annotate", Target::Edit)
            ),
            "请选择其他工具，来源工作保留"
        );
        let (busy, has_work) = self.target_state(transfer.target);
        ensure!(!busy, "目标工具任务进行中，请等待完成");
        ensure!(
            !has_work || transfer.replace,
            "目标有原图或未保存工作，请明确允许替换"
        );
        match transfer.target {
            Target::Convert => {
                self.source = Some(prepared.image.clone());
                self.source_bytes = prepared.image.as_bytes().len() as u64;
                self.width = prepared.image.width();
                self.format = Format::Png;
                self.relay_route = Some("image-tools");
                self.input.clear();
                self.output.clear();
                self.encoded = None;
                self.origins = prepared.origins.clone();
                self.texture = Some(ctx.load_texture(
                    "image-workbench-preview",
                    prepared.thumbnail.clone(),
                    egui::TextureOptions::LINEAR,
                ));
                self.mode = Mode::Single;
                self.message =
                    "已接收内存图片；生成编码预览后选择输出路径另存，不自动写文件。".into();
                self.error = false;
            }
            Target::Edit => {
                self.editor.receive_relay(ctx, prepared);
                self.relay_route = Some("image-crop-annotate");
                self.mode = Mode::Editor;
            }
        }
        self.relay = None;
        Ok(())
    }
    pub(super) fn relay_ui(&mut self, ui: &mut egui::Ui) {
        if self
            .relay
            .as_ref()
            .is_some_and(|t| t.cancelled && !t.busy())
        {
            self.relay = None;
        }
        if self
            .relay
            .as_ref()
            .is_some_and(|transfer| transfer.cancelled)
        {
            ui.small("上一请求已取消，后台释放快照中；可以继续使用其他工具。");
            return;
        }
        let source = self.relay_source();
        let available = source.is_some();
        ui.horizontal_wrapped(|ui| {
            let response = ui.add_enabled(
                source.is_some() && self.relay.is_none(),
                egui::Button::new("发送图片到工具…"),
            );
            #[cfg(feature = "ui-preview")]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("image-relay-send"), response.rect));
            if response.clicked() {
                if let Some((source, origins, id)) = source {
                    self.relay = Some(Transfer::start(ui.ctx(), source, origins, id));
                }
            }
            ui.small(if !available {
                "先载入图片或生成结果预览；有编辑操作时须先生成编码结果。"
            } else {
                "当前编码结果优先，截图传递透明选区；预览后确认，不创建临时图片。"
            });
        });
        let Some(transfer) = self.relay.as_mut() else {
            return;
        };
        transfer.poll(ui.ctx());
        let mut cancel = false;
        let mut apply = false;
        let converter_state = (
            self.pending.is_some(),
            self.source.is_some() || self.encoded.is_some() || !self.input.is_empty(),
        );
        let editor_state = self.editor.relay_target_state();
        egui::Modal::new(egui::Id::new("image-relay-modal"))
            .area(egui::Modal::default_area(egui::Id::new("image-relay-modal"))
                .anchor(egui::Align2::CENTER_TOP,egui::vec2(0.0,48.0)))
            .show(ui.ctx(), |ui| {
            ui.set_width(540.0_f32.min((ui.ctx().screen_rect().width() - 64.0).max(160.0)));
            ui.heading("图片接力");
            ui.small("来源时间记录接力时刻，不推断原图拍摄或文件创建时间。");
            if let Some(value)=&transfer.prepared {
                ui.label(format!("{} · {} × {} · 来源链{}步",value.kind,value.image.width(),value.image.height(),value.origins.len()));
            }else {ui.label("正在后台准备图片快照");}
            // Reserve the same geometry before and after decoding so controls do
            // not move underneath a pending pointer press/release.
            let width=ui.available_width().min(480.0);
            let (preview_rect,_)=ui.allocate_exact_size(egui::vec2(width,240.0),egui::Sense::hover());
            if let Some(texture)=&transfer.texture {
                let original=texture.size_vec2();
                let size=original*(width/original.x).min(240.0/original.y).min(1.0);
                ui.put(egui::Rect::from_center_size(preview_rect.center(),size),egui::Image::new((texture.id(),size)));
            }else if transfer.busy() {
                ui.put(egui::Rect::from_center_size(preview_rect.center(),egui::vec2(24.0,24.0)),egui::Spinner::new());
            }
            egui::CollapsingHeader::new("来源工具与版本").show(ui,|ui| {
                egui::ScrollArea::vertical().max_height(120.0).show(ui,|ui| {
                    if let Some(value)=&transfer.prepared {
                        for origin in &value.origins {ui.small(format!("{} v{} · {}",origin.id,origin.version,origin.utc));}
                    }else {ui.small("来源链准备中");}
                });
            });
            ui.horizontal(|ui| {
                for target in [Target::Convert, Target::Edit] {
                    if !matches!(
                        (transfer.source_id, target),
                        ("image-tools", Target::Convert) | ("image-crop-annotate", Target::Edit)
                    ) {
                        let response=ui.selectable_value(&mut transfer.target,target,target.label());
                        #[cfg(feature="ui-preview")]
                        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new(if target==Target::Edit {"image-relay-edit"}else{"image-relay-convert"}),response.rect));
                        if response.changed() {transfer.replace=false;}
                    }
                }
            });
            let (busy, has_work) = match transfer.target {
                Target::Convert => converter_state,
                Target::Edit => editor_state,
            };
            if busy {
                ui.label("目标后台任务未完成，不能替换");
            }
            if has_work {
                let response=ui.checkbox(&mut transfer.replace,"允许替换目标当前原图、操作和未保存预览");
                #[cfg(feature="ui-preview")]
                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-relay-replace"),response.rect));
                #[cfg(not(feature="ui-preview"))]
                let _=response;
            }
            ui.small("来源图片和来源工具的工作保留；目标只接收内存原图，不自动编码、保存或联网。更换目标后需重新确认覆盖。");
            if !transfer.error.is_empty() {
                ui.label(&transfer.error);
            }
            ui.horizontal(|ui| {
                cancel = ui.button("取消").clicked();
                let response = ui.add_enabled(
                    !busy && (!has_work || transfer.replace) && transfer.prepared.is_some() && !transfer.cancelled,
                    egui::Button::new("确认接力并打开"),
                );
                apply=response.clicked();
                #[cfg(feature="ui-preview")]
                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("image-relay-apply"),response.rect));
            });
        });
        if ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            cancel = true;
        }
        if cancel {
            if transfer.busy() {
                transfer.cancelled = true;
            } else {
                self.relay = None;
            }
        } else if apply {
            if let Err(error) = self.apply_relay(ui.ctx()) {
                if let Some(transfer) = &mut self.relay {
                    transfer.error = error.to_string();
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoded_result_preserves_alpha_and_dimensions_and_arc_source() {
        let source = Arc::new(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(
            7,
            5,
            |x, y| image::Rgba([x as u8, y as u8, 90, if x == 0 { 0 } else { 255 }]),
        )));
        let prepared = prepare(Source::Image(source.clone()), Vec::new(), "image-tools").unwrap();
        assert!(Arc::ptr_eq(&source, &prepared.image));
        let mut bytes = Cursor::new(Vec::new());
        source.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let encoded = prepare(
            Source::Encoded(Arc::new(bytes.into_inner())),
            prepared.origins,
            "image-crop-annotate",
        )
        .unwrap();
        assert_eq!(encoded.image.to_rgba8(), source.to_rgba8());
        assert_eq!(encoded.origins.len(), 2);
        assert!(
            prepare(
                Source::Encoded(Arc::new(b"bad fixture".to_vec())),
                Vec::new(),
                "image-tools"
            )
            .is_err()
        );
        assert!(
            prepare(
                Source::Image(Arc::new(DynamicImage::new_rgba8(0, 0))),
                Vec::new(),
                "image-tools"
            )
            .is_err()
        );
    }
    #[test]
    fn state_prefers_encoded_snapshot_and_cancelled_receipt_does_not_apply() {
        let ctx = egui::Context::default();
        let mut state = State {
            source: Some(Arc::new(DynamicImage::new_rgba8(20, 10))),
            ..Default::default()
        };
        let output = DynamicImage::new_rgba8(3, 2);
        let mut bytes = Cursor::new(Vec::new());
        output.write_to(&mut bytes, ImageFormat::Png).unwrap();
        state.encoded = Some(Arc::new(bytes.into_inner()));
        let (source, origins, id) = state.relay_source().unwrap();
        let value = prepare(source, origins, id).unwrap();
        assert_eq!((value.image.width(), value.image.height()), (3, 2));
        assert_eq!(value.kind, "当前编码结果");
        let (tx, rx) = mpsc::channel();
        let mut transfer = Transfer {
            source_id: "image-tools",
            target: Target::Edit,
            replace: false,
            cancelled: true,
            error: String::new(),
            prepared: None,
            texture: None,
            receiver: Some(rx),
        };
        tx.send(Ok(value)).unwrap();
        transfer.poll(&ctx);
        assert!(!transfer.busy() && transfer.prepared.is_none() && transfer.texture.is_none());
        assert_eq!(state.source.as_ref().unwrap().width(), 20);
    }
    #[test]
    fn busy_and_unconfirmed_targets_preserve_work_then_explicit_relay_shares_source() {
        let ctx = egui::Context::default();
        let source = Arc::new(DynamicImage::new_rgba8(12, 8));
        let prepared = prepare(
            Source::Image(source.clone()),
            Vec::new(),
            "screenshot-workbench",
        )
        .unwrap();
        let mut state = State::default();
        let original = Arc::new(DynamicImage::new_rgba8(2, 3));
        state.source = Some(original.clone());
        state.input = "synthetic old input".into();
        state.relay = Some(Transfer {
            source_id: "screenshot-workbench",
            target: Target::Convert,
            replace: false,
            cancelled: false,
            error: String::new(),
            prepared: Some(prepared),
            texture: None,
            receiver: None,
        });
        assert!(state.apply_relay(&ctx).is_err());
        assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &original));
        let (_tx, rx) = mpsc::channel();
        state.pending = Some(rx);
        state.relay.as_mut().unwrap().replace = true;
        assert!(state.apply_relay(&ctx).is_err());
        assert_eq!(state.input, "synthetic old input");
        state.pending = None;
        state.apply_relay(&ctx).unwrap();
        assert!(Arc::ptr_eq(state.source.as_ref().unwrap(), &source));
        assert_eq!(state.origins[0].id, "screenshot-workbench");
        assert!(state.encoded.is_none() && state.output.is_empty());
    }
}
