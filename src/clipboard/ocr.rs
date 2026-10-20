//! On-device Windows OCR for explicitly selected clipboard image previews.
use super::MAX_PDF_OCR_PAGES;
use anyhow::{Context, Result, ensure};
use std::sync::{
    OnceLock,
    mpsc::{self, Sender, SyncSender},
};
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};

const TEXT_LIMIT: usize = 1024 * 1024;
const LANGUAGE_LIMIT: u32 = 256;
const PDF_RENDER_PIXEL_LIMIT: u64 = 8_000_000;
// Keep pages below the practical Windows OCR sweet spot even when the engine advertises a larger
// hard limit. Our synthetic PDF fixture returned empty OCR text at larger dimensions.
const PDF_RENDER_DIMENSION_LIMIT: f32 = 1_200.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecognizerLanguage {
    pub tag: String,
    pub display_name: String,
}

enum OcrCommand {
    Languages(Sender<std::result::Result<Vec<RecognizerLanguage>, String>>),
    Recognize {
        image: image::DynamicImage,
        language_tag: Option<String>,
        reply: Sender<std::result::Result<String, String>>,
    },
    RecognizePdf {
        bytes: Vec<u8>,
        expected_page_count: u32,
        page_indices: Vec<(u32, u32)>,
        language_tag: Option<String>,
        reply: Sender<std::result::Result<Vec<(u32, String)>, String>>,
    },
}

static OCR_WORKER: OnceLock<std::result::Result<SyncSender<OcrCommand>, String>> = OnceLock::new();

fn ocr_worker() -> std::result::Result<&'static SyncSender<OcrCommand>, String> {
    match OCR_WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("zi-devtools-ocr".into())
            .spawn(move || {
                // WinRT factories used by OCR must stay within a live MTA. Keep this worker and
                // its apartment alive across language enumeration and subsequent OCR requests.
                let apartment = WinRtApartment::initialize().map_err(|error| format!("{error:#}"));
                while let Ok(command) = receiver.recv() {
                    match command {
                        OcrCommand::Languages(reply) => {
                            let result = match &apartment {
                                Ok(_apartment) => available_languages_inner()
                                    .map_err(|error| format!("无法读取本机 OCR 语言：{error:#}")),
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                        OcrCommand::Recognize {
                            image,
                            language_tag,
                            reply,
                        } => {
                            let result = match &apartment {
                                Ok(_apartment) => {
                                    recognize_image_inner(&image, language_tag.as_deref())
                                        .map_err(|error| format!("本机 OCR 失败：{error:#}"))
                                }
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                        OcrCommand::RecognizePdf {
                            bytes,
                            expected_page_count,
                            page_indices,
                            language_tag,
                            reply,
                        } => {
                            let result = match &apartment {
                                Ok(_apartment) => recognize_pdf_inner(
                                    &bytes,
                                    expected_page_count,
                                    &page_indices,
                                    language_tag.as_deref(),
                                )
                                .map_err(|error| format!("扫描 PDF OCR 失败：{error:#}")),
                                Err(error) => Err(error.clone()),
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .map_err(|error| format!("无法启动本机 OCR 工作线程：{error}"))?;
        Ok(sender)
    }) {
        Ok(sender) => Ok(sender),
        Err(error) => Err(error.clone()),
    }
}

fn enqueue_ocr(sender: &SyncSender<OcrCommand>, command: OcrCommand) -> Result<(), String> {
    sender.try_send(command).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => "本机 OCR 队列已满，请稍候重试".into(),
        mpsc::TrySendError::Disconnected(_) => "本机 OCR 工作线程已停止".into(),
    })
}

struct WinRtApartment;

impl WinRtApartment {
    fn initialize() -> Result<Self> {
        // OCR runs on its own worker, so it owns a fresh MTA apartment.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
            .context("无法初始化 Windows OCR 运行环境")?;
        Ok(Self)
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

pub(super) fn recognize(
    picture: &super::Picture,
    region: Option<super::OcrRegion>,
    language_tag: Option<String>,
) -> std::result::Result<String, String> {
    let image = picture
        .image()
        .map_err(|error| format!("无法读取剪贴板图片像素：{error:#}"))?;
    let image = crop_region(image, region)?;
    recognize_owned_image_with_language(image, language_tag.as_deref())
}

fn crop_region(
    image: image::DynamicImage,
    region: Option<super::OcrRegion>,
) -> std::result::Result<image::DynamicImage, String> {
    let Some(region) = region else {
        return Ok(image);
    };
    let right = region.x.checked_add(region.width);
    let bottom = region.y.checked_add(region.height);
    if region.width == 0
        || region.height == 0
        || right.is_none_or(|right| right > image.width())
        || bottom.is_none_or(|bottom| bottom > image.height())
    {
        return Err("局部 OCR 区域超出图片范围".into());
    }
    Ok(image.crop_imm(region.x, region.y, region.width, region.height))
}

pub(crate) fn recognize_image(image: &image::DynamicImage) -> std::result::Result<String, String> {
    recognize_image_with_language(image, None)
}

pub(crate) fn recognize_image_with_language(
    image: &image::DynamicImage,
    language_tag: Option<&str>,
) -> std::result::Result<String, String> {
    recognize_owned_image_with_language(image.clone(), language_tag)
}

pub(crate) fn recognize_pdf_pages_with_language(
    bytes: &[u8],
    expected_page_count: u32,
    page_indices: &[(u32, u32)],
    language_tag: Option<&str>,
) -> std::result::Result<Vec<(u32, String)>, String> {
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("PDF 超过 16 MiB".into());
    }
    if page_indices.len() > MAX_PDF_OCR_PAGES {
        return Err("扫描 PDF 超过 50 个待识别页面，请拆分后导入".into());
    }
    if page_indices.is_empty() {
        return Ok(Vec::new());
    }
    let (reply, result) = mpsc::channel();
    enqueue_ocr(
        ocr_worker()?,
        OcrCommand::RecognizePdf {
            bytes: bytes.to_vec(),
            expected_page_count,
            page_indices: page_indices.to_vec(),
            language_tag: language_tag.map(str::to_owned),
            reply,
        },
    )?;
    result
        .recv()
        .unwrap_or_else(|_| Err("本机 OCR 工作线程意外结束".into()))
}

fn recognize_owned_image_with_language(
    image: image::DynamicImage,
    language_tag: Option<&str>,
) -> std::result::Result<String, String> {
    let (reply, result) = mpsc::channel();
    enqueue_ocr(
        ocr_worker()?,
        OcrCommand::Recognize {
            image,
            language_tag: language_tag.map(str::to_owned),
            reply,
        },
    )?;
    result
        .recv()
        .unwrap_or_else(|_| Err("本机 OCR 工作线程意外结束".into()))
}

pub(crate) fn available_languages() -> std::result::Result<Vec<RecognizerLanguage>, String> {
    let (reply, result) = mpsc::channel();
    enqueue_ocr(ocr_worker()?, OcrCommand::Languages(reply))?;
    result
        .recv()
        .unwrap_or_else(|_| Err("本机 OCR 工作线程意外结束".into()))
}

fn available_languages_inner() -> Result<Vec<RecognizerLanguage>> {
    let available =
        OcrEngine::AvailableRecognizerLanguages().context("无法查询 Windows 已安装的 OCR 语言")?;
    let count = available.Size().context("无法读取 OCR 语言数量")?;
    ensure!(count <= LANGUAGE_LIMIT, "Windows OCR 返回过多语言选项");
    let mut languages = Vec::with_capacity(count as usize);
    for index in 0..count {
        let language = available
            .GetAt(index)
            .with_context(|| format!("无法读取第 {} 个 OCR 语言", index + 1))?;
        let tag = language
            .LanguageTag()
            .context("无法读取 OCR 语言标识")?
            .to_string();
        let display_name = language
            .DisplayName()
            .context("无法读取 OCR 语言名称")?
            .to_string();
        if !tag.is_empty() && !display_name.is_empty() {
            languages.push(RecognizerLanguage { tag, display_name });
        }
    }
    sort_and_deduplicate_languages(&mut languages);
    Ok(languages)
}

fn sort_and_deduplicate_languages(languages: &mut Vec<RecognizerLanguage>) {
    let mut seen = std::collections::HashSet::with_capacity(languages.len());
    languages.retain(|language| seen.insert(language.tag.to_ascii_lowercase()));
    languages.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.tag.cmp(&right.tag))
    });
}

fn language_tag_matches(available: &str, requested: &str) -> bool {
    available.eq_ignore_ascii_case(requested)
}

fn recognize_image_inner(
    decoded: &image::DynamicImage,
    language_tag: Option<&str>,
) -> Result<String> {
    let engine = create_ocr_engine(language_tag)?;
    let max_dimension = OcrEngine::MaxImageDimension()?;
    ensure!(max_dimension > 0, "Windows OCR 返回了无效的图像尺寸上限");

    let rgba = if decoded.width() > max_dimension || decoded.height() > max_dimension {
        decoded.thumbnail(max_dimension, max_dimension).to_rgba8()
    } else {
        decoded.to_rgba8()
    };
    let width = i32::try_from(rgba.width()).context("图片宽度超出 OCR 范围")?;
    let height = i32::try_from(rgba.height()).context("图片高度超出 OCR 范围")?;
    let stream = InMemoryRandomAccessStream::new().context("无法创建 OCR 图像缓冲")?;
    let writer = DataWriter::CreateDataWriter(&stream).context("无法初始化 OCR 图像缓冲")?;
    writer
        .WriteBytes(&rgba.into_raw())
        .context("无法写入 OCR 图像像素")?;
    let buffer = writer.DetachBuffer().context("无法读取 OCR 图像缓冲")?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Rgba8, width, height)
            .context("Windows 无法将图片像素转换为 OCR 位图")?;
    let text = recognize_bitmap_inner(&engine, &bitmap)?;
    validate_text(text)
}

fn create_ocr_engine(language_tag: Option<&str>) -> Result<OcrEngine> {
    let engine = if let Some(language_tag) = language_tag {
        let language =
            Language::CreateLanguage(&language_tag.into()).context("无法解析所选 OCR 语言标识")?;
        let engine = OcrEngine::TryCreateFromLanguage(&language)
            .context("无法创建所选 OCR 语言的识别引擎；请检查 Windows 语言功能")?;
        let resolved_tag = engine
            .RecognizerLanguage()
            .context("无法读取 OCR 引擎语言")?
            .LanguageTag()
            .context("无法读取 OCR 引擎语言标识")?
            .to_string();
        ensure!(
            language_tag_matches(&resolved_tag, language_tag),
            "Windows 将所选 OCR 语言解析为 {resolved_tag}，未使用其他语言"
        );
        engine
    } else {
        OcrEngine::TryCreateFromUserProfileLanguages()
            .context("Windows 未提供用户首选语言的 OCR 引擎；请安装相应语言的 OCR 功能")?
    };
    Ok(engine)
}

fn recognize_bitmap_inner(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<String> {
    let result = engine
        .RecognizeAsync(bitmap)
        .context("Windows OCR 无法开始识别")?;
    let result = result.join().context("Windows OCR 识别失败")?;
    let text = result
        .Text()
        .context("无法读取 Windows OCR 结果")?
        .to_string();
    let text = normalize_han_spacing(text);
    ensure!(text.len() <= TEXT_LIMIT, "识别文本超过 1 MiB");
    Ok(text)
}

fn is_han_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F
    )
}

fn normalize_han_spacing(text: String) -> String {
    let characters: Vec<_> = text.chars().collect();
    let mut normalized = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == ' ' {
            let start = index;
            while index < characters.len() && characters[index] == ' ' {
                index += 1;
            }
            let previous = normalized.chars().next_back();
            let next = characters.get(index).copied();
            if previous.is_some_and(is_han_character) && next.is_some_and(is_han_character) {
                continue;
            }
            normalized.extend(characters[start..index].iter());
        } else {
            normalized.push(characters[index]);
            index += 1;
        }
    }
    normalized
}

fn recognize_pdf_inner(
    bytes: &[u8],
    expected_page_count: u32,
    page_indices: &[(u32, u32)],
    language_tag: Option<&str>,
) -> Result<Vec<(u32, String)>> {
    ensure!(
        page_indices.len() <= MAX_PDF_OCR_PAGES,
        "扫描 PDF 超过 50 个待识别页面，请拆分后导入"
    );
    let engine = create_ocr_engine(language_tag)?;
    let stream = InMemoryRandomAccessStream::new().context("无法创建 PDF 输入缓冲")?;
    let writer = DataWriter::CreateDataWriter(&stream).context("无法初始化 PDF 输入缓冲")?;
    writer.WriteBytes(bytes).context("无法写入 PDF 输入缓冲")?;
    writer
        .StoreAsync()
        .context("无法提交 PDF 输入缓冲")?
        .join()
        .context("无法提交 PDF 输入缓冲")?;
    writer
        .DetachStream()
        .context("无法释放 PDF 输入缓冲写入器")?;
    stream.Seek(0).context("无法定位 PDF 输入缓冲")?;
    let document = PdfDocument::LoadFromStreamAsync(&stream)
        .context("Windows 无法打开此 PDF")?
        .join()
        .context("Windows 无法解析此 PDF")?;
    ensure!(
        !document
            .IsPasswordProtected()
            .context("无法读取 PDF 加密状态")?,
        "PDF 已加密，无法解析"
    );
    let page_count = document.PageCount().context("无法读取 PDF 页数")?;
    ensure!(
        page_count == expected_page_count,
        "PDF 解析器对页数的判断不一致，无法保证识别结果页码"
    );
    let mut output = Vec::with_capacity(page_indices.len());
    let mut total_text = 0usize;
    for (number, index) in page_indices {
        ensure!(*index < page_count, "PDF 页面索引超出范围");
        let page = document
            .GetPage(*index)
            .with_context(|| format!("无法读取 PDF 第 {number} 页"))?;
        let result = (|| {
            let size = page.Size().context("无法读取 PDF 页面尺寸")?;
            ensure!(
                size.Width.is_finite()
                    && size.Height.is_finite()
                    && size.Width > 0.0
                    && size.Height > 0.0,
                "PDF 页面尺寸无效"
            );
            let area = f64::from(size.Width) * f64::from(size.Height);
            let scale = (2.0_f64)
                .min(f64::from(PDF_RENDER_DIMENSION_LIMIT) / f64::from(size.Width.max(size.Height)))
                .min((PDF_RENDER_PIXEL_LIMIT as f64 / area).sqrt());
            let width = (f64::from(size.Width) * scale)
                .round()
                .clamp(1.0, f64::from(PDF_RENDER_DIMENSION_LIMIT)) as u32;
            let height = (f64::from(size.Height) * scale)
                .round()
                .clamp(1.0, f64::from(PDF_RENDER_DIMENSION_LIMIT)) as u32;
            ensure!(
                u64::from(width) * u64::from(height) <= PDF_RENDER_PIXEL_LIMIT,
                "PDF 页面渲染尺寸超过 800 万像素"
            );
            let options = PdfPageRenderOptions::new().context("无法配置 PDF 页面渲染")?;
            options
                .SetDestinationWidth(width)
                .context("无法设置 PDF 页面渲染宽度")?;
            options
                .SetDestinationHeight(height)
                .context("无法设置 PDF 页面渲染高度")?;
            let rendered =
                InMemoryRandomAccessStream::new().context("无法创建 PDF 页面图像缓冲")?;
            page.RenderWithOptionsToStreamAsync(&rendered, &options)
                .context("无法开始 PDF 页面渲染")?
                .join()
                .context("PDF 页面渲染失败")?;
            let rendered_size = rendered.Size().context("无法读取 PDF 页面图像长度")?;
            ensure!(
                rendered_size > 0 && rendered_size <= 40 * 1024 * 1024,
                "PDF 页面图像超过 40 MiB"
            );
            let input = rendered
                .GetInputStreamAt(0)
                .context("无法读取 PDF 页面图像缓冲")?;
            let reader =
                DataReader::CreateDataReader(&input).context("无法初始化 PDF 页面图像读取器")?;
            reader
                .LoadAsync(rendered_size as u32)
                .context("无法读取 PDF 页面图像")?
                .join()
                .context("无法读取 PDF 页面图像")?;
            let mut png = vec![0; rendered_size as usize];
            reader
                .ReadBytes(&mut png)
                .context("无法复制 PDF 页面图像")?;
            let decoded = image::load_from_memory(&png).context("无法解码 PDF 页面图像")?;
            let actual_width = decoded.width();
            let actual_height = decoded.height();
            ensure!(
                actual_width == width
                    && actual_height == height
                    && u64::from(actual_width) * u64::from(actual_height) <= PDF_RENDER_PIXEL_LIMIT,
                "PDF 页面图像尺寸与渲染预算不符"
            );
            let max_dimension =
                OcrEngine::MaxImageDimension().context("无法读取 Windows OCR 图像尺寸上限")?;
            ensure!(max_dimension > 0, "Windows OCR 返回了无效的图像尺寸上限");
            let rgba = if actual_width > max_dimension || actual_height > max_dimension {
                decoded.thumbnail(max_dimension, max_dimension).to_rgba8()
            } else {
                decoded.to_rgba8()
            };
            let ocr_width = rgba.width();
            let ocr_height = rgba.height();
            let pixel_stream =
                InMemoryRandomAccessStream::new().context("无法创建 PDF 页面 OCR 缓冲")?;
            let pixel_writer = DataWriter::CreateDataWriter(&pixel_stream)
                .context("无法初始化 PDF 页面 OCR 缓冲")?;
            pixel_writer
                .WriteBytes(&rgba.into_raw())
                .context("无法复制 PDF 页面 OCR 像素")?;
            let pixel_buffer = pixel_writer
                .DetachBuffer()
                .context("无法读取 PDF 页面 OCR 像素")?;
            let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
                &pixel_buffer,
                BitmapPixelFormat::Rgba8,
                i32::try_from(ocr_width)?,
                i32::try_from(ocr_height)?,
            )
            .context("无法创建 PDF 页面 OCR 位图")?;
            let text = recognize_bitmap_inner(&engine, &bitmap)?;
            Ok::<_, anyhow::Error>(text)
        })();
        page.Close().context("无法释放 PDF 页面资源")?;
        let text = result.with_context(|| format!("PDF 第 {number} 页 OCR 失败"))?;
        total_text = total_text.saturating_add(text.len());
        ensure!(total_text <= 4 * 1024 * 1024, "扫描 PDF OCR 正文超过 4 MiB");
        output.push((*number, text));
    }
    Ok(output)
}

fn validate_text(text: String) -> Result<String> {
    ensure!(!text.trim().is_empty(), "未在图片中识别到文字");
    ensure!(
        text.len() <= TEXT_LIMIT,
        "识别文本超过 1 MiB，未生成接力结果"
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognized_text_has_a_hard_output_bound() {
        let short = " first line\nsecond line ".to_string();
        assert_eq!(validate_text(short.clone()).unwrap(), short);
        assert!(validate_text(" \n\t".into()).is_err());
        assert!(validate_text("x".repeat(TEXT_LIMIT + 1)).is_err());
    }

    #[test]
    fn crop_region_uses_original_pixels_and_rejects_overflow() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(10, 8, |x, y| {
            image::Rgba([x as u8, y as u8, 17, 255])
        }));
        let cropped = crop_region(
            image,
            Some(super::super::OcrRegion {
                x: 3,
                y: 2,
                width: 4,
                height: 3,
            }),
        )
        .unwrap();
        assert_eq!((cropped.width(), cropped.height()), (4, 3));
        assert_eq!(
            cropped.to_rgba8().get_pixel(0, 0),
            &image::Rgba([3, 2, 17, 255])
        );
        assert!(
            crop_region(
                cropped.clone(),
                Some(super::super::OcrRegion {
                    x: u32::MAX,
                    y: 0,
                    width: 1,
                    height: 1,
                }),
            )
            .is_err()
        );
        let unchanged = crop_region(cropped.clone(), None).unwrap();
        assert_eq!(
            (unchanged.width(), unchanged.height()),
            (cropped.width(), cropped.height())
        );
    }

    #[test]
    fn language_options_are_sorted_and_case_insensitive_duplicates_are_removed() {
        let mut languages = vec![
            RecognizerLanguage {
                tag: "zh-CN".into(),
                display_name: "Chinese (Simplified)".into(),
            },
            RecognizerLanguage {
                tag: "en-US".into(),
                display_name: "English (United States)".into(),
            },
            RecognizerLanguage {
                tag: "ZH-cn".into(),
                display_name: "Zulu duplicate".into(),
            },
        ];
        sort_and_deduplicate_languages(&mut languages);
        assert_eq!(languages.len(), 2);
        assert_eq!(languages[0].tag, "zh-CN");
        assert_eq!(languages[1].tag, "en-US");
    }

    #[test]
    fn language_selection_requires_an_exact_tag_and_never_uses_a_fallback() {
        assert!(language_tag_matches("en-US", "EN-us"));
        assert!(!language_tag_matches("en-GB", "en-US"));
        assert!(!language_tag_matches("zh-Hans-CN", "zh-CN"));
    }

    #[test]
    fn chinese_ocr_spacing_joins_han_runs_but_preserves_latin_boundaries_and_lines() {
        assert_eq!(
            normalize_han_spacing("本 地 OCR 测 试 成 功\n第二 行".into()),
            "本地 OCR 测试成功\n第二行"
        );
        assert_eq!(
            normalize_han_spacing("A B 中 OCR 文".into()),
            "A B 中 OCR 文"
        );
        assert_eq!(normalize_han_spacing("中文".into()), "中文");
    }

    #[test]
    fn ocr_queue_rejects_work_when_its_single_pending_slot_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (first_reply, _first_result) = mpsc::channel();
        let (second_reply, _second_result) = mpsc::channel();
        enqueue_ocr(&sender, OcrCommand::Languages(first_reply)).unwrap();
        assert_eq!(
            enqueue_ocr(&sender, OcrCommand::Languages(second_reply)).unwrap_err(),
            "本机 OCR 队列已满，请稍候重试"
        );
        assert!(matches!(receiver.try_recv(), Ok(OcrCommand::Languages(_))));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn windows_installed_ocr_languages_can_be_enumerated() {
        let languages = available_languages().expect("Windows OCR language enumeration works");
        println!(
            "installed Windows OCR languages: {}",
            languages
                .iter()
                .map(|language| language.tag.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(languages.len() <= LANGUAGE_LIMIT as usize);
        for pair in languages.windows(2) {
            assert!(
                pair[0]
                    .display_name
                    .cmp(&pair[1].display_name)
                    .then_with(|| pair[0].tag.cmp(&pair[1].tag))
                    .is_le()
            );
        }
        for (index, language) in languages.iter().enumerate() {
            assert!(!language.tag.is_empty());
            assert!(!language.display_name.is_empty());
            assert!(
                !languages[..index]
                    .iter()
                    .any(|previous| previous.tag.eq_ignore_ascii_case(&language.tag))
            );
        }
    }

    #[test]
    #[ignore = "requires the en-US Windows OCR component; creates only a synthetic image"]
    fn windows_engine_reads_synthetic_high_contrast_text() {
        let path = std::env::var_os("ZIDEVTOOLS_OCR_FIXTURE")
            .expect("set ZIDEVTOOLS_OCR_FIXTURE to a synthetic high-contrast PNG");
        let bytes = std::fs::read(path).unwrap();
        let picture = super::super::Picture::from_png(&bytes).unwrap();
        let languages = available_languages().expect("Windows OCR languages are available");
        let language = languages
            .iter()
            .find(|language| language.tag.eq_ignore_ascii_case("en-US"))
            .expect("install the en-US Windows OCR component to run this test");
        eprintln!(
            "Testing Windows OCR with installed language {}",
            language.tag
        );
        let text = recognize(&picture, None, Some(language.tag.clone())).unwrap();
        assert!(
            text.to_ascii_uppercase().contains("HELLO"),
            "recognized: {text:?}"
        );
        let repeated = recognize(&picture, None, Some(language.tag.clone())).unwrap();
        assert!(
            repeated.to_ascii_uppercase().contains("HELLO"),
            "repeated recognition: {repeated:?}"
        );
    }
}
