//! File eligibility and exclusions shared with the native knowledge source scanner.

use anyhow::{Result, ensure};
use std::path::Path;

const DEFAULT_EXCLUDES: &[&str] = &[".git", "node_modules", "target", ".venv", "__pycache__"];

pub fn parse_excludes(text: &str) -> Result<Vec<String>> {
    let mut rules = Vec::new();
    for raw in text.split([',', ';', '\n']) {
        let rule = raw.trim().replace('\\', "/");
        if rule.is_empty() {
            continue;
        }
        ensure!(
            rule.len() <= 120 && rules.len() < 32,
            "排除规则最多 32 条、每条 120 字符"
        );
        ensure!(
            !rule.starts_with('/') && !rule.contains(':') && !rule.chars().any(char::is_control),
            "排除规则必须是相对名称"
        );
        ensure!(
            !rule.split('/').any(|part| part == ".." || part == "."),
            "排除规则不能包含路径穿越"
        );
        ensure!(
            !rule
                .split('/')
                .enumerate()
                .any(|(i, part)| part.is_empty() && i > 0 && i + 1 < rule.split('/').count()),
            "排除规则不能包含空目录段"
        );
        ensure!(
            !rule.contains('*')
                || (rule.starts_with("*.")
                    && rule.len() > 2
                    && rule[2..].chars().all(|c| c.is_alphanumeric())),
            "仅支持 *.扩展名 通配规则"
        );
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

pub struct Exclusions {
    segments: Vec<String>,
    paths: Vec<String>,
    extensions: Vec<String>,
}

impl Exclusions {
    pub fn new(rules: &[String]) -> Self {
        let mut result = Self {
            segments: Vec::new(),
            paths: Vec::new(),
            extensions: Vec::new(),
        };
        for rule in rules {
            let lower = rule.to_lowercase();
            if let Some(suffix) = lower.strip_prefix("*.") {
                result.extensions.push(format!(".{suffix}"));
            } else if lower.ends_with('/') {
                let name = lower.trim_end_matches('/');
                if name.contains('/') {
                    result.paths.push(name.to_owned());
                } else {
                    result.segments.push(name.to_owned());
                }
            } else {
                result.paths.push(lower);
            }
        }
        result
    }

    pub fn excludes(&self, relative: &str, directory: bool) -> bool {
        self.excludes_lowercase(&relative.to_lowercase(), directory)
    }

    /// Match a name that has already been lowercased for comparison.
    pub fn excludes_lowercase(&self, lower: &str, directory: bool) -> bool {
        lower.split('/').any(|segment| {
            DEFAULT_EXCLUDES.contains(&segment) || self.segments.iter().any(|name| name == segment)
        }) || self.paths.iter().any(|path| {
            lower == path
                || lower
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        }) || (!directory
            && self
                .extensions
                .iter()
                .any(|extension| lower.ends_with(extension)))
    }
}

#[derive(Clone, Copy)]
pub enum Format {
    Markdown,
    Text,
    Html,
    Pdf,
    Docx,
    Image,
}

impl Format {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Html => "html",
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Image => "image",
        }
    }
}

pub fn format(path: &Path) -> Option<Format> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "md" | "markdown" => Format::Markdown,
        "txt" => Format::Text,
        "html" | "htm" => Format::Html,
        "pdf" => Format::Pdf,
        "docx" => Format::Docx,
        "png" | "jpg" | "jpeg" | "webp" => Format::Image,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_rules_keep_existing_segment_root_and_extension_semantics() {
        let rules = parse_excludes("Cache/;foo/bar/;draft;*.pdf").unwrap();
        let excludes = Exclusions::new(&rules);
        assert!(excludes.excludes("project/.git/private.md", false));
        assert!(excludes.excludes("project/CACHE/readme.md", false));
        assert!(excludes.excludes("foo/BAR/chapter.md", false));
        assert!(excludes.excludes("draft/chapter.md", false));
        assert!(excludes.excludes("ref/appendix.PDF", false));
        assert!(!excludes.excludes("ref/appendix.PDF", true));
        assert!(!excludes.excludes("project/foo/bar/chapter.md", false));
        assert!(!excludes.excludes("project/draft/chapter.md", false));
        assert!(!excludes.excludes("foo/barrier/chapter.md", false));
    }
}
