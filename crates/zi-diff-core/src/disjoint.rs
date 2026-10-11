//! Exact fast path for large LF-terminated texts with no equal lines.
//! Hashes only select candidates: full line equality decides intersection.
use std::fmt::{self, Write};

const MIN_LINES: usize = 512;
const MAX_COLLISION_COMPARISONS: usize = 64;
const BUCKETS: usize = 4096;
const MAX_BUCKET_PROBES: usize = 8192;

pub(super) struct Plan {
    before_lines: usize,
    after_lines: usize,
}

fn hash(line: &str) -> u64 {
    line.bytes().fold(0xcbf29ce484222325, |value, byte| {
        (value ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

pub(super) fn plan(before: &str, after: &str) -> Option<Plan> {
    plan_with_hash(before, after, hash)
}

fn plan_with_hash(before: &str, after: &str, hash: impl Fn(&str) -> u64) -> Option<Plan> {
    if !before.ends_with('\n')
        || !after.ends_with('\n')
        || before.contains('\r')
        || after.contains('\r')
    {
        return None;
    }
    let before_lines = before.split_inclusive('\n').count();
    let after_lines = after.split_inclusive('\n').count();
    if before_lines < MIN_LINES || after_lines < MIN_LINES {
        return None;
    }
    let result = Plan {
        before_lines,
        after_lines,
    };
    let mut heads = [usize::MAX; BUCKETS];
    let mut entries: Vec<(u64, &str, usize)> = Vec::with_capacity(before_lines);
    for line in before.split_inclusive('\n') {
        let digest = hash(line);
        let bucket = (digest as usize) & (BUCKETS - 1);
        entries.push((digest, line, heads[bucket]));
        heads[bucket] = entries.len() - 1;
    }
    let (mut probes, mut comparisons) = (0, 0);
    for line in after.split_inclusive('\n') {
        let digest = hash(line);
        let mut index = heads[(digest as usize) & (BUCKETS - 1)];
        while index != usize::MAX {
            if probes == MAX_BUCKET_PROBES {
                return None;
            }
            probes += 1;
            let entry = &entries[index];
            if entry.0 == digest {
                if comparisons == MAX_COLLISION_COMPARISONS {
                    return None;
                }
                comparisons += 1;
                if entry.1 == line {
                    return None;
                }
            }
            index = entry.2;
        }
    }
    Some(result)
}

pub(super) fn write_report(
    before: &str,
    after: &str,
    plan: Plan,
    output: &mut impl Write,
) -> fmt::Result {
    write!(
        output,
        "--- 原文\n+++ 修改后\n@@ -1,{} +1,{} @@\n",
        plan.before_lines, plan.after_lines
    )?;
    for (prefix, text) in [("-", before), ("+", after)] {
        for line in text.split_inclusive('\n') {
            output.write_str(prefix)?;
            output.write_str(line)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompareError, compare_text, tests::legacy};

    #[test]
    fn exact_original_reports_and_budgets() {
        for count in [511, 512, 513, 1400, 1450, 5000] {
            let before: String = (0..count).map(|i| format!("old {i}中🙂\n")).collect();
            let after: String = (0..count).map(|i| format!("new {i}中🙂\n")).collect();
            assert_eq!(plan(&before, &after).is_some(), count >= MIN_LINES);
            let expected = legacy(&before, &after).unwrap();
            assert_eq!(compare_text(&before, &after, None).unwrap(), expected);
            assert_eq!(
                compare_text(&before, &after, Some(expected.len())).unwrap(),
                expected
            );
            assert_eq!(
                compare_text(&before, &after, Some(expected.len() - 1)),
                Err(CompareError::ReportTooLarge)
            );
        }
    }

    #[test]
    fn shared_lines_endings_and_hash_collisions_fall_back() {
        let before = "a\n".repeat(512);
        let after = "b\n".repeat(512);
        assert!(plan(&before, &after).is_some());
        assert!(plan_with_hash(&before, &after, |_| 0).is_none());
        // Bucket collisions without full-hash collisions hit the probe guard.
        assert!(
            plan_with_hash(&before, &after, |line| {
                if line == "a\n" { 4096 } else { 8192 }
            })
            .is_none()
        );
        for altered in [
            format!("a\n{}", "b\n".repeat(511)),
            "b\r\n".repeat(512),
            format!("{}b", "b\n".repeat(511)),
        ] {
            assert!(plan(&before, &altered).is_none());
            assert_eq!(
                compare_text(&before, &altered, None),
                legacy(&before, &altered)
            );
        }
        // Distinct single lines with equal hashes must not be mistaken for matches.
        let left = "old\n".repeat(512);
        let right = "new\n".repeat(512);
        assert!(plan_with_hash(&left, &right, |_| 7).is_none());
    }
}
