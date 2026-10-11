//! Exact bounded line indexing before the original Myers algorithm.
use std::{
    fmt,
    fmt::Write,
    time::{Duration, Instant},
};

use similar::{Algorithm, capture_diff_deadline, group_diff_ops, udiff::UnifiedHunkHeader};

use crate::{ComparePhase, disjoint::has_bare_cr};

const MAX_LINE_BYTES: usize = 64;
const BUCKETS: usize = 4096;
const MAX_PROBES: usize = 16384;
const MAX_COLLISIONS: usize = 64;

pub(super) fn try_write(
    before: &str,
    after: &str,
    output: &mut impl Write,
    observe: &mut impl FnMut(ComparePhase),
) -> Option<fmt::Result> {
    if has_bare_cr(before) || has_bare_cr(after) {
        return None;
    }
    let old: Vec<_> = before.split_inclusive('\n').collect();
    let new: Vec<_> = after.split_inclusive('\n').collect();
    if old.len() + new.len() < 128
        || old
            .iter()
            .chain(&new)
            .any(|line| line.len() > MAX_LINE_BYTES)
    {
        return None;
    }
    let deadline = Some(Instant::now() + Duration::from_millis(500));
    let (old_ids, new_ids) = index_lines(&old, &new, crate::disjoint::hash)?;
    observe(ComparePhase::DirectPrepared);
    // IDs follow IdentifyDistinct: first old occurrences, then new ones.
    // Equality and the Myers/Compact/Replace pipeline remain unchanged.
    let ops = capture_diff_deadline(
        Algorithm::Myers,
        &old_ids,
        0..old_ids.len(),
        &new_ids,
        0..new_ids.len(),
        deadline,
    );
    observe(ComparePhase::DiffComputed);
    let result = write_ops(ops, &old, &new, output);
    if result.is_ok() {
        observe(ComparePhase::ReportFormatted);
    }
    Some(result)
}

fn index_lines<'a>(
    old: &[&'a str],
    new: &[&'a str],
    hash: impl Fn(&str) -> u64,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut heads = [usize::MAX; BUCKETS];
    let mut entries: Vec<(u64, &'a str, usize)> = Vec::new();
    let (mut old_ids, mut new_ids) = (Vec::with_capacity(old.len()), Vec::with_capacity(new.len()));
    let (mut probes, mut collisions) = (0, 0);
    for (ids, lines) in [(&mut old_ids, old), (&mut new_ids, new)] {
        for &line in lines {
            let digest = hash(line);
            let bucket = (digest as usize) & (BUCKETS - 1);
            let mut index = heads[bucket];
            let mut found = None;
            while index != usize::MAX {
                if probes == MAX_PROBES {
                    return None;
                }
                probes += 1;
                let entry = &entries[index];
                if entry.0 == digest {
                    if entry.1 == line {
                        found = Some(index as u32);
                        break;
                    }
                    if collisions == MAX_COLLISIONS {
                        return None;
                    }
                    collisions += 1;
                }
                index = entry.2;
            }
            let id = match found {
                Some(id) => id,
                None => {
                    let id = u32::try_from(entries.len()).ok()?;
                    entries.push((digest, line, heads[bucket]));
                    heads[bucket] = entries.len() - 1;
                    id
                }
            };
            ids.push(id);
        }
    }
    Some((old_ids, new_ids))
}

fn write_ops(
    ops: Vec<similar::DiffOp>,
    old: &[&str],
    new: &[&str],
    output: &mut impl Write,
) -> fmt::Result {
    for (index, group) in group_diff_ops(ops, 3).iter().enumerate() {
        if index == 0 {
            output.write_str("--- 原文\n+++ 修改后\n")?;
        }
        writeln!(output, "{}", UnifiedHunkHeader::new(group))?;
        for op in group {
            for change in op.iter_changes(old, new) {
                write!(output, "{}{}", change.tag(), change.value())?;
                if !change.value().ends_with(['\r', '\n']) {
                    output.write_str("\n\\ No newline at end of file\n")?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{CompareError, compare_text, tests::legacy};

    #[test]
    fn deterministic_mixed_line_reports_match_original_algorithm() {
        let alphabet = [
            "a\n",
            "b\n",
            "\n",
            "中🙂\n",
            "x\r\n",
            "x\0\n",
            "\t\n",
            "abcdefgh\n",
        ];
        for seed in 0..512u64 {
            let mut state = seed + 1;
            let mut build = || {
                let mut value = String::new();
                for _ in 0..128 + seed as usize % 64 {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    value.push_str(alphabet[(state >> 32) as usize % alphabet.len()]);
                }
                value
            };
            let (old, new) = (build(), build());
            let expected = legacy(&old, &new).unwrap();
            let mut direct = String::new();
            super::try_write(&old, &new, &mut direct, &mut |_| {})
                .unwrap()
                .unwrap();
            assert_eq!(direct, expected, "seed {seed}");
            assert_eq!(compare_text(&old, &new, None).unwrap(), expected);
        }
    }

    #[test]
    fn exact_ids_match_upstream_and_hash_collisions_are_bounded() {
        let old = ["a\n", "b\n", "a\n", "c\n"];
        let new = ["b\n", "d\n", "a\n"];
        let upstream = similar::algorithms::IdentifyDistinct::<u32>::new(
            &old,
            0..old.len(),
            &new,
            0..new.len(),
        );
        for hash in [crate::disjoint::hash as fn(&str) -> u64, |_| 0] {
            let (a, b) = super::index_lines(&old, &new, hash).unwrap();
            assert_eq!(
                a,
                (0..old.len())
                    .map(|i| upstream.old_lookup()[i])
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                b,
                (0..new.len())
                    .map(|i| upstream.new_lookup()[i])
                    .collect::<Vec<_>>()
            );
        }
        let values: Vec<_> = (0..100).map(|i| format!("{i}\n")).collect();
        let refs: Vec<_> = values.iter().map(String::as_str).collect();
        assert!(super::index_lines(&refs, &[], |_| 0).is_none());
        assert!(super::index_lines(&vec!["a"; super::MAX_PROBES + 2], &[], |_| 0).is_none());
    }

    #[test]
    fn repeated_lines_keep_original_hunks_and_eof_markers() {
        for count in [128, 512, 2000] {
            for (old, new) in [
                ("a\nb\n".repeat(count), "b\na\n".repeat(count)),
                (
                    format!("{}a", "x\n".repeat(count)),
                    format!("{}b\n", "x\n".repeat(count)),
                ),
                (
                    "x\r\n".repeat(count),
                    format!("y\r\n{}z\r\n", "x\r\n".repeat(count)),
                ),
                ("x\n".repeat(count), String::new()),
                ("x\n".repeat(count), "x\ny\nx\nz\n".repeat(count / 4)),
            ] {
                let expected = legacy(&old, &new).unwrap();
                assert_eq!(compare_text(&old, &new, None).unwrap(), expected);
                assert_eq!(
                    compare_text(&old, &new, Some(expected.len())).unwrap(),
                    expected
                );
                assert_eq!(
                    compare_text(&old, &new, Some(expected.len() - 1)),
                    Err(CompareError::ReportTooLarge)
                );
            }
        }
    }
}
