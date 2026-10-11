//! Equivalent literal search for a bounded, fully parsed exact repetition.
//! Everything outside this shape continues through the original regex engine.
use anyhow::Result;
use regex_syntax::hir::HirKind;

use super::{ReportTooLarge, clipped};

#[cfg(test)]
use crate::tests::legacy;

// With the pinned default compiler, an exact non-empty Literal repetition
// creates one Range per expanded byte, plus the small unanchored prefix,
// capture 0 and Match wrappers. 256KiB of bytes accounts for at most 8MiB
// of Windows64 intermediate Range states (32 bytes each), leaving 2MiB for
// those fixed wrappers under the original 10MiB limit. There are no classes,
// branches, assertions or explicit capture states inside this accepted HIR.
// This is an acceleration bound, not a new input or regex acceptance limit.
const MAX_LITERAL_BYTES: usize = 256 * 1024;

struct LiteralRepeat {
    unit: String,
    count: usize,
    bytes: usize,
}

fn literal(pattern: &str) -> Option<LiteralRepeat> {
    // Avoid a second parse for ordinary expressions without counted repeats.
    if !pattern.contains('{') {
        return None;
    }
    let hir = regex_syntax::Parser::new().parse(pattern).ok()?;
    if hir.properties().explicit_captures_len() != 0 {
        return None;
    }
    let HirKind::Repetition(repetition) = hir.kind() else {
        return None;
    };
    if repetition.min < 2 || repetition.max != Some(repetition.min) {
        return None;
    }
    let HirKind::Literal(bytes) = repetition.sub.kind() else {
        return None;
    };
    let unit = std::str::from_utf8(&bytes.0).ok()?;
    let count = usize::try_from(repetition.min).ok()?;
    let length = unit.len().checked_mul(count)?;
    if length == 0 || length > MAX_LITERAL_BYTES {
        return None;
    }
    Some(LiteralRepeat {
        unit: unit.to_owned(),
        count,
        bytes: length,
    })
}

pub(crate) fn evaluate(pattern: &str, input: &str, budget: usize) -> Option<Result<String>> {
    let repetition = literal(pattern)?;
    // The HIR and the conservative program-size bound have already been
    // validated. A shorter haystack cannot contain this non-empty literal;
    // avoid materializing or preprocessing a huge search needle in that case.
    if input.len() < repetition.bytes {
        let output = "没有匹配";
        return Some(if output.len() > budget {
            Err(ReportTooLarge.into())
        } else {
            Ok(output.to_owned())
        });
    }
    let needle = repetition.unit.repeat(repetition.count);
    let mut output = String::new();
    for (index, (offset, matched)) in input.match_indices(needle.as_str()).take(101).enumerate() {
        if index == 100 {
            output.push_str("\n... 仅展示前 100 个匹配 ...");
            break;
        }
        output.push_str(&format!(
            "#{} 字节 {}..{}: {}\n",
            index + 1,
            offset,
            offset + matched.len(),
            clipped(matched)
        ));
        if output.len() > budget {
            return Some(Err(ReportTooLarge.into()));
        }
    }
    if output.is_empty() {
        output.push_str("没有匹配");
    }
    Some(if output.len() > budget {
        Err(ReportTooLarge.into())
    } else {
        Ok(output)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bounded_exact_literal_hir_is_accelerated() {
        for pattern in ["a{2}", "(?:中日){3}", r"\x61{2}", "a{2,2}?", "a{262144}"] {
            assert!(literal(pattern).is_some(), "{pattern}");
        }
        for pattern in [
            "a{0}",
            "a{1}",
            "a{2,3}",
            "a{2,}",
            "(a){2}",
            "[ab]{2}",
            "(?i:a){2}",
            "^a{2}$",
            "a{2}|b",
            "a{262145}",
            "中{87382}",
            "a{1000000}",
            "a{4294967295}",
            "a{4294967296}",
            "a{2}(",
        ] {
            assert!(literal(pattern).is_none(), "{pattern}");
        }
    }

    #[test]
    fn matches_original_offsets_clipping_overlap_and_display_count() {
        for unit in ["a", "中", "🙂a", "aba", "\n", "\\", ".", "\0"] {
            for count in [2, 3, 120, 121] {
                let pattern = format!("(?:{}){{{count}}}", regex::escape(unit));
                let needle = unit.repeat(count);
                for text in [
                    String::new(),
                    needle.clone(),
                    format!("甲{needle}乙{needle}"),
                    unit.repeat(count - 1),
                    unit.repeat(count * 3 + 1),
                ] {
                    let actual = evaluate(&pattern, &text, usize::MAX)
                        .expect("eligible HIR")
                        .unwrap();
                    assert_eq!(
                        actual,
                        legacy::test_regex(&pattern, &text).unwrap(),
                        "{pattern:?}"
                    );
                    assert_eq!(
                        evaluate(&pattern, &text, actual.len()).unwrap().unwrap(),
                        actual
                    );
                    assert!(
                        evaluate(&pattern, &text, actual.len() - 1)
                            .unwrap()
                            .unwrap_err()
                            .is::<ReportTooLarge>()
                    );
                }
            }
        }
        for count in [99, 100, 101, 200] {
            let text = "aa".repeat(count);
            assert_eq!(
                evaluate("a{2}", &text, usize::MAX).unwrap().unwrap(),
                legacy::test_regex("a{2}", &text).unwrap()
            );
        }
    }

    #[test]
    fn acceleration_bound_remains_below_original_compiler_limit() {
        // Check the conservative upper bound against the actual pinned native
        // compiler, including multi-byte units. Empty text isolates compiler
        // acceptance; the original engine's huge matching cost is a separate
        // issue and must not make this size-bound regression unbounded.
        for (pattern, text) in [
            ("a{262144}", String::new()),
            ("a{200000}", String::new()),
            ("中{87381}", String::new()),
            ("(?:🙂a){52428}", String::new()),
        ] {
            assert_eq!(
                evaluate(pattern, &text, usize::MAX).unwrap().unwrap(),
                legacy::test_regex(pattern, &text).unwrap(),
                "{pattern}"
            );
        }
        for pattern in ["a{262145}", "a{327680}", "a{400000}", "中{87382}"] {
            assert!(evaluate(pattern, "", usize::MAX).is_none(), "{pattern}");
        }
    }

    #[test]
    fn large_match_has_independently_known_report() {
        for (pattern, unit, count) in [("a{200000}", "a", 200000), ("中{87381}", "中", 87381)] {
            let text = unit.repeat(count);
            let expected = format!("#1 字节 0..{}: {}…\n", text.len(), unit.repeat(120));
            assert_eq!(
                evaluate(pattern, &text, usize::MAX).unwrap().unwrap(),
                expected
            );
            assert_eq!(
                evaluate(pattern, &text[..text.len() - unit.len()], usize::MAX)
                    .unwrap()
                    .unwrap(),
                "没有匹配"
            );
        }
    }
}
