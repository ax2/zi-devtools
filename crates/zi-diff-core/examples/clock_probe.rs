//! Independent native/WASI evidence probe, not a plugin or Host contract.
use std::time::Instant;

use similar::{ChangeTag, TextDiff};
use zi_diff_core::{CompareError, compare_text};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("clock") => {
            let start = Instant::now();
            let mut end = start;
            for _ in 0..10_000 {
                end = Instant::now();
                assert!(end >= start);
                if end > start {
                    break;
                }
            }
            assert!(end > start, "monotonic clock did not advance");
            println!("monotonic-clock-advances");
        }
        Some("expired") => {
            let before: String = (0..1_000).map(|i| format!("old {i}\n")).collect();
            let after: String = (0..1_000).rev().map(|i| format!("new {i}\n")).collect();
            let diff = TextDiff::configure()
                .deadline(Instant::now())
                .diff_lines(&before, &after);
            let mut reconstructed_before = String::new();
            let mut reconstructed_after = String::new();
            for change in diff.iter_all_changes() {
                if change.tag() != ChangeTag::Insert {
                    reconstructed_before.push_str(change.value());
                }
                if change.tag() != ChangeTag::Delete {
                    reconstructed_after.push_str(change.value());
                }
            }
            assert_eq!(reconstructed_before, before);
            assert_eq!(reconstructed_after, after);
            // Do not require identical approximate edit scripts across machines.
            println!("expired-deadline-reconstructs-inputs");
        }
        Some("compare") => {
            // No direct Instant calls here: observed clock reads belong to the core.
            let inputs = ["", "a", "a\n", "a\r\n", "中🙂\n", "x\ny\nx\n"];
            for before in inputs {
                for after in inputs {
                    let report = compare_text(before, after, None).unwrap();
                    assert_eq!(
                        compare_text(before, after, Some(report.len())).unwrap(),
                        report
                    );
                    assert_eq!(
                        compare_text(before, after, Some(report.len() - 1)),
                        Err(CompareError::ReportTooLarge)
                    );
                    // ASCII hex makes byte comparison independent of console encoding.
                    for byte in report.bytes() {
                        print!("{byte:02x}");
                    }
                    println!();
                }
            }
        }
        Some("limits") => {
            // Exercise the complete native range in WASI, not just plugin-size text.
            for text in ["x".repeat(128 * 1024), "\n".repeat(5_000)] {
                assert_eq!(
                    compare_text(&text, &text, None).unwrap(),
                    "两份文本完全相同"
                );
            }
            for text in ["x".repeat(128 * 1024 + 1), "\n".repeat(5_001)] {
                for (before, after) in [(&text[..], &text[..]), (&text[..], ""), ("", &text[..])] {
                    assert_eq!(
                        compare_text(before, after, Some(0)),
                        Err(CompareError::InputTooLarge)
                    );
                }
            }
            let text = "x".repeat(128 * 1024);
            let report = compare_text(&text, "", None).unwrap();
            assert!(report.contains(&text));
            assert_eq!(
                compare_text(&text, "", Some(48 * 1024)),
                Err(CompareError::ReportTooLarge)
            );
            assert_eq!(compare_text(&text, "", Some(report.len())).unwrap(), report);
            println!("complete-input-limits-and-report-budget");
        }
        _ => panic!("expected clock, expired, compare or limits"),
    }
}
