use super::*;

#[test]
fn byte_line_count_preserves_rust_lines_semantics() {
    let alphabet = ["a", "\r", "\n", "中🙂"];
    for size in 0..=7 {
        for mut encoded in 0..4usize.pow(size) {
            let mut text = String::new();
            for _ in 0..size {
                text.push_str(alphabet[encoded % 4]);
                encoded /= 4;
            }
            assert_eq!(rust_line_count(&text), text.lines().count());
        }
    }
}

#[test]
fn escaped_report_budget_matches_actual_json_serializer() {
    let controls: String = (0..=127).map(char::from).collect();
    for (before, after) in [
        ("".to_owned(), controls.clone()),
        (controls, "中🙂\u{2028}\u{2029}\\\"/\n".to_owned()),
        ("same".to_owned(), "same".to_owned()),
    ] {
        let expected = legacy(&before, &after).unwrap();
        let escaped = serde_json::to_vec(&expected).unwrap().len() - 2;
        assert_eq!(
            compare_text_budgets_observed(&before, &after, None, Some(escaped), |_| {}).unwrap(),
            expected
        );
        assert_eq!(
            compare_text_budgets_observed(&before, &after, None, Some(escaped - 1), |_| {}),
            Err(CompareError::ReportTooLarge)
        );
    }
}

// The original desktop implementation is retained independently as the oracle.
pub(super) fn legacy(before: &str, after: &str) -> Result<String, CompareError> {
    if before.len() > 128 * 1024
        || after.len() > 128 * 1024
        || before.lines().count() > 5_000
        || after.lines().count() > 5_000
    {
        return Err(CompareError::InputTooLarge);
    }
    if before == after {
        return Ok("两份文本完全相同".to_owned());
    }
    Ok(TextDiff::configure()
        .timeout(Duration::from_millis(500))
        .diff_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header("原文", "修改后")
        .to_string())
}

#[test]
fn original_reports_and_exact_utf8_budgets() {
    let inputs = [
        "",
        "same",
        "\n",
        "a\n",
        "a",
        "a\r\n",
        "a\rb",
        "中🙂\n",
        "中🙂",
        "a\nb\na\n",
        "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n",
        "A\nb\nc\nd\ne\nf\ng\nh\ni\nj\nK\n",
    ];
    for before in inputs {
        for after in inputs {
            let expected = legacy(before, after).unwrap();
            assert_eq!(compare_text(before, after, None).unwrap(), expected);
            assert_eq!(
                compare_text(before, after, Some(expected.len())).unwrap(),
                expected
            );
            assert_eq!(
                compare_text(before, after, Some(expected.len() - 1)),
                Err(CompareError::ReportTooLarge)
            );
        }
    }
}

#[test]
fn original_input_limits_are_checked_before_equality() {
    for text in [
        "x".repeat(MAX_INPUT_BYTES),
        "中".repeat(MAX_INPUT_BYTES / 3),
    ] {
        assert_eq!(compare_text(&text, &text, None).unwrap(), IDENTICAL);
        assert_eq!(compare_text(&text, "", None), legacy(&text, ""));
    }
    for text in [
        "x".repeat(MAX_INPUT_BYTES + 1),
        "中".repeat(MAX_INPUT_BYTES / 3 + 1),
        "\n".repeat(5_001),
    ] {
        assert_eq!(
            compare_text(&text, &text, None),
            Err(CompareError::InputTooLarge)
        );
        assert_eq!(
            compare_text(&text, "", Some(0)),
            Err(CompareError::InputTooLarge)
        );
        assert_eq!(
            compare_text("", &text, None),
            Err(CompareError::InputTooLarge)
        );
    }
    for text in ["\n".repeat(5_000), "\r\n".repeat(5_000), "\r".repeat(5_001)] {
        assert_eq!(compare_text(&text, &text, None).unwrap(), IDENTICAL);
    }
}

#[test]
fn many_hunks_are_complete_or_rejected() {
    let before: String = (0..1_000).map(|i| format!("line {i}\n")).collect();
    let after: String = (0..1_000)
        .map(|i| {
            if i % 12 == 0 {
                format!("changed {i}🙂\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    let expected = legacy(&before, &after).unwrap();
    assert!(expected.matches("@@ -").count() > 50);
    assert_eq!(compare_text(&before, &after, None).unwrap(), expected);
    for limit in [0, 1, 100, expected.len() / 2, expected.len() - 1] {
        assert_eq!(
            compare_text(&before, &after, Some(limit)),
            Err(CompareError::ReportTooLarge)
        );
    }
    assert_eq!(
        compare_text(&before, &after, Some(expected.len())).unwrap(),
        expected
    );
}
