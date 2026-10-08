//! Versioned definitions and bounded image execution, separate from view state.
use super::*;
use serde::{Deserialize, Serialize};
pub(super) const RECIPE_LIMIT: usize = 65536;
const RESULT_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", deny_unknown_fields)]
pub(super) enum Step {
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
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Encoding {
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
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        let version = match self {
            Self::Crop { version, .. }
            | Self::Resize { version, .. }
            | Self::Encode { version, .. }
            | Self::Info { version } => *version,
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
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Definition {
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
    pub(super) fn parse(bytes: &[u8]) -> Result<Self> {
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
    pub(super) label: &'static str,
}
#[derive(Default)]
pub(super) struct Run {
    pub(super) outputs: Vec<Output>,
    pub(super) failure: Option<String>,
    pub(super) cancelled: bool,
}
pub(super) fn execute(
    def: &Definition,
    source: Arc<DynamicImage>,
    cancel: &AtomicBool,
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
            match step {
                Step::Crop {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    let w = u64::from(current.width());
                    let h = u64::from(current.height());
                    let x0 = w * u64::from(*x) / 10000;
                    let y0 = h * u64::from(*y) / 10000;
                    let x1 = (w * (u64::from(*x) + u64::from(*width))).div_ceil(10000);
                    let y1 = (h * (u64::from(*y) + u64::from(*height))).div_ceil(10000);
                    ensure!(x1 > x0 && y1 > y0, "裁剪区域为空");
                    reserve(
                        retained,
                        ((x1 - x0) * (y1 - y0)) as usize
                            * usize::from(current.color().bytes_per_pixel()),
                    )?;
                    current = Arc::new(current.crop_imm(
                        x0 as u32,
                        y0 as u32,
                        (x1 - x0) as u32,
                        (y1 - y0) as u32,
                    ));
                    encoded = None;
                    retained = retained.saturating_add(current.as_bytes().len());
                }
                Step::Resize { max_width, .. } => {
                    let width = current.width().min(*max_width);
                    if width != current.width() {
                        let height = ((u64::from(current.height()) * u64::from(width)
                            + u64::from(current.width()) / 2)
                            / u64::from(current.width()))
                        .max(1) as u32;
                        reserve(
                            retained,
                            width as usize
                                * height as usize
                                * usize::from(current.color().bytes_per_pixel()),
                        )?;
                        current =
                            Arc::new(current.resize_exact(width, height, FilterType::Lanczos3));
                        encoded = None;
                        retained = retained.saturating_add(current.as_bytes().len());
                    }
                }
                Step::Encode {
                    format, quality, ..
                } => {
                    let (bytes, _, _) =
                        encode_image(&current, current.width(), format.native(), *quality)?;
                    let bytes = Arc::new(bytes);
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
                    retained = retained
                        .saturating_add(current.as_bytes().len())
                        .saturating_add(bytes.len());
                    encoded = Some(bytes);
                }
                Step::Info { .. } => {
                    report = Some(serde_json::to_string_pretty(&serde_json::json!({
                        "schemaVersion":1,"material":"image-workflow-report","step":index+1,
                        "width":current.width(),"height":current.height(),"hasAlphaChannel":current.color().has_alpha(),
                        "encoding":encoded.as_ref().map(|b|serde_json::json!({"bytes":b.len(),"format":match image::guess_format(b).ok(){Some(ImageFormat::Png)=>"png",Some(ImageFormat::Jpeg)=>"jpeg",Some(ImageFormat::WebP)=>"webp",_=>"unknown"}})),
                        "actionVersion":1,"toolVersion":relay::origin("image-workflow")?.version
                    }))?);
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
