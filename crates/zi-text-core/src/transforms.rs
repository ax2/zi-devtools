//! Shared standalone/compute algorithms. No host, files, network or clock.
use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;
use std::collections::HashSet;

fn bounded(input: &str) -> Result<()> {
    if input.len() > 1024 * 1024 {
        bail!("输入超过 1 MiB，请先缩小内容范围");
    }
    Ok(())
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

pub fn url_encode(input: &str) -> String {
    urlencoding::encode(input).into_owned()
}

pub fn url_decode(input: &str) -> Result<String> {
    Ok(urlencoding::decode(input)
        .context("URL 编码无效")?
        .into_owned())
}

pub fn hex_encode(input: &str) -> Result<String> {
    bounded(input)?;
    Ok(input
        .as_bytes()
        .iter()
        .map(|v| format!("{v:02X}"))
        .collect::<Vec<_>>()
        .join(" "))
}

pub fn hex_decode(input: &str) -> Result<String> {
    bounded(input)?;
    let value: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if !value.is_ascii() || !value.bytes().all(|c| c.is_ascii_hexdigit()) || value.len() % 2 != 0 {
        bail!("请输入成对的十六进制字符，可用空格或换行分隔，例如 E4 BD A0");
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    String::from_utf8(bytes)
        .map_err(|_| anyhow!("字节不是有效的 UTF-8 文本；不会替换或丢弃无效字节"))
}

pub fn process_lines(input: &str, action: &str) -> Result<String> {
    bounded(input)?;
    let mut lines = input.lines().map(str::to_owned).collect::<Vec<_>>();
    match action {
        "去重" => {
            let mut seen = HashSet::new();
            lines.retain(|line| seen.insert(line.clone()));
        }
        "升序" => lines.sort(),
        "降序" => lines.sort_by(|a, b| b.cmp(a)),
        "去空行" => lines.retain(|line| !line.trim().is_empty()),
        "去首尾空白" => lines
            .iter_mut()
            .for_each(|line| *line = line.trim().to_owned()),
        _ => bail!("未知行处理操作"),
    }
    Ok(lines.join("\n"))
}

pub fn inspect_url(input: &str) -> Result<String> {
    bounded(input)?;
    let url = url::Url::parse(input.trim())
        .map_err(|e| anyhow!("URL 无效：{e}；请包含 https:// 等协议"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "scheme":url.scheme(),"host":url.host_str(),"port":url.port_or_known_default(),
        "path":url.path(),"fragment":url.fragment(),
        "credentials_present": !url.username().is_empty() || url.password().is_some(),
        "query":url.query_pairs().map(|(k,v)| json!({"key":k,"value":v})).collect::<Vec<_>>()
    }))?)
}

#[derive(Clone, Copy)]
pub struct Action {
    pub id: &'static str,
    pub source_tool_id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
}
pub const ACTIONS: &[Action] = &[
    Action {
        id: "url.encode",
        source_tool_id: "url",
        title: "URL 编码",
        group: "编码",
    },
    Action {
        id: "url.decode",
        source_tool_id: "url",
        title: "URL 解码",
        group: "编码",
    },
    Action {
        id: "html.escape",
        source_tool_id: "html-escape",
        title: "HTML 转义",
        group: "编码",
    },
    Action {
        id: "html.unescape",
        source_tool_id: "html-escape",
        title: "HTML 还原",
        group: "编码",
    },
    Action {
        id: "string.escape",
        source_tool_id: "text-escape",
        title: "JSON 字符串转义",
        group: "编码",
    },
    Action {
        id: "string.unescape",
        source_tool_id: "text-escape",
        title: "JSON 字符串还原",
        group: "编码",
    },
    Action {
        id: "case.camel",
        source_tool_id: "case-convert",
        title: "camelCase",
        group: "文本",
    },
    Action {
        id: "case.pascal",
        source_tool_id: "case-convert",
        title: "PascalCase",
        group: "文本",
    },
    Action {
        id: "case.snake",
        source_tool_id: "case-convert",
        title: "snake_case",
        group: "文本",
    },
    Action {
        id: "case.kebab",
        source_tool_id: "case-convert",
        title: "kebab-case",
        group: "文本",
    },
    Action {
        id: "stats.inspect",
        source_tool_id: "text-stats",
        title: "文本统计",
        group: "文本",
    },
    Action {
        id: "lines.deduplicate",
        source_tool_id: "lines",
        title: "去重",
        group: "文本",
    },
    Action {
        id: "lines.ascending",
        source_tool_id: "lines",
        title: "升序",
        group: "文本",
    },
    Action {
        id: "lines.descending",
        source_tool_id: "lines",
        title: "降序",
        group: "文本",
    },
    Action {
        id: "lines.nonempty",
        source_tool_id: "lines",
        title: "去空行",
        group: "文本",
    },
    Action {
        id: "lines.trim",
        source_tool_id: "lines",
        title: "去首尾空白",
        group: "文本",
    },
    Action {
        id: "number.from2",
        source_tool_id: "number",
        title: "2 进制输入",
        group: "数值",
    },
    Action {
        id: "number.from8",
        source_tool_id: "number",
        title: "8 进制输入",
        group: "数值",
    },
    Action {
        id: "number.from10",
        source_tool_id: "number",
        title: "10 进制输入",
        group: "数值",
    },
    Action {
        id: "number.from16",
        source_tool_id: "number",
        title: "16 进制输入",
        group: "数值",
    },
    Action {
        id: "hex.encode",
        source_tool_id: "hex",
        title: "UTF-8 → Hex",
        group: "编码",
    },
    Action {
        id: "hex.decode",
        source_tool_id: "hex",
        title: "Hex → UTF-8",
        group: "编码",
    },
    Action {
        id: "url.inspect",
        source_tool_id: "url-inspect",
        title: "URL 拆解",
        group: "结构",
    },
];
pub fn run(id: &str, input: &str) -> Result<String> {
    bounded(input)?;
    match id {
        "url.encode" => Ok(url_encode(input)),
        "url.decode" => url_decode(input),
        "html.escape" => Ok(html_escape(input)),
        "html.unescape" => Ok(html_unescape(input)),
        "string.escape" => text_escape(input),
        "string.unescape" => text_unescape(input),
        "case.camel" => Ok(case_convert(input, "camel")),
        "case.pascal" => Ok(case_convert(input, "pascal")),
        "case.snake" => Ok(case_convert(input, "snake")),
        "case.kebab" => Ok(case_convert(input, "kebab")),
        "stats.inspect" => text_stats(input),
        "lines.deduplicate" => process_lines(input, "去重"),
        "lines.ascending" => process_lines(input, "升序"),
        "lines.descending" => process_lines(input, "降序"),
        "lines.nonempty" => process_lines(input, "去空行"),
        "lines.trim" => process_lines(input, "去首尾空白"),
        "number.from2" => convert_number(input, 2),
        "number.from8" => convert_number(input, 8),
        "number.from10" => convert_number(input, 10),
        "number.from16" => convert_number(input, 16),
        "hex.encode" => hex_encode(input),
        "hex.decode" => hex_decode(input),
        "url.inspect" => inspect_url(input),
        _ => bail!("未知文本转换操作"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_order_and_unicode_naming_are_preserved() {
        assert_eq!(html_escape("<&\"'中"), "&lt;&amp;&quot;&#39;中");
        assert_eq!(html_unescape("&amp;lt; &#x41;"), "&lt; &#x41;");
        assert_eq!(case_convert("HELLO-world_中", "camel"), "helloWorld中");
        assert_eq!(case_convert("alreadyCamel", "snake"), "alreadycamel");
        assert_eq!(case_convert("ß_word", "pascal"), "SSWord");
    }
    #[test]
    fn byte_decoding_never_discards_invalid_utf8() {
        assert_eq!(
            hex_decode(&hex_encode("你好🙂\n").unwrap()).unwrap(),
            "你好🙂\n"
        );
        for value in ["F", "FF", "GG", "中", "C0 AF"] {
            assert!(hex_decode(value).is_err());
        }
        assert_eq!(url_encode("a+b 中"), "a%2Bb%20%E4%B8%AD");
        assert_eq!(url_decode("a+b%20中").unwrap(), "a+b 中");
        assert_eq!(url_decode("%GG%").unwrap(), "%GG%");
        assert!(url_decode("%FF").is_err());
    }
    #[test]
    fn full_signed_128_bit_radix_and_range_are_preserved() {
        let min = i128::MIN.to_string();
        assert!(convert_number(&min, 10).unwrap().contains(&min));
        assert!(
            convert_number("+0XFF_FF", 16)
                .unwrap()
                .starts_with("十进制  65535")
        );
        assert!(convert_number("-0", 10).unwrap().starts_with("十进制  0"));
        for (value, base) in [
            ("170141183460469231731687303715884105728", 10),
            ("-170141183460469231731687303715884105729", 10),
            ("2", 2),
            ("0x", 16),
            ("1", 3),
        ] {
            assert!(convert_number(value, base).is_err());
        }
    }
    #[test]
    fn line_endings_and_duplicate_stability_are_preserved() {
        assert_eq!(process_lines("b\r\na\r\nb\r\n", "去重").unwrap(), "b\na");
        assert_eq!(process_lines(" a \n\t\n中\n", "去空行").unwrap(), " a \n中");
        assert_eq!(process_lines(" b \n a ", "去首尾空白").unwrap(), "b\na");
        assert_eq!(
            text_stats("中\r\n").unwrap(),
            "UTF-8 字节  5\nUnicode 字符  3\n行数  2\n空白分隔词段  1"
        );
    }
    #[test]
    fn url_structure_preserves_repeated_queries_without_credentials() {
        let value: serde_json::Value = serde_json::from_str(
            &inspect_url("https://user:pass@example.com/p?a=1&a=2&q=%E4%B8%AD#f").unwrap(),
        )
        .unwrap();
        assert_eq!(value["query"].as_array().unwrap().len(), 3);
        assert_eq!(value["query"][2]["value"], "中");
        assert_eq!(value["port"], 443);
        assert_eq!(value["credentials_present"], true);
        assert!(value.get("password").is_none());
        assert!(inspect_url("/relative").is_err());
    }
}
