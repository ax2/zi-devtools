//! Fixed grammar scanners, preserving the original regular-expression captures.
include!(concat!(env!("OUT_DIR"), "/trace_classes.rs"));

fn in_class(c: char, ranges: &[(char, char)]) -> bool {
    let at = ranges.partition_point(|(start, _)| *start <= c);
    at > 0 && c <= ranges[at - 1].1
}
fn word(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '_'
    } else {
        in_class(c, WORD)
    }
}
fn digit(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_digit()
    } else {
        in_class(c, DECIMAL)
    }
}
fn initial(c: char, dollar: bool) -> bool {
    c.is_ascii_alphabetic() || c == '_' || (dollar && c == '$')
}
#[derive(Debug)]
pub struct Captures<'a>([Option<&'a str>; 4]);
pub struct Match<'a>(&'a str);
impl<'a> Match<'a> {
    pub fn as_str(&self) -> &'a str {
        self.0
    }
}
impl<'a> Captures<'a> {
    pub fn get(&self, index: usize) -> Option<Match<'a>> {
        self.0.get(index).copied().flatten().map(Match)
    }
}
impl std::ops::Index<usize> for Captures<'_> {
    type Output = str;
    fn index(&self, index: usize) -> &str {
        self.0[index].unwrap()
    }
}
fn captures<'a>(a: &'a str, b: Option<&'a str>, c: Option<&'a str>) -> Captures<'a> {
    Captures([None, Some(a), b, c])
}
fn class_message(text: &str, java: bool) -> Option<Captures<'_>> {
    let (class, message) = match text.split_once(':') {
        Some((name, message)) => (
            name,
            Some(message.strip_prefix(char::is_whitespace).unwrap_or(message)),
        ),
        None => (text, None),
    };
    if class.is_empty() || class.contains('\n') || message.is_some_and(|s| s.contains('\n')) {
        return None;
    }
    if java {
        for segment in class.split(['.', '/']) {
            let mut chars = segment.chars();
            if !chars.next().is_some_and(|c| initial(c, true))
                || !chars.all(|c| word(c) || c == '$')
            {
                return None;
            }
        }
    } else {
        let mut chars = class.chars();
        if !chars.next().is_some_and(|c| initial(c, false)) || !chars.all(|c| word(c) || c == '.') {
            return None;
        }
    }
    Some(captures(class, message, None))
}
pub enum Pattern {
    JavaHeader,
    JavaFrame,
    JavaOmitted,
    PythonFrame,
    PythonError,
}
impl Pattern {
    pub fn captures<'a>(&self, input: &'a str) -> Option<Captures<'a>> {
        match self {
            Self::JavaHeader => {
                let mut text = input;
                if let Some(tail) = text.strip_prefix("Exception in thread \"") {
                    let (thread, rest) = tail.split_once('"')?;
                    if thread.is_empty() {
                        return None;
                    }
                    text = rest.strip_prefix(' ')?;
                }
                class_message(text, true)
            }
            Self::PythonError => class_message(input, false),
            Self::JavaOmitted => {
                let tail = input.strip_prefix("... ")?;
                let count = tail.find(|c| !digit(c)).unwrap_or(tail.len());
                if count == 0 || !matches!(&tail[count..], " more" | " common frames omitted") {
                    return None;
                }
                Some(captures(&tail[..count], None, None))
            }
            Self::JavaFrame => {
                let tail = input.strip_prefix("at")?;
                let first = tail.chars().next()?;
                if !first.is_whitespace() {
                    return None;
                }
                let whitespace = tail.len() - tail.trim_start_matches(char::is_whitespace).len();
                let body = tail.strip_suffix(')')?;
                let open = body.rfind('(')?;
                let location = &body[open + 1..];
                if location.contains(['(', ')']) || open < first.len_utf8() {
                    return None;
                }
                // The greedy whitespace quantifier backtracks one character if
                // necessary to leave the required nonempty call capture.
                let start = if whitespace < open {
                    whitespace
                } else {
                    body[..open].char_indices().next_back()?.0
                };
                if start < first.len_utf8() {
                    return None;
                }
                let call = &body[start..open];
                if call.is_empty() || call.contains('\n') {
                    return None;
                }
                Some(captures(call, Some(location), None))
            }
            Self::PythonFrame => {
                let tail = input
                    .trim_start_matches(char::is_whitespace)
                    .strip_prefix("File \"")?;
                if tail.contains('\n') {
                    return None;
                }
                // The original path group is greedy, including quotes. Search
                // separators backwards and retain the last valid suffix.
                for (at, _) in tail.rmatch_indices("\", line ") {
                    let path = &tail[..at];
                    if path.is_empty() {
                        continue;
                    }
                    let suffix = &tail[at + "\", line ".len()..];
                    let end = suffix.find(|c| !digit(c)).unwrap_or(suffix.len());
                    if end == 0 {
                        continue;
                    }
                    let function = if end == suffix.len() {
                        None
                    } else {
                        let Some(function) = suffix[end..].strip_prefix(", in ") else {
                            continue;
                        };
                        if function.is_empty() || function.contains('\n') {
                            continue;
                        }
                        Some(function)
                    };
                    return Some(captures(path, Some(&suffix[..end]), function));
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scanner_captures_match_original_regex_grammar() {
        use Pattern::*;
        let patterns = [
            (
                JavaHeader,
                r#"^(?:Exception in thread "[^"]+" )?([A-Za-z_$][\w$]*(?:[./][A-Za-z_$][\w$]*)*)(?::\s?(.*))?$"#,
            ),
            (JavaFrame, r"^at\s+(.+)\(([^()]*)\)$"),
            (
                JavaOmitted,
                r"^\.\.\. (\d+) (?:more|common frames omitted)$",
            ),
            (PythonFrame, r#"^\s*File "(.+)", line (\d+)(?:, in (.+))?$"#),
            (
                PythonError,
                r"^([A-Za-z_][\w.]*(?:Error|Exception|Warning|Exit|Interrupt|Match|Host|Exist|History|Configured)?)(?::\s?(.*))?$",
            ),
        ];
        let fixed = [
            "",
            "Error",
            "a.中Error",
            "a.A中Error",
            "a.A\u{301}Error:  中文",
            "$X/Mod$Error: x",
            "a..Error",
            "Exception in thread \"main\" Error: x",
            "Exception in thread \"\" Error",
            "... ١ more",
            "... 2 common frames omitted",
            "at x(file)",
            "at  (file)",
            "at (file)",
            "at x(a(b)",
            "at x(a)b)",
            "File \"p\", line ١, in f",
            "File \"a\", line 1\", line 2, in x",
            "File \"\", line 2",
            "File \"a\", line 2, in ",
            "File \"a\", line 2",
            "ValueError: \tmessage",
            "Error:\nmessage",
        ];
        for (scanner, pattern) in patterns {
            let regex = regex::Regex::new(pattern).unwrap();
            for text in fixed.iter().copied().map(str::to_owned).chain(
                [
                    ' ', '\t', '\r', '\u{85}', '\u{a0}', '\u{2003}', '\u{2028}', '\u{2029}',
                ]
                .into_iter()
                .flat_map(|space| {
                    [
                        format!("at{space}x(file)"),
                        format!("at{space}{space}(file)"),
                        format!("{space}File \"项目.py\", line 7, in f"),
                        format!("Error:{space}message"),
                    ]
                }),
            ) {
                let old = regex.captures(&text);
                let new = scanner.captures(&text);
                assert_eq!(old.is_some(), new.is_some(), "{pattern}: {text:?}");
                if let (Some(old), Some(new)) = (old, new) {
                    for index in 1..old.len() {
                        assert_eq!(
                            old.get(index).map(|m| m.as_str()),
                            new.get(index).map(|m| m.as_str()),
                            "{pattern}: {text:?}, group {index}"
                        );
                    }
                }
            }
        }
    }
}
