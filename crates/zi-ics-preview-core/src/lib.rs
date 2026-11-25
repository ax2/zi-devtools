//! Shared lexical iCalendar parsing for the native planner and Studio preview.

use anyhow::{Context, Result, bail, ensure};
use ical::property::Property;
use std::io::Cursor;

pub mod count;
pub mod preview;
pub mod recurrence;

pub fn text_value(value: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        result.push(match chars.next() {
            Some('n' | 'N') => '\n',
            Some(ch @ ('\\' | ',' | ';')) => ch,
            _ => bail!("ICS 含无效的文本转义"),
        });
    }
    Ok(result)
}
pub fn prop<'a>(props: &'a [Property], name: &str) -> Result<Option<&'a Property>> {
    let mut found = props.iter().filter(|p| p.name == name);
    let value = found.next();
    ensure!(
        found.next().is_none(),
        "ICS 属性 {name} 重复，无法确定使用哪一个"
    );
    Ok(value)
}
pub fn value<'a>(props: &'a [Property], name: &str) -> Result<Option<&'a str>> {
    Ok(prop(props, name)?.and_then(|p| p.value.as_deref()))
}
/// Preserve whitespace while unfolding, then let ical parse quoted parameters.
/// ical's line reader trims trailing whitespace; a removable suffix prevents it.
pub fn properties(
    text: &str,
    max_bytes: usize,
    max_lines: usize,
    max_property_bytes: usize,
) -> Result<Vec<Property>> {
    ensure!(
        text.len() <= max_bytes && !text.contains('\0'),
        "ICS 超过 16 MiB 或包含空字节"
    );
    let mut lines: Vec<String> = Vec::new();
    for (n, raw) in text.trim_start_matches('\u{feff}').split('\n').enumerate() {
        ensure!(n < max_lines, "ICS 行数超过 100000，请拆分文件");
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        ensure!(
            !line.chars().any(|c| c.is_control() && c != '\t'),
            "ICS 包含无效控制字符"
        );
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let last = lines.last_mut().context("ICS 第一行不能是续行")?;
            last.push_str(&line[1..]);
            ensure!(last.len() <= max_property_bytes, "ICS 单个属性过大");
        } else {
            ensure!(line.len() <= max_property_bytes, "ICS 单个属性过大");
            lines.push(line.to_owned());
        }
    }
    let mut protected = String::new();
    for line in lines {
        protected.push_str(&line);
        protected.push_str("\u{e000}\r\n");
    }
    ical::PropertyParser::from_reader(Cursor::new(protected))
        .map(|p| {
            let mut p = p.context("ICS 属性格式无效")?;
            ensure!(
                !p.name.is_empty()
                    && p.name.len() <= 64
                    && p.name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "ICS 属性名无效"
            );
            let value = p.value.as_mut().context("ICS 属性缺少值")?;
            ensure!(value.pop() == Some('\u{e000}'), "ICS 属性解析失败");
            Ok(p)
        })
        .collect()
}
