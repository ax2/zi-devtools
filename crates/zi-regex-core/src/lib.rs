//! Complete standalone regex matching with no Host IO.
use anyhow::{Context, Result, anyhow};

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
