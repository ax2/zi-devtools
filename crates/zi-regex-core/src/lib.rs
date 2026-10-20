//! Complete standalone regex matching with no Host IO.
use anyhow::{Context, Result, anyhow};
mod empty;
mod literal_repeat;

const REPLACE_INPUT_LIMIT: usize = 2 * 1024 * 1024;
const REPLACE_PATTERN_LIMIT: usize = 4096;
const REPLACE_TEMPLATE_LIMIT: usize = 64 * 1024;
const REPLACE_MATCH_LIMIT: usize = 1000;

#[derive(Debug, PartialEq, Eq)]
pub struct ReplacePreview {
    pub output: String,
    pub matches: usize,
}

/// Build an in-memory replacement preview. The input is never modified, and
/// the full result is refused if it exceeds the same 2 MiB boundary as input.
pub fn preview_replacements(
    pattern: &str,
    input: &str,
    replacement: &str,
) -> Result<ReplacePreview> {
    anyhow::ensure!(
        pattern.len() <= REPLACE_PATTERN_LIMIT,
        "正则表达式超过 4096 UTF-8 字节"
    );
    anyhow::ensure!(input.len() <= REPLACE_INPUT_LIMIT, "输入文本超过 2 MiB");
    anyhow::ensure!(
        replacement.len() <= REPLACE_TEMPLATE_LIMIT,
        "替换模板超过 64 KiB"
    );
    let expression = regex::Regex::new(pattern).context("正则表达式无效")?;
    let mut output = String::with_capacity(input.len().min(64 * 1024));
    let mut copied = 0;
    let mut matches = 0;
    for captures in expression.captures_iter(input) {
        anyhow::ensure!(
            matches < REPLACE_MATCH_LIMIT,
            "匹配数超过 1000 条，请缩小输入范围或收紧正则表达式"
        );
        let found = captures.get(0).expect("capture 0 always exists");
        append_replacement_part(&mut output, &input[copied..found.start()])?;
        append_expanded_replacement(&captures, replacement, &mut output)?;
        copied = found.end();
        matches += 1;
    }
    append_replacement_part(&mut output, &input[copied..])?;
    Ok(ReplacePreview { output, matches })
}

fn append_expanded_replacement(
    captures: &regex::Captures<'_>,
    mut template: &str,
    output: &mut String,
) -> Result<()> {
    while let Some(dollar) = template.find('$') {
        append_replacement_part(output, &template[..dollar])?;
        template = &template[dollar..];
        if template.starts_with("$$") {
            append_replacement_part(output, "$")?;
            template = &template[2..];
            continue;
        }
        let (reference, consumed) = if let Some(braced) = template.strip_prefix("${") {
            if let Some(end) = braced.find('}') {
                (Some(&braced[..end]), end + 3)
            } else {
                (None, 1)
            }
        } else {
            let name_len = template[1..]
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                .count();
            if name_len == 0 {
                (None, 1)
            } else {
                (Some(&template[1..1 + name_len]), 1 + name_len)
            }
        };
        let Some(reference) = reference else {
            append_replacement_part(output, "$")?;
            template = &template[consumed..];
            continue;
        };
        let matched = reference
            .parse::<usize>()
            .ok()
            .and_then(|index| captures.get(index))
            .or_else(|| captures.name(reference));
        if let Some(matched) = matched {
            append_replacement_part(output, matched.as_str())?;
        }
        template = &template[consumed..];
    }
    append_replacement_part(output, template)
}

fn append_replacement_part(output: &mut String, part: &str) -> Result<()> {
    anyhow::ensure!(
        output.len().saturating_add(part.len()) <= REPLACE_INPUT_LIMIT,
        "替换预览结果超过 2 MiB"
    );
    output.push_str(part);
    Ok(())
}

#[derive(Debug)]
pub struct ReportTooLarge;
impl std::fmt::Display for ReportTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Regex report exceeds result budget")
    }
}
impl std::error::Error for ReportTooLarge {}

pub fn test_regex(pattern: &str, input: &str) -> Result<String> {
    test_regex_with_result_budget(pattern, input, usize::MAX)
}

/// Report UTF-8 bytes are a lower bound; the caller must also check the escaped
/// serialized envelope. Full standalone output does not use a reduced budget.
pub fn test_regex_with_result_budget(
    pattern: &str,
    input: &str,
    result_budget: usize,
) -> Result<String> {
    if pattern.len() > 4_096 || input.len() > 2 * 1024 * 1024 {
        return Err(anyhow!("正则表达式或输入文本超过大小限制"));
    }
    if let Some(groups) = empty::groups(pattern) {
        let mut output = String::new();
        let offsets = input
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(input.len()));
        for (index, offset) in offsets.take(101).enumerate() {
            if index == 100 {
                output.push_str("\n... 仅展示前 100 个匹配 ...");
                break;
            }
            output.push_str(&format!("#{} 字节 {offset}..{offset}: \n", index + 1));
            if output.len() > result_budget {
                return Err(ReportTooLarge.into());
            }
            for group in &groups {
                output.push_str(&format!("  ${group}: \n"));
                if output.len() > result_budget {
                    return Err(ReportTooLarge.into());
                }
            }
        }
        if output.len() > result_budget {
            return Err(ReportTooLarge.into());
        }
        return Ok(output);
    }
    if let Some(result) = literal_repeat::evaluate(pattern, input, result_budget) {
        return result;
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
        if output.len() > result_budget {
            return Err(ReportTooLarge.into());
        }
        for group in 1..captures.len() {
            if let Some(value) = captures.get(group) {
                output.push_str(&format!("  ${group}: {}\n", clipped(value.as_str())));
                if output.len() > result_budget {
                    return Err(ReportTooLarge.into());
                }
            }
        }
    }
    if count == 0 {
        output = "没有匹配".to_owned();
    }
    if output.len() > result_budget {
        return Err(ReportTooLarge.into());
    }
    Ok(output)
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
mod tests;
