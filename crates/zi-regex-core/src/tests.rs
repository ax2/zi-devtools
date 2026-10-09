use super::*;
#[path = "legacy.rs"]
mod legacy;
fn outcome(value: Result<String>) -> std::result::Result<String, String> {
    value.map_err(|error| format!("{error:#}"))
}
#[test]
fn full_syntax_captures_errors_and_unicode_match_original() {
    let patterns = [
        "",
        "a",
        "(a)?b",
        r"(?P<word>\p{L}+)-(\d+)",
        r"(?m)^.*$",
        r"(?s).+",
        r"\b\w+\b",
        r"(?i)zi",
        r"a{0,3}",
        "(",
        r"(?=a)",
        r"(a)\1",
        "a{1000000}",
    ];
    let inputs = ["", "aab", "甲zi-7\n乙-42", "zi ZI", "中🙂a", "b"];
    for pattern in patterns {
        for input in inputs {
            assert_eq!(
                outcome(test_regex(pattern, input)),
                outcome(legacy::test_regex(pattern, input)),
                "{pattern:?} {input:?}"
            );
        }
    }
    assert_eq!(
        test_regex(r"(zi)-(\d+)", "甲zi-7").unwrap(),
        "#1 字节 3..7: zi-7\n  $1: zi\n  $2: 7\n"
    );
    assert_eq!(test_regex("(a)?b", "b").unwrap(), "#1 字节 0..1: b\n");
    assert_eq!(test_regex("z", "甲").unwrap(), "没有匹配");
    assert_eq!(
        test_regex("", "中a").unwrap(),
        "#1 字节 0..0: \n#2 字节 3..3: \n#3 字节 4..4: \n"
    );
}
#[test]
fn match_and_character_display_boundaries_are_preserved() {
    for count in [99, 100, 101, 200] {
        let input = "a".repeat(count);
        let result = test_regex("a", &input).unwrap();
        assert_eq!(result, legacy::test_regex("a", &input).unwrap());
        assert_eq!(
            result.lines().filter(|line| line.starts_with('#')).count(),
            count.min(100)
        );
        assert_eq!(result.contains("仅展示前 100 个匹配"), count > 100);
    }
    for count in [119, 120, 121] {
        let input = "中".repeat(count);
        let result = test_regex("(.+)", &input).unwrap();
        assert_eq!(result, legacy::test_regex("(.+)", &input).unwrap());
        assert_eq!(result.matches('…').count(), if count > 120 { 2 } else { 0 });
        assert!(result.contains(&format!("字节 0..{}:", count * 3)));
    }
}
#[test]
fn exact_byte_limits_keep_the_full_native_range() {
    for count in [4096, 4097] {
        let pattern = "a".repeat(count);
        let result = test_regex(&pattern, "");
        assert_eq!(result.is_ok(), count == 4096);
        assert_eq!(outcome(result), outcome(legacy::test_regex(&pattern, "")));
    }
    for count in [2 * 1024 * 1024, 2 * 1024 * 1024 + 1] {
        let input = "a".repeat(count);
        let result = test_regex("^$", &input);
        assert_eq!(result.is_ok(), count == 2 * 1024 * 1024);
        assert_eq!(outcome(result), outcome(legacy::test_regex("^$", &input)));
    }
    assert!(
        test_regex(&"中".repeat(1366), "")
            .unwrap_err()
            .to_string()
            .contains("大小限制")
    );
}

#[test]
fn result_budget_rejects_only_proven_oversize_without_changing_full_output() {
    for (pattern, input) in [("", "中a"), ("(a)", "aaaa"), ("z", "甲")] {
        let full = test_regex(pattern, input).unwrap();
        assert_eq!(
            test_regex_with_result_budget(pattern, input, full.len()).unwrap(),
            full
        );
        assert!(
            test_regex_with_result_budget(pattern, input, full.len() - 1)
                .unwrap_err()
                .is::<ReportTooLarge>()
        );
    }
    let pattern = "()".repeat(1000);
    let full = test_regex(&pattern, "aaaaaaa").unwrap();
    assert!(full.len() > 49152);
    assert_eq!(full, legacy::test_regex(&pattern, "aaaaaaa").unwrap());
    assert!(
        test_regex_with_result_budget(&pattern, "aaaaaaa", 49152)
            .unwrap_err()
            .is::<ReportTooLarge>()
    );
    assert!(
        !test_regex_with_result_budget("(", "a", 0)
            .unwrap_err()
            .is::<ReportTooLarge>()
    );
}
