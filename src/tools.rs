use std::{collections::HashMap, fs::File, path::Path};

use anyhow::{Context, Result, anyhow};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeZone};
use qrcode::{Color, QrCode};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolKind {
    #[default]
    Json,
    Base64,
    Url,
    Sha256,
    Timestamp,
    Uuid,
    Jwt,
    Regex,
    Number,
    Qr,
    Color,
    TextStats,
    HtmlEscape,
    TextEscape,
    CaseConvert,
    Yaml,
    Hex,
    Lines,
    UrlInspect,
    Cidr,
    JsonPath,
    JsonDiff,
    DataQuality,
    Cron,
    Random,
    Unicode,
}

impl ToolKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Base64 => "base64",
            Self::Url => "url",
            Self::Sha256 => "sha256",
            Self::Timestamp => "timestamp",
            Self::Uuid => "uuid",
            Self::Jwt => "jwt",
            Self::Regex => "regex",
            Self::Number => "number",
            Self::Qr => "qr",
            Self::Color => "color",
            Self::TextStats => "text-stats",
            Self::HtmlEscape => "html-escape",
            Self::TextEscape => "text-escape",
            Self::CaseConvert => "case-convert",
            Self::Yaml => "yaml",
            Self::Hex => "hex",
            Self::Lines => "lines",
            Self::UrlInspect => "url-inspect",
            Self::Cidr => "cidr",
            Self::JsonPath => "json-path",
            Self::JsonDiff => "json-diff",
            Self::DataQuality => "data-schema",
            Self::Cron => "cron",
            Self::Random => "random",
            Self::Unicode => "unicode",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Json => "格式化、压缩与校验 JSON",
            Self::Base64 => "UTF-8 文本与 Base64 编解码",
            Self::Url => "URL 参数百分号编码与还原",
            Self::Sha256 => "计算文本的 SHA-256 摘要",
            Self::Timestamp => "Unix 时间戳与本地时间转换",
            Self::Uuid => "生成随机 UUID v4",
            Self::Jwt => "查看 Header 与 Payload；不验证签名",
            Self::Regex => "查找匹配与捕获组",
            Self::Number => "二、八、十、十六进制互转",
            Self::Qr => "从文本生成二维码并保存 PNG",
            Self::Color => "HEX、RGB、HSL 颜色表示",
            Self::TextStats => "字符、字节、行数与词段统计",
            Self::HtmlEscape => "HTML 特殊字符转义与还原",
            Self::TextEscape => "JSON 字符串转义与还原",
            Self::CaseConvert => "camel、Pascal、snake 与 kebab 命名",
            Self::Yaml => "YAML 与 JSON 双向转换",
            Self::Hex => "UTF-8 文本与十六进制字节互转",
            Self::Lines => "稳定去重、排序、去空行与空白处理",
            Self::UrlInspect => "解析协议、主机、路径和重复查询参数",
            Self::Cidr => "IPv4 掩码、网络地址与可用范围",
            Self::JsonPath => "提取 JSONPath 子集匹配结果；支持字段、索引与通配符",
            Self::JsonDiff => "比较两个 JSON；对象键顺序无关，可选择数组策略",
            Self::DataQuality => "检查 JSON / CSV / TSV 的空值、重复行与类型分布",
            Self::Cron => "五字段 Cron；预览未来 5 年内最多 10 次运行时间",
            Self::Random => "操作系统安全随机字符串，或可复现的测试数据",
            Self::Unicode => "检查码点、UTF-8、常见不可见字符与 Unicode 规范化",
        }
    }
    pub fn sample(self) -> &'static str {
        match self {
            Self::Json => r#"{"name":"Zi DevTools","local":true,"tools":["JSON","CSV"]}"#,
            Self::Yaml => "name: Zi DevTools\nlocal: true\ntools: [JSON, CSV]",
            Self::Timestamp => "1780000000",
            Self::Number => "255",
            Self::Color => "#538DE8",
            Self::Cidr => "192.168.10.42/24",
            Self::JsonPath => r#"{"items":[{"name":"Rust"},{"name":"Python"}]}"#,
            Self::JsonDiff => r#"{"name":"Zi","tools":["JSON","CSV"],"version":14}"#,
            Self::DataQuality => {
                r#"[{"id":1,"name":"Zi"},{"id":2,"name":" "},{"id":1,"name":"Zi"},{"id":"3"}]"#
            }
            Self::Cron => "*/15 9-18 * * 1-5",
            Self::Random => r#"{"length":24,"count":5,"seed":42}"#,
            Self::Unicode => "Hello\u{200b} Ａ e\u{301}",

            Self::Qr | Self::UrlInspect => "https://example.com/search?q=hello&tag=rust&tag=tools",
            Self::Jwt => "eyJhbGciOiJub25lIn0.eyJzdWIiOiJkZW1vIn0.",
            Self::Regex => "build-2026 release-14 test-42",
            Self::Lines => "Rust\nPython\nRust\n\nTypeScript",
            Self::CaseConvert => "hello local developer tools",
            Self::HtmlEscape => "<p title=\"Zi\">Hello & welcome</p>",
            Self::TextEscape => "Hello\nZi DevTools\t本地工具",
            _ => "Hello, 字与码!",
        }
    }
    pub fn actions(self) -> &'static [&'static str] {
        match self {
            Self::Json => &["格式化", "压缩", "校验"],
            Self::Base64 | Self::Url => &["编码", "解码"],
            Self::Sha256 => &["计算哈希"],
            Self::Timestamp => &["时间戳 → 本地时间", "本地时间 → 秒"],
            Self::Uuid => &["生成 UUID v4"],
            Self::Jwt => &["解析内容"],
            Self::Regex => &["查找匹配"],
            Self::Number => &["转换"],
            Self::Qr => &[],
            Self::Color => &["转换颜色"],
            Self::TextStats => &["统计文本"],
            Self::HtmlEscape => &["转义 HTML", "还原 HTML"],
            Self::TextEscape => &["转义文本", "还原文本"],
            Self::CaseConvert => &["camelCase", "PascalCase", "snake_case", "kebab-case"],
            Self::Yaml => &["YAML → JSON", "JSON → YAML"],
            Self::Hex => &["文本 → Hex", "Hex → 文本"],
            Self::Lines => &["去重", "升序", "降序", "去空行", "去首尾空白"],
            Self::UrlInspect => &["解析 URL"],
            Self::Cidr => &["计算子网"],
            Self::JsonPath => &["提取匹配结果"],
            Self::JsonDiff => &["按数组索引对比", "忽略数组顺序"],
            Self::DataQuality => &["检查 JSON", "检查 CSV", "检查 TSV"],
            Self::Cron => &["预览运行时间"],
            Self::Random => &["安全随机字符串", "种子测试数据（非安全）"],
            Self::Unicode => &["检查字符", "转换 NFC", "转换 NFKC"],
        }
    }
    pub const ALL: [Self; 26] = [
        Self::Json,
        Self::Base64,
        Self::Url,
        Self::Sha256,
        Self::Timestamp,
        Self::Uuid,
        Self::Jwt,
        Self::Regex,
        Self::Number,
        Self::Qr,
        Self::Color,
        Self::TextStats,
        Self::HtmlEscape,
        Self::TextEscape,
        Self::CaseConvert,
        Self::Yaml,
        Self::Hex,
        Self::Lines,
        Self::UrlInspect,
        Self::Cidr,
        Self::JsonPath,
        Self::JsonDiff,
        Self::DataQuality,
        Self::Cron,
        Self::Random,
        Self::Unicode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Base64 => "Base64",
            Self::Url => "URL 编码",
            Self::Sha256 => "SHA-256",
            Self::Timestamp => "时间戳",
            Self::Uuid => "UUID",
            Self::Jwt => "JWT 查看",
            Self::Regex => "正则测试",
            Self::Number => "进制转换",
            Self::Qr => "二维码",
            Self::Color => "颜色转换",
            Self::TextStats => "文本统计",
            Self::HtmlEscape => "HTML 转义",
            Self::TextEscape => "文本转义",
            Self::CaseConvert => "大小写转换",
            Self::Yaml => "YAML / JSON",
            Self::Hex => "UTF-8 / Hex",
            Self::Lines => "文本行整理",
            Self::UrlInspect => "URL 拆解",
            Self::Cidr => "IPv4 子网",
            Self::JsonPath => "JSON 路径查询",
            Self::JsonDiff => "JSON 结构对比",
            Self::DataQuality => "数据质量检查",
            Self::Cron => "Cron 预览",
            Self::Random => "随机数据生成",
            Self::Unicode => "Unicode 检查",
        }
    }

    pub fn secondary_sample(self) -> &'static str {
        match self {
            Self::Regex => r"([a-z]+)-(\d+)",
            Self::JsonPath => "$.items[*].name",
            Self::JsonDiff => r#"{"name":"Zi","tools":["CSV","JSON"],"version":15}"#,
            Self::Cron => "2026-09-26T09:00:00+08:00",
            _ => "",
        }
    }
    pub fn option_label(self) -> Option<&'static str> {
        match self {
            Self::Regex => Some("正则表达式"),
            Self::JsonPath => Some("JSON 路径 · $、.字段、[索引]、[\"字段\"]、[*]"),
            Self::JsonDiff => Some("右侧 JSON · 数组无序模式保留重复次数；数组差异作为整体报告"),
            Self::Cron => Some("参考时间（RFC3339，含时区偏移）· 留空使用当前 UTC"),
            _ => None,
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::JsonPath => "不支持递归、过滤器或切片。未匹配返回 []，所有匹配以 JSON 数组输出。",
            Self::JsonDiff => {
                "差异路径采用 JSON Pointer。无序模式忽略各层数组顺序、保留重复次数，数组差异整体报告。"
            }
            Self::DataQuality => {
                "JSON 需为对象数组；CSV/TSV 首行为列名。缺失与 null 合计，空白字符串单列；CSV 不推断类型。"
            }
            Self::Cron => {
                "分 时 日 月 周；支持数字、*、列表、范围与步长。日和周均不以 * 开头时按 OR；否则按 AND。固定偏移，不模拟夏令时。"
            }
            Self::Random => {
                "输入 JSON 参数 length（1–256）、count（1–100）、alphabet（可选）、seed（仅测试模式）。安全模式使用系统随机源；不保证各类字符必选。"
            }
            Self::Unicode => {
                "按码点而非字形检查，最多 10000 码点。NFC 合并等价字符；NFKC 会折叠兼容字符，可能改变语义。"
            }
            _ => "",
        }
    }
    pub fn is_encoding(self) -> bool {
        matches!(
            self,
            Self::Json
                | Self::Base64
                | Self::Url
                | Self::Sha256
                | Self::Jwt
                | Self::HtmlEscape
                | Self::TextEscape
                | Self::Yaml
                | Self::JsonPath
                | Self::JsonDiff
                | Self::Unicode
                | Self::Hex
        )
    }
}

pub fn run_tool(
    kind: ToolKind,
    action: usize,
    input: &str,
    pattern: &str,
    base: u32,
) -> Result<String> {
    use crate::tools_extra as extra;
    extra::bounded(input)?;
    if action >= kind.actions().len() {
        return Err(anyhow!("无效的工具操作"));
    }
    match kind {
        ToolKind::Json => match action {
            0 => json_format(input),
            1 => json_minify(input),
            _ => json_format(input).map(|_| "JSON 有效".into()),
        },
        ToolKind::Base64 => {
            if action == 0 {
                Ok(base64_encode(input))
            } else {
                base64_decode(input)
            }
        }
        ToolKind::Url => {
            if action == 0 {
                Ok(url_encode(input))
            } else {
                url_decode(input)
            }
        }
        ToolKind::Sha256 => Ok(sha256(input)),
        ToolKind::Timestamp => {
            if action == 0 {
                timestamp_to_local(input)
            } else {
                local_to_timestamp(input)
            }
        }
        ToolKind::Uuid => Ok(generate_uuid()),
        ToolKind::Jwt => inspect_jwt(input),
        ToolKind::Regex => test_regex(pattern, input),
        ToolKind::Number => convert_number(input, base),
        ToolKind::Qr => Err(anyhow!("请使用二维码生成按钮")),
        ToolKind::Color => color_convert(input),
        ToolKind::TextStats => text_stats(input),
        ToolKind::HtmlEscape => Ok(if action == 0 {
            html_escape(input)
        } else {
            html_unescape(input)
        }),
        ToolKind::TextEscape => {
            if action == 0 {
                text_escape(input)
            } else {
                text_unescape(input)
            }
        }
        ToolKind::CaseConvert => Ok(case_convert(
            input,
            ["camel", "pascal", "snake", "kebab"][action],
        )),
        ToolKind::Yaml => {
            if action == 0 {
                extra::yaml_to_json(input)
            } else {
                extra::json_to_yaml(input)
            }
        }
        ToolKind::Hex => {
            if action == 0 {
                extra::hex_encode(input)
            } else {
                extra::hex_decode(input)
            }
        }
        ToolKind::Lines => extra::process_lines(input, kind.actions()[action]),
        ToolKind::UrlInspect => extra::inspect_url(input),
        ToolKind::Cidr => extra::inspect_cidr(input),
        ToolKind::JsonPath => crate::tools_advanced::json_path(input, pattern),
        ToolKind::JsonDiff => crate::tools_advanced::json_diff(input, pattern, action == 1),
        ToolKind::DataQuality => crate::tools_advanced::quality(input, action),
        ToolKind::Cron => crate::tools_advanced::cron(input, pattern),
        ToolKind::Random => crate::tools_advanced::random(input, action == 1),
        ToolKind::Unicode => crate::tools_advanced::unicode(input, action),
    }
}

pub fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn html_unescape(input: &str) -> String {
    input
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

pub fn text_escape(input: &str) -> Result<String> {
    Ok(serde_json::to_string(input)?)
}

pub fn text_unescape(input: &str) -> Result<String> {
    Ok(serde_json::from_str(input.trim())?)
}

pub fn case_convert(input: &str, mode: &str) -> String {
    let words = input
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect::<Vec<_>>();
    match mode {
        "camel" => words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 {
                    word.clone()
                } else {
                    let mut chars = word.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                }
            })
            .collect(),
        "pascal" => words
            .iter()
            .map(|word| {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect(),
        "kebab" => words.join("-"),
        _ => words.join("_"),
    }
}

pub fn color_convert(input: &str) -> Result<String> {
    let value = input.trim().trim_start_matches('#');
    let hex = match value.len() {
        3 if value.chars().all(|ch| ch.is_ascii_hexdigit()) => {
            value.chars().flat_map(|ch| [ch, ch]).collect::<String>()
        }
        6 if value.chars().all(|ch| ch.is_ascii_hexdigit()) => value.to_owned(),
        _ => return Err(anyhow!("请输入 #RGB 或 #RRGGBB 格式的颜色")),
    };
    let rgb = u32::from_str_radix(&hex, 16)?;
    let (r, g, b) = (
        ((rgb >> 16) & 255) as u8,
        ((rgb >> 8) & 255) as u8,
        (rgb & 255) as u8,
    );
    let values = [r, g, b].map(|value| value as f64 / 255.0);
    let max = values.iter().copied().fold(0.0_f64, f64::max);
    let min = values.iter().copied().fold(1.0_f64, f64::min);
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    let saturation = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    let hue = if delta == 0.0 {
        0.0
    } else if max == values[0] {
        60.0 * ((values[1] - values[2]) / delta).rem_euclid(6.0)
    } else if max == values[1] {
        60.0 * ((values[2] - values[0]) / delta + 2.0)
    } else {
        60.0 * ((values[0] - values[1]) / delta + 4.0)
    };
    Ok(format!(
        "HEX  #{r:02X}{g:02X}{b:02X}\nRGB  rgb({r}, {g}, {b})\nHSL  hsl({hue:.0}, {:.0}%, {:.0}%)",
        saturation * 100.0,
        lightness * 100.0
    ))
}

pub fn text_stats(input: &str) -> Result<String> {
    if input.len() > 1024 * 1024 {
        return Err(anyhow!("文本最多 1 MiB"));
    }
    let lines = if input.is_empty() {
        0
    } else {
        input.lines().count() + usize::from(input.ends_with('\n'))
    };
    Ok(format!(
        "UTF-8 字节  {}\nUnicode 字符  {}\n行数  {}\n空白分隔词段  {}",
        input.len(),
        input.chars().count(),
        lines,
        input.split_whitespace().count()
    ))
}

pub struct ToolState {
    pub selected: ToolKind,
    pub input: String,
    pub output: String,
    pub message: String,
    pub pattern: String,
    pub number_base: u32,
    pub qr_image: Option<QrImage>,
    drafts: HashMap<ToolKind, ToolDraft>,
}

#[derive(Default)]
struct ToolDraft {
    input: String,
    output: String,
    message: String,
    pattern: String,
    number_base: u32,
}

impl ToolState {
    pub fn select(&mut self, kind: ToolKind) {
        if self.selected == kind {
            return;
        }
        self.drafts.insert(
            self.selected,
            ToolDraft {
                input: std::mem::take(&mut self.input),
                output: std::mem::take(&mut self.output),
                message: std::mem::take(&mut self.message),
                pattern: std::mem::take(&mut self.pattern),
                number_base: self.number_base,
            },
        );
        let next = self.drafts.remove(&kind).unwrap_or_default();
        self.selected = kind;
        self.input = next.input;
        self.output = next.output;
        self.message = next.message;
        self.pattern = next.pattern;
        self.number_base = if next.number_base == 0 {
            10
        } else {
            next.number_base
        };
    }
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            selected: ToolKind::default(),
            input: String::new(),
            output: String::new(),
            message: String::new(),
            pattern: String::new(),
            number_base: 10,
            qr_image: None,
            drafts: HashMap::new(),
        }
    }
}

pub struct QrImage {
    pub rgba: Vec<u8>,
    pub side: u32,
}

impl QrImage {
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let file = File::create(path).with_context(|| format!("无法创建 {}", path.display()))?;
        let mut encoder = png::Encoder::new(file, self.side, self.side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.rgba)?;
        Ok(())
    }
}

pub fn convert_number(input: &str, base: u32) -> Result<String> {
    if ![2, 8, 10, 16].contains(&base) {
        return Err(anyhow!("只支持 2、8、10、16 进制"));
    }
    let trimmed = input.trim().replace('_', "");
    let (negative, digits) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else {
        (false, trimmed.strip_prefix('+').unwrap_or(&trimmed))
    };
    let digits = match base {
        2 => digits
            .strip_prefix("0b")
            .or_else(|| digits.strip_prefix("0B")),
        8 => digits
            .strip_prefix("0o")
            .or_else(|| digits.strip_prefix("0O")),
        16 => digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X")),
        _ => None,
    }
    .unwrap_or(digits);
    if digits.is_empty() {
        return Err(anyhow!("请输入要转换的整数"));
    }
    let magnitude = u128::from_str_radix(digits, base).context("数字与所选进制不匹配或超出范围")?;
    let value = if negative {
        if magnitude == (i128::MAX as u128) + 1 {
            i128::MIN
        } else {
            -i128::try_from(magnitude).context("数值超出有符号 128 位范围")?
        }
    } else {
        i128::try_from(magnitude).context("数值超出有符号 128 位范围")?
    };
    let sign = if value < 0 { "-" } else { "" };
    let absolute = value.unsigned_abs();
    Ok(format!(
        "十进制  {value}\n二进制  {sign}0b{absolute:b}\n八进制  {sign}0o{absolute:o}\n十六进制  {sign}0x{absolute:X}"
    ))
}

pub fn generate_qr(input: &str) -> Result<QrImage> {
    if input.is_empty() {
        return Err(anyhow!("请输入二维码内容"));
    }
    if input.len() > 2_048 {
        return Err(anyhow!("二维码内容不能超过 2048 字节"));
    }
    let code = QrCode::new(input.as_bytes()).context("内容无法编码为二维码")?;
    let width = code.width();
    const SCALE: usize = 6;
    const QUIET: usize = 4;
    let side = (width + QUIET * 2) * SCALE;
    let mut rgba = vec![255; side * side * 4];
    for (index, color) in code.to_colors().iter().enumerate() {
        if *color == Color::Dark {
            let x0 = (index % width + QUIET) * SCALE;
            let y0 = (index / width + QUIET) * SCALE;
            for y in y0..y0 + SCALE {
                for x in x0..x0 + SCALE {
                    let offset = (y * side + x) * 4;
                    rgba[offset..offset + 3].fill(0);
                }
            }
        }
    }
    Ok(QrImage {
        rgba,
        side: side as u32,
    })
}

pub fn json_format(input: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(input).context("JSON 格式无效")?;
    Ok(serde_json::to_string_pretty(&value)?)
}

pub fn json_minify(input: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(input).context("JSON 格式无效")?;
    Ok(serde_json::to_string(&value)?)
}

pub fn base64_encode(input: &str) -> String {
    STANDARD.encode(input.as_bytes())
}

pub fn base64_decode(input: &str) -> Result<String> {
    let bytes = STANDARD.decode(input.trim()).context("Base64 数据无效")?;
    String::from_utf8(bytes).context("解码结果不是 UTF-8 文本")
}

pub fn url_encode(input: &str) -> String {
    urlencoding::encode(input).into_owned()
}

pub fn url_decode(input: &str) -> Result<String> {
    Ok(urlencoding::decode(input)
        .context("URL 编码无效")?
        .into_owned())
}

pub fn sha256(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

pub fn timestamp_to_local(input: &str) -> Result<String> {
    let raw: i64 = input.trim().parse().context("请输入整数时间戳")?;
    let seconds = if raw.unsigned_abs() > 10_000_000_000 {
        raw / 1_000
    } else {
        raw
    };
    match Local.timestamp_opt(seconds, 0) {
        LocalResult::Single(value) => Ok(value.format("%Y-%m-%d %H:%M:%S %:z").to_string()),
        _ => Err(anyhow!("时间戳超出可转换范围")),
    }
}

pub fn local_to_timestamp(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if let Ok(value) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(value.timestamp().to_string());
    }
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S")
        .context("请输入 YYYY-MM-DD HH:MM:SS 或 RFC 3339 时间")?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.timestamp().to_string()),
        LocalResult::Ambiguous(_, _) => Err(anyhow!("该本地时间处于时区切换歧义区间")),
        LocalResult::None => Err(anyhow!("该本地时间不存在")),
    }
}

pub fn generate_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn inspect_jwt(input: &str) -> Result<String> {
    let parts: Vec<&str> = input.trim().split('.').collect();
    if parts.len() != 3 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(anyhow!("请输入由 header.payload.signature 组成的 JWT"));
    }
    let decode_json = |part: &str| -> Result<serde_json::Value> {
        if part.len() > 64 * 1024 {
            return Err(anyhow!("JWT 单段超过 64 KB 限制"));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(part)
            .context("JWT Base64URL 数据无效")?;
        let value: serde_json::Value =
            serde_json::from_slice(&decoded).context("JWT 中的 JSON 无效")?;
        if !value.is_object() {
            return Err(anyhow!("JWT header 和 payload 必须是 JSON 对象"));
        }
        Ok(value)
    };
    let header = decode_json(parts[0])?;
    let payload = decode_json(parts[1])?;
    Ok(format!(
        "仅解码内容；未验证签名、有效期或可信度。\n\nHeader:\n{}\n\nPayload:\n{}",
        serde_json::to_string_pretty(&header)?,
        serde_json::to_string_pretty(&payload)?
    ))
}

pub fn test_regex(pattern: &str, input: &str) -> Result<String> {
    if pattern.len() > 4_096 || input.len() > 2 * 1024 * 1024 {
        return Err(anyhow!("正则表达式或输入文本超过大小限制"));
    }
    let expression = regex::Regex::new(pattern).context("正则表达式无效")?;
    let mut output = String::new();
    let mut count = 0;
    for captures in expression.captures_iter(input).take(101) {
        if count == 100 {
            output.push_str("\n... 仅展示前 100 个匹配 ...");
            break;
        }
        count += 1;
        let matched = captures.get(0).expect("capture 0 always exists");
        output.push_str(&format!(
            "#{count} 字节 {}..{}: {}\n",
            matched.start(),
            matched.end(),
            clipped(matched.as_str())
        ));
        for group in 1..captures.len() {
            if let Some(value) = captures.get(group) {
                output.push_str(&format!("  ${group}: {}\n", clipped(value.as_str())));
            }
        }
    }
    if count == 0 {
        Ok("没有匹配".to_owned())
    } else {
        Ok(output)
    }
}

fn clipped(value: &str) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_tool_samples_run_and_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for kind in super::ToolKind::ALL {
            assert!(ids.insert(kind.id()));
            if kind == super::ToolKind::Qr {
                continue;
            }
            assert!(
                super::run_tool(kind, 0, kind.sample(), kind.secondary_sample(), 10).is_ok(),
                "{kind:?}"
            );
        }
    }
    use super::*;

    #[test]
    fn formats_and_minifies_json() {
        assert_eq!(json_minify("{ \"a\": 1 }").unwrap(), "{\"a\":1}");
        assert!(json_format("not-json").is_err());
    }

    #[test]
    fn text_codecs_round_trip_unicode() {
        let input = "Zi 开发工具";
        assert_eq!(base64_decode(&base64_encode(input)).unwrap(), input);
        assert_eq!(url_decode(&url_encode(input)).unwrap(), input);
    }

    #[test]
    fn added_text_helpers_round_trip() {
        let input = "<Zi & tools>\"";
        assert_eq!(html_unescape(&html_escape(input)), input);
        let escaped = text_escape(input).unwrap();
        assert_eq!(text_unescape(&escaped).unwrap(), input);
        assert_eq!(case_convert("Zi dev-tools", "camel"), "ziDevTools");
        assert_eq!(case_convert("Zi dev-tools", "pascal"), "ZiDevTools");
        assert!(ToolKind::Json.is_encoding());
        assert!(!ToolKind::CaseConvert.is_encoding());
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(
            sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn jwt_view_decodes_without_claiming_verification() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"zi"}"#);
        let output = inspect_jwt(&format!("{header}.{payload}.fake")).unwrap();
        assert!(output.contains("未验证签名"));
        assert!(output.contains("\"sub\": \"zi\""));
        assert!(inspect_jwt("not-a-jwt").is_err());
    }

    #[test]
    fn regex_lists_captures_and_rejects_invalid_pattern() {
        let output = test_regex(r"(zi)-(\d+)", "zi-42 and zi-7").unwrap();
        assert!(output.contains("#1 字节 0..5"));
        assert!(output.contains("$2: 42"));
        assert!(output.contains("#2 字节 10..14"));
        assert!(test_regex("(", "text").is_err());
    }

    #[test]
    fn converts_signed_numbers_between_common_bases() {
        let output = convert_number("-0x2A", 16).unwrap();
        assert!(output.contains("十进制  -42"));
        assert!(output.contains("二进制  -0b101010"));
        assert!(output.contains("八进制  -0o52"));
        assert!(convert_number("2", 2).is_err());
        assert!(convert_number("", 10).is_err());
        assert!(convert_number(&i128::MIN.to_string(), 10).is_ok());
    }

    #[test]
    fn converts_colors_and_reports_text_statistics() {
        let color = color_convert("#f80").unwrap();
        assert!(color.contains("#FF8800"));
        assert!(color.contains("rgb(255, 136, 0)"));
        assert!(color_convert("#xyz").is_err());
        let stats = text_stats("你好 zi\n").unwrap();
        assert!(stats.contains("UTF-8 字节  10"));
        assert!(stats.contains("Unicode 字符  6"));
        assert!(stats.contains("行数  2"));
    }

    #[test]
    fn each_small_tool_keeps_its_own_draft() {
        let mut state = ToolState {
            input: "json draft".into(),
            ..Default::default()
        };
        state.select(ToolKind::Color);
        assert!(state.input.is_empty());
        state.input = "#f80".into();
        state.select(ToolKind::Json);
        assert_eq!(state.input, "json draft");
        state.select(ToolKind::Color);
        assert_eq!(state.input, "#f80");
    }

    #[test]
    fn renders_qr_with_quiet_zone_and_writes_png() {
        let image = generate_qr("Zi 开发工具").unwrap();
        assert_eq!(
            image.rgba.len(),
            image.side as usize * image.side as usize * 4
        );
        assert_eq!(&image.rgba[..4], &[255, 255, 255, 255]);
        assert!(
            image
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [0, 0, 0, 255])
        );
        let path = std::env::temp_dir().join(format!("zi-qr-{}.png", uuid::Uuid::new_v4()));
        image.save_png(&path).unwrap();
        assert!(
            std::fs::read(&path)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        std::fs::remove_file(path).unwrap();
        assert!(generate_qr("").is_err());
    }
}
