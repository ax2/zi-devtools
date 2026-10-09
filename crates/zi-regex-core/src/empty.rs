//! Exact fast path for concatenations of empty captures; all other HIR falls back.
use regex_syntax::hir::{Hir, HirKind};

pub(crate) fn groups(pattern: &str) -> Option<Vec<u32>> {
    // A flat sequence of literal empty groups has a complete, unambiguous
    // grammar and no nesting, flags, escapes or optional groups. Recognize it
    // directly: the general parser's capture bookkeeping is costly at scale.
    if !pattern.is_empty()
        && pattern.len().is_multiple_of(2)
        && pattern.as_bytes().chunks_exact(2).all(|pair| pair == b"()")
    {
        return Some((1..=u32::try_from(pattern.len() / 2).ok()?).collect());
    }
    // Avoid reparsing ordinary patterns and expensive Unicode classes.
    if !pattern.contains("()") {
        return None;
    }
    let hir = regex_syntax::Parser::new().parse(pattern).ok()?;
    fn collect(hir: &Hir, groups: &mut Vec<u32>) -> bool {
        match hir.kind() {
            HirKind::Empty => true,
            HirKind::Capture(capture) => {
                groups.push(capture.index);
                collect(&capture.sub, groups)
            }
            HirKind::Concat(parts) => parts.iter().all(|part| collect(part, groups)),
            _ => false,
        }
    }
    let mut groups = Vec::new();
    if !collect(&hir, &mut groups) {
        return None;
    }
    groups.sort_unstable();
    Some(groups)
}
