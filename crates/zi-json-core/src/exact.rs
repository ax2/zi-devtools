mod number;
mod value;

use self::value::{Value, parse};
use super::{ReportTooLarge, bounded};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    io::{self, Write},
};

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Added,
    Changed,
    Removed,
}

#[derive(Serialize)]
struct Change<'v, 's> {
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<&'v Value<'s>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<&'v Value<'s>>,
    kind: Kind,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'v, 's> {
    array_order: &'static str,
    changes: Vec<Change<'v, 's>>,
    equal: bool,
}

struct Comparison<'v, 's> {
    unordered: bool,
    remaining: usize,
    changes: Vec<Change<'v, 's>>,
}

impl<'v, 's> Comparison<'v, 's> {
    fn record(
        &mut self,
        path: &str,
        before: Option<&'v Value<'s>>,
        after: Option<&'v Value<'s>>,
    ) -> Result<()> {
        // Changed arrays are pretty-printed under the report, changes list,
        // change record and payload array. Even one-byte elements need a
        // newline and eight-byte indentation. Count that proven minimum
        // without visiting each element or building a partial report.
        let minimum_array_bytes = |value: Option<&Value<'_>>| match value {
            Some(Value::Array(items)) => 10 * items.len() + items.len().saturating_sub(1),
            _ => 0,
        };
        let minimum = path.len() + minimum_array_bytes(before) + minimum_array_bytes(after);
        self.remaining = self.remaining.checked_sub(minimum).ok_or(ReportTooLarge)?;
        let kind = match (before, after) {
            (None, Some(_)) => Kind::Added,
            (Some(_), None) => Kind::Removed,
            (Some(_), Some(_)) => Kind::Changed,
            (None, None) => return Ok(()),
        };
        self.changes.push(Change {
            after,
            before,
            kind,
            path: path.to_owned(),
        });
        Ok(())
    }

    fn compare(&mut self, a: &'v Value<'s>, b: &'v Value<'s>, path: &str) -> Result<()> {
        if a == b
            || (self.unordered
                && ((matches!(a, Value::Array(_)) && matches!(b, Value::Array(_)))
                    || self.changes.len() >= 10_000)
                && a.canonical() == b.canonical())
        {
            return Ok(());
        }
        ensure!(self.changes.len() < 10_000, "差异超过 10000 项，请缩小输入");
        match (a, b) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
                    let child = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    self.compare_optional(a.get(key), b.get(key), &child)?;
                }
            }
            (Value::Array(a), Value::Array(b)) if !self.unordered => {
                for i in 0..a.len().max(b.len()) {
                    self.compare_optional(a.get(i), b.get(i), &format!("{path}/{i}"))?;
                }
            }
            _ => self.record(path, Some(a), Some(b))?,
        }
        Ok(())
    }

    fn compare_optional(
        &mut self,
        before: Option<&'v Value<'s>>,
        after: Option<&'v Value<'s>>,
        path: &str,
    ) -> Result<()> {
        match (before, after) {
            (Some(a), Some(b)) => self.compare(a, b, path),
            (None, None) => Ok(()),
            _ => {
                ensure!(self.changes.len() < 10_000, "差异超过 10000 项，请缩小输入");
                self.record(path, before, after)
            }
        }
    }
}

/// Exact numeric comparison for Studio adapters. The standalone entry keeps its legacy policy.
pub fn json_diff_exact_with_result_budget(
    left: &str,
    right: &str,
    unordered: bool,
    result_budget: usize,
) -> Result<String> {
    bounded(left)?;
    bounded(right)?;
    let left = parse(left).context("左侧 JSON 无效")?;
    let right = parse(right).context("右侧 JSON 无效")?;
    let mut comparison = Comparison {
        unordered,
        remaining: result_budget,
        changes: Vec::new(),
    };
    comparison.compare(&left, &right, "")?;
    let report = Report {
        array_order: if unordered {
            "ignored (duplicates retained)"
        } else {
            "by index"
        },
        equal: comparison.changes.is_empty(),
        changes: comparison.changes,
    };
    let mut output = BoundedOutput {
        bytes: Vec::new(),
        limit: result_budget.min(8 * 1024 * 1024),
        overflowed: false,
    };
    let result = serde_json::to_writer_pretty(&mut output, &report);
    if output.overflowed {
        return Err(ReportTooLarge.into());
    }
    result?;
    Ok(String::from_utf8(output.bytes)?)
}

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

impl Write for BoundedOutput {
    fn write(&mut self, incoming: &[u8]) -> io::Result<usize> {
        if incoming.len() > self.limit - self.bytes.len() {
            self.overflowed = true;
            return Err(io::Error::other(ReportTooLarge));
        }
        self.bytes.write(incoming)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_without_rounding_and_serializes_original_number_tokens() {
        for unordered in [false, true] {
            for (a, b) in [
                ("0.123456789012345678901", "0.123456789012345678902"),
                ("18446744073709551616", "18446744073709551617"),
                ("1e99999", "1e100000"),
            ] {
                let report = json_diff_exact_with_result_budget(
                    &format!(r#"{{"amount":[{a}]}}"#),
                    &format!(r#"{{"amount":[{b}]}}"#),
                    unordered,
                    48 * 1024,
                )
                .unwrap();
                assert!(report.contains(r#""equal": false"#));
                assert!(report.contains(a), "{report}");
                assert!(report.contains(b), "{report}");
            }
        }
    }

    #[test]
    fn preserves_number_kinds_neg_zero_and_decimal_equivalence() {
        for (a, b, equal) in [
            ("1", "1.0", false),
            ("1.0", "1e0", true),
            ("-0", "0", false),
            ("-0.0", "0.0", true),
            ("1.25e0", "1.25", true),
            ("1e100000000000000000000", "10e99999999999999999999", true),
            (
                "1e-100000000000000000000",
                "10e-100000000000000000001",
                true,
            ),
            ("1e10000000000000000000", "10e9999999999999999999", true),
        ] {
            assert_eq!(parse(a).unwrap() == parse(b).unwrap(), equal, "{a} {b}");
        }
        let left = r#"[1,1.0,1e0,{"v":[0.01,2e99999]}]"#;
        let right = r#"[{"v":[2e99999,1e-2]},1e0,1,1.0]"#;
        assert!(
            json_diff_exact_with_result_budget(left, right, true, 48 * 1024)
                .unwrap()
                .contains(r#""equal": true"#)
        );
        assert!(
            json_diff_exact_with_result_budget("[1,1.0,1e0]", "[1,1.0,1]", true, 48 * 1024)
                .unwrap()
                .contains(r#""equal": false"#)
        );
    }

    #[test]
    fn validates_full_json_before_comparing_and_preserves_exact_limits() {
        for bad in [
            "01",
            "1e-",
            "1.",
            ".1",
            "--1",
            "[] []",
            "{2:0}",
            "[1 2]",
            r#"{"x":"\uD800","n":1e99999}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let deep = |depth: usize| format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        assert!(parse(&deep(127)).is_ok());
        assert!(parse(&deep(128)).is_err());
        let exact_size = "1e".to_string() + "9".repeat(1024 * 1024 - 2).as_str();
        assert!(
            json_diff_exact_with_result_budget(&exact_size, &exact_size, false, 8 * 1024 * 1024)
                .unwrap()
                .contains(r#""equal": true"#)
        );
        assert!(
            json_diff_exact_with_result_budget(&(exact_size + " "), "0", false, 8 * 1024 * 1024)
                .is_err()
        );
        assert!(
            json_diff_exact_with_result_budget(r#"{"a/b~":0}"#, "{}", false, 6)
                .unwrap_err()
                .is::<ReportTooLarge>()
        );
    }
    #[test]
    fn rejects_a_proven_oversized_pretty_array_without_losing_equal_arrays() {
        let left = format!("[{}0]", "1,".repeat(3999));
        let right = format!("[{}2]", "1,".repeat(3999));
        assert!(
            json_diff_exact_with_result_budget(&left, &right, true, 48 * 1024)
                .unwrap_err()
                .is::<ReportTooLarge>()
        );
        assert!(
            json_diff_exact_with_result_budget(&left, &left, true, 96)
                .unwrap()
                .contains(r#""equal": true"#)
        );
        let result = json_diff_exact_with_result_budget("[1,1]", "[2,1]", true, 48 * 1024).unwrap();
        assert!(json_diff_exact_with_result_budget("[1,1]", "[2,1]", true, result.len()).is_ok());
        assert!(
            json_diff_exact_with_result_budget("[1,1]", "[2,1]", true, result.len() - 1).is_err()
        );
    }
}
