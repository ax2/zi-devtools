//! Name rules shared by the standalone rename workflow and read-only preview.

use anyhow::{Result, ensure};

const MAX_RULE_BYTES: usize = 4096;
const MAX_NAME_UTF16: usize = 255;
const MAX_NAME_BYTES: usize = MAX_NAME_UTF16 * 3;
const TOO_LONG: &str = "目标文件名超过 255 个 UTF-16 单元";

pub struct Rules<'a> {
    pub find: &'a str,
    pub replacement: &'a str,
    pub prefix: &'a str,
    pub suffix: &'a str,
    pub case_sensitive: bool,
}

impl Rules<'_> {
    pub fn validate(&self) -> Result<()> {
        for value in [self.find, self.replacement, self.prefix, self.suffix] {
            ensure!(
                value.len() <= MAX_RULE_BYTES,
                "重命名规则每项不能超过 4 KiB"
            );
        }
        Ok(())
    }

    pub fn target_name(&self, stem: &str, extension: Option<&str>) -> Result<String> {
        let replaced = replace_literal(stem, self.find, self.replacement, self.case_sensitive)?;
        let mut target = format!("{}{replaced}{}", self.prefix, self.suffix);
        if let Some(extension) = extension {
            target.push('.');
            target.push_str(extension);
        }
        validate_component(&target)?;
        Ok(target)
    }
}

fn replace_literal(
    input: &str,
    find: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Result<String> {
    if find.is_empty() {
        return Ok(input.to_owned());
    }
    if case_sensitive {
        let matches = input.matches(find).count();
        let size = input.len() - matches * find.len() + matches * replacement.len();
        ensure!(size <= MAX_NAME_BYTES, "{TOO_LONG}");
        return Ok(input.replace(find, replacement));
    }

    let mut folded = String::with_capacity(input.len());
    let ascii = input.is_ascii();
    let mut boundaries = Vec::new();
    if ascii {
        folded = input.to_ascii_lowercase();
    } else {
        boundaries.push(Some(0));
        for (start, character) in input.char_indices() {
            folded.extend(character.to_lowercase());
            boundaries.resize(folded.len(), None);
            boundaries.push(Some(start + character.len_utf8()));
        }
    }
    let source_boundary = |offset| {
        if ascii {
            Some(offset)
        } else {
            boundaries[offset]
        }
    };
    let needle = find.to_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut source_cursor = 0;
    let mut folded_cursor = 0;
    while let Some(relative) = folded[folded_cursor..].find(&needle) {
        let folded_start = folded_cursor + relative;
        let folded_end = folded_start + needle.len();
        folded_cursor = folded_end;
        let (Some(start), Some(end)) = (source_boundary(folded_start), source_boundary(folded_end))
        else {
            continue;
        };
        if start < source_cursor {
            continue;
        }
        let prefix = &input[source_cursor..start];
        ensure!(
            output.len() + prefix.len() + replacement.len() <= MAX_NAME_BYTES,
            "{TOO_LONG}"
        );
        output.push_str(prefix);
        output.push_str(replacement);
        source_cursor = end;
    }
    output.push_str(&input[source_cursor..]);
    Ok(output)
}

pub fn validate_component(name: &str) -> Result<()> {
    ensure!(!name.is_empty(), "目标文件名不能为空");
    ensure!(name.encode_utf16().count() <= MAX_NAME_UTF16, "{TOO_LONG}");
    ensure!(
        !name.ends_with(' ') && !name.ends_with('.'),
        "Windows 文件名不能以空格或句点结尾"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control()
                || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')),
        "目标文件名包含 Windows 不允许的字符"
    );
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    ensure!(!reserved, "目标文件名是 Windows 保留设备名");
    Ok(())
}
