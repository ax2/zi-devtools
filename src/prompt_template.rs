//! Bounded local prompt templates. Rendering never executes a model request.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const MAX_RENDER_BYTES: usize = 128 * 1024;
pub const MAX_VARIABLES: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    pub tool_id: String,
    pub model: String,
    pub stream: bool,
    pub multi_turn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub saved_at: String,
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
    pub options: RunOptions,
}

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
        let open = rest.find("{{");
        let close = rest.find("}}");
        let next = match (open, close) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => {
                parts.push(Part::Text(rest));
                break;
            }
        };
        if next > 0 {
            parts.push(Part::Text(&rest[..next]));
            index += next;
            continue;
        }
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

pub fn variables(body: &str) -> Result<Vec<String>> {
    ensure!(!body.trim().is_empty(), "模板正文不能为空");
    ensure!(body.len() <= MAX_BODY_BYTES, "模板正文超过 16 KiB");
    ensure!(
        !body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
        "模板正文含不支持的控制字符"
    );
    let names: BTreeSet<String> = parts(body)?
        .into_iter()
        .filter_map(|part| match part {
            Part::Variable(name) => Some(name.into()),
            _ => None,
        })
        .collect();
    ensure!(names.len() <= MAX_VARIABLES, "模板最多包含 12 个不同变量");
    Ok(names.into_iter().collect())
}

pub fn render(body: &str, values: &BTreeMap<String, String>) -> Result<String> {
    variables(body)?;
    let mut output = String::new();
    for part in parts(body)? {
        match part {
            Part::Text(text) | Part::Literal(text) => output.push_str(text),
            Part::Variable(name) => {
                let value = values.get(name).filter(|v| !v.trim().is_empty());
                let Some(value) = value else {
                    anyhow::bail!("请填写变量 {name}");
                };
                output.push_str(value);
            }
        }
        ensure!(output.len() <= MAX_RENDER_BYTES, "变量展开结果超过 128 KiB");
    }
    Ok(output)
}

pub fn parse_tags(raw: &str) -> Result<Vec<String>> {
    let mut tags = Vec::new();
    let mut seen = BTreeSet::new();
    for tag in raw.split([',', '，']) {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        ensure!(
            tag.chars().count() <= 24 && !tag.chars().any(char::is_control),
            "标签最多 24 字且不能包含控制字符"
        );
        ensure!(seen.insert(tag.to_lowercase()), "标签不能重复");
        tags.push(tag.into());
    }
    ensure!(tags.len() <= 8, "每份模板最多 8 个标签");
    Ok(tags)
}

impl RunOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.tool_id.is_empty()
                && self.tool_id.len() <= 256
                && !self.tool_id.chars().any(char::is_control),
            "来源工具标识无效"
        );
        ensure!(
            self.model.len() <= 256 && !self.model.chars().any(char::is_control),
            "模型名称无效"
        );
        Ok(())
    }
}

impl Revision {
    pub fn new(name: &str, tags: Vec<String>, body: &str, options: RunOptions) -> Result<Self> {
        let revision = Self {
            saved_at: chrono::Utc::now().to_rfc3339(),
            name: name.trim().into(),
            tags,
            body: body.into(),
            options,
        };
        revision.validate()?;
        Ok(revision)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name == self.name.trim()
                && self.name.chars().count() <= 80
                && !self.name.chars().any(char::is_control),
            "模板名称无效（最多 80 字）"
        );
        ensure!(
            chrono::DateTime::parse_from_rfc3339(&self.saved_at).is_ok(),
            "修订保存时间无效"
        );
        let tags = parse_tags(&self.tags.join(","))?;
        ensure!(tags == self.tags, "模板标签无效");
        variables(&self.body)?;
        self.options.validate()
    }
}

pub fn matches_query(revision: &Revision, query: &str) -> bool {
    let haystack = format!("{} {}", revision.name, revision.tags.join(" ")).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

pub fn diff(previous: &Revision, current: &Revision) -> String {
    let mut output = format!(
        "名称：{} → {}\n标签：{} → {}\n模型：{} → {}\n流式：{} → {}；多轮：{} → {}\n\n",
        previous.name,
        current.name,
        previous.tags.join(", "),
        current.tags.join(", "),
        previous.options.model,
        current.options.model,
        previous.options.stream,
        current.options.stream,
        previous.options.multi_turn,
        current.options.multi_turn
    );
    for change in TextDiff::from_lines(&previous.body, &current.body).iter_all_changes() {
        let mark = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        output.push(mark);
        output.push_str(change.to_string().as_str());
        if output.len() > 64 * 1024 {
            let mut cut = 64 * 1024;
            while !output.is_char_boundary(cut) {
                cut -= 1;
            }
            output.truncate(cut);
            output.push_str("\n…差异已截断");
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_escape_braces_and_render_only_after_complete_values() {
        let body = "请检查 {{project}}：{{{{literal}}}} / {{project}} 🙂";
        assert_eq!(variables(body).unwrap(), ["project"]);
        assert!(render(body, &BTreeMap::new()).is_err());
        let values = BTreeMap::from([("project".into(), "Zi DevTools".into())]);
        assert_eq!(
            render(body, &values).unwrap(),
            "请检查 Zi DevTools：{{literal}} / Zi DevTools 🙂"
        );
        for malformed in [
            "{{bad-name}}",
            "{{9name}}",
            "{{unfinished",
            "stray }}",
            "{{}}",
        ] {
            assert!(variables(malformed).is_err(), "{malformed}");
        }
        let huge = BTreeMap::from([("project".into(), "x".repeat(MAX_RENDER_BYTES))]);
        assert!(render(body, &huge).is_err());
    }

    #[test]
    fn tags_search_and_revision_diff_report_changed_text() {
        let tags = parse_tags("Rust, 文档，Windows").unwrap();
        assert_eq!(tags, ["Rust", "文档", "Windows"]);
        assert!(parse_tags("Rust,rust").is_err());
        let options = RunOptions {
            tool_id: "plugin:example/chat".into(),
            model: "local".into(),
            stream: true,
            multi_turn: false,
        };
        let before = Revision::new("检查", tags, "旧正文\n", options.clone()).unwrap();
        let after = Revision::new("检查", vec!["Rust".into()], "新正文\n", options).unwrap();
        assert!(matches_query(&before, "rust 文档"));
        assert!(!matches_query(&before, "python"));
        let text = diff(&before, &after);
        assert!(text.contains("-旧正文") && text.contains("+新正文"));
    }
}
