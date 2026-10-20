//! Versioned definitions and bounded image execution, separate from view state.
use super::*;
use anyhow::Context;
use serde::{Deserialize, Serialize};
pub(super) const RECIPE_LIMIT: usize = 65536;
const RESULT_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", deny_unknown_fields)]
pub(crate) enum Step {
    #[serde(rename = "image.crop")]
    Crop {
        version: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    #[serde(rename = "image.resize")]
    Resize { version: u32, max_width: u32 },
    #[serde(rename = "image.encode")]
    Encode {
        version: u32,
        format: Encoding,
        quality: u8,
    },
    #[serde(rename = "image.info")]
    Info { version: u32 },
    #[serde(rename = "image.ocr")]
    Ocr { version: u32 },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Encoding {
    Png,
    Jpeg,
    Webp,
}
impl Encoding {
    pub(super) fn native(self) -> Format {
        match self {
            Self::Png => Format::Png,
            Self::Jpeg => Format::Jpeg,
            Self::Webp => Format::WebP,
        }
    }
}
impl Step {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Crop { .. } => "裁剪",
            Self::Resize { .. } => "缩放",
            Self::Encode { .. } => "编码",
            Self::Info { .. } => "信息报告",
            Self::Ocr { .. } => "本机 OCR",
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        let version = match self {
            Self::Crop { version, .. }
            | Self::Resize { version, .. }
            | Self::Encode { version, .. }
            | Self::Info { version }
            | Self::Ocr { version } => *version,
        };
        ensure!(version == 1, "不支持此图片动作版本");
        match self {
            Self::Crop {
                x,
                y,
                width,
                height,
                ..
            } => ensure!(
                *width > 0
                    && *height > 0
                    && u64::from(*x) + u64::from(*width) <= 10000
                    && u64::from(*y) + u64::from(*height) <= 10000,
                "裁剪比例必须在0..10000内且不能为空"
            ),
            Self::Resize { max_width, .. } => {
                ensure!((1..=12000).contains(max_width), "缩放宽度需在1..12000内")
            }
            Self::Encode { quality, .. } => {
                ensure!((35..=95).contains(quality), "JPEG质量需在35..95内")
            }
            Self::Info { .. } => {}
            Self::Ocr { .. } => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    pub(super) kind: String,
    pub(super) schema_version: u32,
    pub steps: Vec<Step>,
}
impl Default for Definition {
    fn default() -> Self {
        Self {
            kind: "image-workflow".into(),
            schema_version: 1,
            steps: vec![
                Step::Resize {
                    version: 1,
                    max_width: 1600,
                },
                Step::Encode {
                    version: 1,
                    format: Encoding::Webp,
                    quality: 80,
                },
                Step::Info { version: 1 },
            ],
        }
    }
}
impl Definition {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == "image-workflow" && self.schema_version == 1,
            "不支持的图片流程格式"
        );
        ensure!(
            !self.steps.is_empty() && self.steps.len() <= 16,
            "图片流程需1..16步"
        );
        for step in &self.steps {
            step.validate()?;
        }
        Ok(())
    }
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= RECIPE_LIMIT, "流程文件最多64KiB");
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }
    pub(super) fn bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let b = serde_json::to_vec_pretty(self)?;
        ensure!(b.len() <= RECIPE_LIMIT, "流程文件过大");
        Ok(b)
    }
}
pub(super) struct Output {
    pub(super) image: Arc<DynamicImage>,
    pub(super) encoded: Option<Arc<Vec<u8>>>,
    pub(super) report: Option<String>,
    pub(super) report_label: &'static str,
    pub(super) label: &'static str,
}
#[derive(Default)]
pub(super) struct Run {
    pub(super) outputs: Vec<Output>,
    pub(super) failure: Option<String>,
    pub(super) cancelled: bool,
}
#[cfg(any(test, feature = "ui-preview"))]
pub(super) fn execute(
    def: &Definition,
    source: Arc<DynamicImage>,
    cancel: &AtomicBool,
) -> Result<Run> {
    execute_budgeted(def, source, cancel, &super::super::memory::Pool::default())
}
pub(super) fn execute_budgeted(
    def: &Definition,
    source: Arc<DynamicImage>,
    cancel: &AtomicBool,
    memory: &super::super::memory::Pool,
) -> Result<Run> {
    def.validate()?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(Run {
            cancelled: true,
            ..Run::default()
        });
    }
    // Reuse the same image validation as explicit relay, without a disk read.
    relay::prepare(
        relay::Source::Image(source.clone()),
        Vec::new(),
        "image-workflow",
    )?;
    memory
        .share(&source, super::super::memory::pixels(&source))
        .map_err(anyhow::Error::msg)?;
    let mut current = source;
    let mut encoded: Option<Arc<Vec<u8>>> = None;
    let mut retained = current.as_bytes().len();
    let mut run = Run::default();
    for (index, step) in def.steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            run.cancelled = true;
            break;
        }
        let result = (|| -> Result<Output> {
            let mut report = None;
            let mut report_label = "图片信息报告 JSON";
            match step {
                Step::Crop {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    let region = zi_image_core::CropRegion {
                        x: *x,
                        y: *y,
                        width: *width,
                        height: *height,
                    };
                    let bounds =
                        zi_image_core::crop_bounds(current.width(), current.height(), region)?;
                    reserve(
                        retained,
                        bounds.width as usize
                            * bounds.height as usize
                            * usize::from(current.color().bytes_per_pixel()),
                    )?;
                    let guard = memory
                        .reserve(
                            bounds.width as usize
                                * bounds.height as usize
                                * usize::from(current.color().bytes_per_pixel()),
                        )
                        .map_err(anyhow::Error::msg)?;
                    current = Arc::new(zi_image_core::crop_percent(&current, region)?);
                    guard
                        .promote(&current, super::super::memory::pixels(&current))
                        .map_err(anyhow::Error::msg)?;
                    encoded = None;
                    retained = retained.saturating_add(current.as_bytes().len());
                }
                Step::Resize { max_width, .. } => {
                    let (width, height) = zi_image_core::resize_dimensions(
                        current.width(),
                        current.height(),
                        *max_width,
                    )?;
                    if width != current.width() {
                        reserve(
                            retained,
                            width as usize
                                * height as usize
                                * usize::from(current.color().bytes_per_pixel()),
                        )?;
                        let guard = memory
                            .reserve(
                                width as usize
                                    * height as usize
                                    * usize::from(current.color().bytes_per_pixel()),
                            )
                            .map_err(anyhow::Error::msg)?;
                        current = Arc::new(zi_image_core::resize_to_width(&current, *max_width)?);
                        guard
                            .promote(&current, super::super::memory::pixels(&current))
                            .map_err(anyhow::Error::msg)?;
                        encoded = None;
                        retained = retained.saturating_add(current.as_bytes().len());
                    }
                }
                Step::Encode {
                    format, quality, ..
                } => {
                    let guard = memory
                        .reserve(
                            current.width() as usize
                                * current.height() as usize
                                * usize::from(current.color().bytes_per_pixel().max(4)),
                        )
                        .map_err(anyhow::Error::msg)?;
                    let bytes = super::super::encoding::encode_tracked(
                        &current,
                        format.native().image_format(),
                        *quality,
                        MAX_OUTPUT_BYTES,
                        memory,
                    )?;
                    reserve(
                        retained,
                        bytes.len().saturating_add(
                            current.width() as usize
                                * current.height() as usize
                                * usize::from(current.color().bytes_per_pixel().max(4)),
                        ),
                    )?;
                    let prepared = relay::prepare(
                        relay::Source::Encoded(bytes.clone()),
                        Vec::new(),
                        "image-workflow",
                    )?;
                    current = prepared.image;
                    guard
                        .promote(&current, super::super::memory::pixels(&current))
                        .map_err(anyhow::Error::msg)?;
                    retained = retained
                        .saturating_add(current.as_bytes().len())
                        .saturating_add(bytes.len());
                    encoded = Some(bytes);
                }
                Step::Info { .. } => {
                    let generated_by = crate::tool_provenance::current_for_tool("image-workflow")?;
                    report = Some(serde_json::to_string_pretty(&serde_json::json!({
                        "schemaVersion":2,"material":"image-workflow-report","generatedBy":generated_by,"step":index+1,
                        "width":current.width(),"height":current.height(),"hasAlphaChannel":current.color().has_alpha(),
                        "encoding":encoded.as_ref().map(|b|serde_json::json!({"bytes":b.len(),"format":match image::guess_format(b).ok(){Some(ImageFormat::Png)=>"png",Some(ImageFormat::Jpeg)=>"jpeg",Some(ImageFormat::WebP)=>"webp",_=>"unknown"}})),
                        "actionVersion":1
                    }))?);
                }
                Step::Ocr { .. } => {
                    let rgba_bytes = (current.width() as usize)
                        .checked_mul(current.height() as usize)
                        .and_then(|pixels| pixels.checked_mul(4))
                        .context("OCR 图片缓冲尺寸溢出")?;
                    let _ocr_memory = memory
                        .reserve(rgba_bytes.saturating_mul(2))
                        .map_err(anyhow::Error::msg)?;
                    #[cfg(windows)]
                    {
                        let text = crate::clipboard::recognize_image(&current)
                            .map_err(anyhow::Error::msg)?;
                        reserve(retained, text.len())?;
                        retained = retained.saturating_add(text.len());
                        report = Some(text);
                        report_label = "本机 OCR 识别文字";
                    }
                    #[cfg(not(windows))]
                    anyhow::bail!("本机 OCR 目前仅支持 Windows");
                }
            }
            ensure!(
                retained <= RESULT_LIMIT,
                "步骤结果累计超过256MiB，请减少步骤或尺寸"
            );
            ensure!(!cancel.load(Ordering::Relaxed), "已取消");
            Ok(Output {
                image: current.clone(),
                encoded: encoded.clone(),
                report,
                report_label,
                label: step.label(),
            })
        })();
        match result {
            Ok(out) => run.outputs.push(out),
            Err(error) => {
                if cancel.load(Ordering::Relaxed) {
                    run.cancelled = true;
                } else {
                    run.failure = Some(format!("第{}步 {}：{error}", index + 1, step.label()));
                }
                break;
            }
        }
    }
    run.cancelled |= cancel.load(Ordering::Relaxed);
    Ok(run)
}
fn reserve(retained: usize, next: usize) -> Result<()> {
    ensure!(
        retained
            .checked_add(next)
            .is_some_and(|n| n <= RESULT_LIMIT),
        "步骤结果累计超过256MiB，请减少步骤或尺寸"
    );
    Ok(())
}
