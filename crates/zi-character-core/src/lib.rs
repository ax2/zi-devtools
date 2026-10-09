//! Shared character algorithms; resource acquisition and mutations belong to the caller.
const CONTROL: [&str; 33] = [
    "NUL 空字符",
    "SOH 标题开始",
    "STX 正文开始",
    "ETX 正文结束",
    "EOT 传输结束",
    "ENQ 询问",
    "ACK 确认",
    "BEL 响铃",
    "BS 退格",
    "HT 制表符",
    "LF 换行",
    "VT 垂直制表",
    "FF 换页",
    "CR 回车",
    "SO 移出",
    "SI 移入",
    "DLE 数据链路转义",
    "DC1 设备控制 1",
    "DC2 设备控制 2",
    "DC3 设备控制 3",
    "DC4 设备控制 4",
    "NAK 否认",
    "SYN 同步",
    "ETB 块结束",
    "CAN 取消",
    "EM 媒体结束",
    "SUB 替换",
    "ESC 转义",
    "FS 文件分隔",
    "GS 组分隔",
    "RS 记录分隔",
    "US 单元分隔",
    "DEL 删除",
];

pub fn ascii_name(code: u8) -> String {
    match code {
        0..=31 => CONTROL[code as usize].to_owned(),
        32 => "SPACE 空格".to_owned(),
        33..=126 => (code as char).to_string(),
        127 => CONTROL[32].to_owned(),
        _ => "非 ASCII".to_owned(),
    }
}

pub fn ascii_matches(code: u8, name: &str, query: &str) -> bool {
    if let Some(hex) = query
        .strip_prefix("0x")
        .or_else(|| query.strip_prefix("0X"))
        && let Ok(value) = u8::from_str_radix(hex, 16)
    {
        return code == value;
    }
    if let Ok(value) = query.parse::<u8>() {
        return code == value;
    }
    if query.len() == 1 && query.as_bytes()[0].is_ascii_graphic() {
        return code == query.as_bytes()[0];
    }
    name.to_lowercase().contains(&query.to_lowercase())
}

#[derive(Debug, PartialEq, Eq)]
pub struct Symbol {
    pub category: &'static str,
    pub name: &'static str,
    pub value: &'static str,
}

pub const SYMBOLS: &[Symbol] = &[
    Symbol {
        category: "标点与排版",
        name: "省略号",
        value: "…",
    },
    Symbol {
        category: "标点与排版",
        name: "项目符号",
        value: "•",
    },
    Symbol {
        category: "标点与排版",
        name: "破折号",
        value: "—",
    },
    Symbol {
        category: "标点与排版",
        name: "引号",
        value: "“”",
    },
    Symbol {
        category: "标点与排版",
        name: "书名号",
        value: "《》",
    },
    Symbol {
        category: "标点与排版",
        name: "版权",
        value: "©",
    },
    Symbol {
        category: "标点与排版",
        name: "注册商标",
        value: "®",
    },
    Symbol {
        category: "数学与单位",
        name: "约等于",
        value: "≈",
    },
    Symbol {
        category: "数学与单位",
        name: "不等于",
        value: "≠",
    },
    Symbol {
        category: "数学与单位",
        name: "大于等于",
        value: "≥",
    },
    Symbol {
        category: "数学与单位",
        name: "小于等于",
        value: "≤",
    },
    Symbol {
        category: "数学与单位",
        name: "无穷",
        value: "∞",
    },
    Symbol {
        category: "数学与单位",
        name: "度",
        value: "°",
    },
    Symbol {
        category: "数学与单位",
        name: "乘号",
        value: "×",
    },
    Symbol {
        category: "数学与单位",
        name: "除号",
        value: "÷",
    },
    Symbol {
        category: "箭头与状态",
        name: "右箭头",
        value: "→",
    },
    Symbol {
        category: "箭头与状态",
        name: "左箭头",
        value: "←",
    },
    Symbol {
        category: "箭头与状态",
        name: "双向箭头",
        value: "↔",
    },
    Symbol {
        category: "箭头与状态",
        name: "勾选",
        value: "✓",
    },
    Symbol {
        category: "箭头与状态",
        name: "叉号",
        value: "✕",
    },
    Symbol {
        category: "箭头与状态",
        name: "警告",
        value: "⚠",
    },
    Symbol {
        category: "箭头与状态",
        name: "信息",
        value: "ℹ",
    },
    Symbol {
        category: "表情",
        name: "笑脸",
        value: "😀",
    },
    Symbol {
        category: "表情",
        name: "微笑",
        value: "😊",
    },
    Symbol {
        category: "表情",
        name: "思考",
        value: "🤔",
    },
    Symbol {
        category: "表情",
        name: "火",
        value: "🔥",
    },
    Symbol {
        category: "表情",
        name: "庆祝",
        value: "🎉",
    },
    Symbol {
        category: "表情",
        name: "点赞",
        value: "👍",
    },
    Symbol {
        category: "表情",
        name: "眼睛",
        value: "👀",
    },
    Symbol {
        category: "表情",
        name: "火箭",
        value: "🚀",
    },
    Symbol {
        category: "颜文字",
        name: "开心",
        value: "(＾▽＾)",
    },
    Symbol {
        category: "颜文字",
        name: "害羞",
        value: "(⁄ ⁄•⁄ω⁄•⁄ ⁄)",
    },
    Symbol {
        category: "颜文字",
        name: "困惑",
        value: "(・_・?)",
    },
    Symbol {
        category: "颜文字",
        name: "惊讶",
        value: "Σ(°△°|||)",
    },
    Symbol {
        category: "颜文字",
        name: "加油",
        value: "(ง •̀_•́)ง",
    },
    Symbol {
        category: "颜文字",
        name: "挥手",
        value: "ヾ(￣▽￣)",
    },
    Symbol {
        category: "颜文字",
        name: "耸肩",
        value: "¯\\_(ツ)_/¯",
    },
];

// Five columns by seven rows, uppercase Latin letters, digits and common punctuation.
fn glyph(ch: char) -> Option<[&'static str; 7]> {
    Some(match ch {
        'A' => [
            " ### ", "#   #", "#   #", "#####", "#   #", "#   #", "#   #",
        ],
        'B' => [
            "#### ", "#   #", "#   #", "#### ", "#   #", "#   #", "#### ",
        ],
        'C' => [
            " ####", "#    ", "#    ", "#    ", "#    ", "#    ", " ####",
        ],
        'D' => [
            "#### ", "#   #", "#   #", "#   #", "#   #", "#   #", "#### ",
        ],
        'E' => [
            "#####", "#    ", "#    ", "#### ", "#    ", "#    ", "#####",
        ],
        'F' => [
            "#####", "#    ", "#    ", "#### ", "#    ", "#    ", "#    ",
        ],
        'G' => [
            " ####", "#    ", "#    ", "#  ##", "#   #", "#   #", " ####",
        ],
        'H' => [
            "#   #", "#   #", "#   #", "#####", "#   #", "#   #", "#   #",
        ],
        'I' => [
            "#####", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", "#####",
        ],
        'J' => [
            "#####", "    #", "    #", "    #", "#   #", "#   #", " ### ",
        ],
        'K' => [
            "#   #", "#  # ", "# #  ", "##   ", "# #  ", "#  # ", "#   #",
        ],
        'L' => [
            "#    ", "#    ", "#    ", "#    ", "#    ", "#    ", "#####",
        ],
        'M' => [
            "#   #", "## ##", "# # #", "# # #", "#   #", "#   #", "#   #",
        ],
        'N' => [
            "#   #", "##  #", "##  #", "# # #", "#  ##", "#  ##", "#   #",
        ],
        'O' => [
            " ### ", "#   #", "#   #", "#   #", "#   #", "#   #", " ### ",
        ],
        'P' => [
            "#### ", "#   #", "#   #", "#### ", "#    ", "#    ", "#    ",
        ],
        'Q' => [
            " ### ", "#   #", "#   #", "#   #", "# # #", "#  # ", " ## #",
        ],
        'R' => [
            "#### ", "#   #", "#   #", "#### ", "# #  ", "#  # ", "#   #",
        ],
        'S' => [
            " ####", "#    ", "#    ", " ### ", "    #", "    #", "#### ",
        ],
        'T' => [
            "#####", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ",
        ],
        'U' => [
            "#   #", "#   #", "#   #", "#   #", "#   #", "#   #", " ### ",
        ],
        'V' => [
            "#   #", "#   #", "#   #", "#   #", "#   #", " # # ", "  #  ",
        ],
        'W' => [
            "#   #", "#   #", "#   #", "# # #", "# # #", "## ##", "#   #",
        ],
        'X' => [
            "#   #", "#   #", " # # ", "  #  ", " # # ", "#   #", "#   #",
        ],
        'Y' => [
            "#   #", "#   #", " # # ", "  #  ", "  #  ", "  #  ", "  #  ",
        ],
        'Z' => [
            "#####", "    #", "   # ", "  #  ", " #   ", "#    ", "#####",
        ],
        '0' => [
            " ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### ",
        ],
        '1' => [
            "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", "#####",
        ],
        '2' => [
            " ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####",
        ],
        '3' => [
            "#### ", "    #", "    #", " ### ", "    #", "    #", "#### ",
        ],
        '4' => [
            "   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # ",
        ],
        '5' => [
            "#####", "#    ", "#    ", "#### ", "    #", "    #", "#### ",
        ],
        '6' => [
            " ### ", "#    ", "#    ", "#### ", "#   #", "#   #", " ### ",
        ],
        '7' => [
            "#####", "    #", "   # ", "  #  ", " #   ", " #   ", " #   ",
        ],
        '8' => [
            " ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### ",
        ],
        '9' => [
            " ### ", "#   #", "#   #", " ####", "    #", "    #", " ### ",
        ],
        ' ' => ["     "; 7],
        '-' => [
            "     ", "     ", "     ", "#####", "     ", "     ", "     ",
        ],
        '.' => [
            "     ", "     ", "     ", "     ", "     ", " ##  ", " ##  ",
        ],
        '!' => [
            "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", "     ", "  #  ",
        ],
        '?' => [
            " ### ", "#   #", "    #", "   # ", "  #  ", "     ", "  #  ",
        ],
        _ => return None,
    })
}

pub fn banner(input: &str, ink: char) -> (String, usize) {
    let mut output = String::new();
    let mut unsupported = 0;
    let chars: Vec<_> = input.chars().take(40).collect();
    for row in 0..7 {
        for ch in &chars {
            let pattern = glyph(ch.to_ascii_uppercase())
                .or_else(|| {
                    unsupported += usize::from(row == 0);
                    glyph('?')
                })
                .expect("question-mark glyph exists");
            for pixel in pattern[row].chars() {
                output.push(if pixel == '#' { ink } else { ' ' });
            }
            output.push(' ');
        }
        output.push('\n');
    }
    (output, unsupported)
}

pub fn rgba_art(source: &image::RgbaImage, width: u32) -> anyhow::Result<String> {
    use anyhow::ensure;
    ensure!((8..=120).contains(&width), "宽度需为 8–120 字符");
    let (source_width, source_height) = source.dimensions();
    ensure!(
        source_width > 0
            && source_height > 0
            && u64::from(source_width) * u64::from(source_height) <= 20_000_000,
        "图片像素超过上限"
    );
    let height = ((source_height as f64 / source_width as f64 * width as f64 * 0.5).round() as u32)
        .clamp(1, 120);
    let resized =
        image::imageops::resize(source, width, height, image::imageops::FilterType::Triangle);
    let ramp = b"@%#*+=-:. ";
    let mut output = String::with_capacity((width as usize + 1) * height as usize);
    for y in 0..height {
        for x in 0..width {
            let pixel = resized.get_pixel(x, y).0;
            let alpha = pixel[3] as u32;
            let channels = [pixel[0], pixel[1], pixel[2]]
                .map(|value| (value as u32 * alpha + 255 * (255 - alpha)) / 255);
            let luminance = (channels[0] * 2126 + channels[1] * 7152 + channels[2] * 722) / 10_000;
            output.push(ramp[(luminance as usize * (ramp.len() - 1)) / 255] as char);
        }
        output.push('\n');
    }
    Ok(output)
}

pub const CATEGORIES: [&str; 6] = [
    "全部",
    "标点与排版",
    "数学与单位",
    "箭头与状态",
    "表情",
    "颜文字",
];

/// Category and query are combined; values are returned intact, including combining marks.
pub fn search_symbols(query: &str, category: &str) -> Vec<&'static Symbol> {
    let query = query.trim().to_lowercase();
    SYMBOLS
        .iter()
        .filter(|symbol| {
            (category == "全部" || symbol.category == category)
                && (query.is_empty()
                    || format!("{} {} {}", symbol.name, symbol.value, symbol.category)
                        .to_lowercase()
                        .contains(&query))
        })
        .collect()
}

/// Decode only supplied bytes. No filesystem, clipboard, network or Host access.
pub fn image_art_bytes(bytes: &[u8], width: u32) -> anyhow::Result<String> {
    use anyhow::{Context, ensure};
    use std::io::Cursor;
    ensure!((8..=120).contains(&width), "宽度需为 8–120 字符");
    ensure!(bytes.len() <= 10 * 1024 * 1024, "图片超过 10 MiB 上限");
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
        ),
        "仅支持 PNG/JPEG/WebP"
    );
    let (w, h) = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .into_dimensions()
        .context("无法读取图片尺寸")?;
    ensure!(
        w > 0 && h > 0 && u64::from(w) * u64::from(h) <= 20_000_000,
        "图片像素超过上限"
    );
    rgba_art(&reader.decode()?.to_rgba8(), width)
}

#[cfg(test)]
mod tests;
