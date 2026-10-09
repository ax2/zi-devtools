use regex_syntax::hir::{Class, HirKind};
use std::sync::LazyLock;

pub fn digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    static RANGES: LazyLock<Vec<(char, char)>> = LazyLock::new(|| {
        let hir = regex_syntax::Parser::new()
            .parse(r"\d")
            .expect("Unicode decimal class");
        let HirKind::Class(Class::Unicode(class)) = hir.kind() else {
            unreachable!()
        };
        class.iter().map(|r| (r.start(), r.end())).collect()
    });
    let at = RANGES.partition_point(|(start, _)| *start <= c);
    at > 0 && c <= RANGES[at - 1].1
}
pub fn decimal(s: &str) -> Option<(&str, &str)> {
    let mut end = s.find(|c| !digit(c)).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    if let Some(rest) = s[end..].strip_prefix('.') {
        let fraction = rest.find(|c| !digit(c)).unwrap_or(rest.len());
        if fraction > 0 {
            end += 1 + fraction;
        }
    }
    Some((&s[..end], &s[end..]))
}

pub fn elapsed(tail: &str) -> Option<f64> {
    for (at, _) in tail.match_indices("in ") {
        if let Some((number, rest)) = decimal(&tail[at + 3..])
            && rest.starts_with('s')
        {
            return number.parse().ok();
        }
    }
    None
}
