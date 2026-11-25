//! Shared, bounded prompt variable extraction and expansion.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const MAX_RENDER_BYTES: usize = 128 * 1024;
pub const MAX_VARIABLES: usize = 12;

enum Part<'a> {
    Text(&'a str),
    Literal(&'static str),
    Variable(&'a str),
}

fn valid_variable(name: &str) -> bool {
    let mut chars = name.bytes();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn parts(body: &str) -> Result<Vec<Part<'_>>> {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let rest = &body[index..];
        // Both delimiters are ASCII. A match starts on a UTF-8 boundary, and
        // scanning only to the next match avoids repeated suffix searches.
        let Some(next) = rest
            .as_bytes()
            .windows(2)
            .position(|pair| matches!(pair, b"{{" | b"}}"))
        else {
            parts.push(Part::Text(rest));
            break;
        };
        if next > 0 {
            parts.push(Part::Text(&rest[..next]));
            index += next;
        }
        let rest = &rest[next..];
        if rest.starts_with("{{{{") {
            parts.push(Part::Literal("{{"));
            index += 4;
        } else if rest.starts_with("}}}}") {
            parts.push(Part::Literal("}}"));
            index += 4;
        } else if let Some(after_open) = rest.strip_prefix("{{") {
            let Some(end) = after_open.find("}}") else {
                anyhow::bail!("变量占位符缺少结束双花括号");
            };
            let name = &after_open[..end];
            ensure!(
                valid_variable(name),
                "变量名只能使用字母、数字和下划线，且不能以数字开头"
            );
            parts.push(Part::Variable(name));
            index += 2 + end + 2;
        } else {
            anyhow::bail!("出现未配对的结束双花括号；字面量请写成 }}}}");
        }
    }
    Ok(parts)
}

pub struct Template<'a> {
    parts: Vec<Part<'a>>,
    names: Vec<String>,
}

impl Template<'_> {
    pub fn variable_names(&self) -> &[String] {
        &self.names
    }

    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String> {
        let mut output = String::new();
        for part in &self.parts {
            let text = match *part {
                Part::Text(text) | Part::Literal(text) => text,
                Part::Variable(name) => {
                    let value = values.get(name).filter(|value| !value.trim().is_empty());
                    let Some(value) = value else {
                        anyhow::bail!("请填写变量 {name}");
                    };
                    value
                }
            };
            ensure!(
                text.len() <= MAX_RENDER_BYTES - output.len(),
                "变量展开结果超过 128 KiB"
            );
            output.push_str(text);
        }
        Ok(output)
    }
}

pub fn parse(body: &str) -> Result<Template<'_>> {
    ensure!(!body.trim().is_empty(), "模板正文不能为空");
    ensure!(body.len() <= MAX_BODY_BYTES, "模板正文超过 16 KiB");
    ensure!(
        !body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
        "模板正文含不支持的控制字符"
    );
    let parts = parts(body)?;
    let names: BTreeSet<&str> = parts
        .iter()
        .filter_map(|part| match part {
            Part::Variable(name) => Some(*name),
            _ => None,
        })
        .collect();
    ensure!(names.len() <= MAX_VARIABLES, "模板最多包含 12 个不同变量");
    Ok(Template {
        parts,
        names: names.into_iter().map(str::to_owned).collect(),
    })
}

pub fn variables(body: &str) -> Result<Vec<String>> {
    Ok(parse(body)?.names)
}

pub fn render(body: &str, values: &BTreeMap<String, String>) -> Result<String> {
    parse(body)?.render(values)
}
