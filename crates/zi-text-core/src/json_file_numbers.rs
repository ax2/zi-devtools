//! Check exact decimal round trips before a JSON file may be rewritten.
pub(crate) fn lossless_numbers(text: &str) -> bool {
    check_json_numbers(text, None)
}

pub(crate) fn lossless_wide_numbers(text: &str, float_round_trips: fn(&str, f64) -> bool) -> bool {
    check_json_numbers(text, Some(float_round_trips))
}

fn check_json_numbers(text: &str, wide_decimal_check: Option<fn(&str, f64) -> bool>) -> bool {
    let wide_integers = wide_decimal_check.is_some();
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut depth = 0usize;
    let mut last_checked = None;
    let depth_limit = if wide_integers { 128 } else { 64 };
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b'"' {
                    if bytes[index] == b'\\' {
                        index += 1;
                    }
                    index += 1;
                }
            }
            b'[' | b'{' => {
                depth += 1;
                if depth > depth_limit {
                    return false;
                }
            }
            b']' | b'}' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            b'-' | b'0'..=b'9' => {
                let start = index;
                while index < bytes.len()
                    && matches!(bytes[index], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    index += 1;
                }
                let token = &text[start..index];
                if wide_integers {
                    // The document parser already checked JSON syntax.
                    // Decimal integers inside i64/u64 never pass through f64.
                    if last_checked == Some(token)
                        || token.parse::<u64>().is_ok()
                        || token.parse::<i64>().is_ok()
                    {
                        continue;
                    }
                    last_checked = Some(token);
                }
                let Ok(number) = serde_json::from_str::<serde_json::Number>(token) else {
                    return false;
                };
                let Some(float) = number.as_f64() else {
                    return false;
                };
                let can_convert = if let Some(float_round_trips) = wide_decimal_check {
                    number.is_i64() || number.is_u64() || float_round_trips(token, float)
                } else {
                    !(float.fract() == 0.0 && float.abs() > 9_007_199_254_740_991.0)
                        && matching_decimals(token, &number.to_string())
                };
                if !can_convert {
                    return false;
                }
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    depth == 0
}

pub(crate) fn matching_decimals(source: &str, written: &str) -> bool {
    decimal(source).is_some_and(|value| Some(value) == decimal(written))
}

pub(crate) fn lossless_decimal(source: &str, value: f64) -> bool {
    let Some(written) = serde_json::Number::from_f64(value) else {
        return false;
    };
    matching_decimals(source, &written.to_string())
}

fn decimal(token: &str) -> Option<(bool, String, i64)> {
    let (negative, token) = token
        .strip_prefix('-')
        .map_or((false, token.strip_prefix('+').unwrap_or(token)), |value| {
            (true, value)
        });
    let (mantissa, exponent) = match token.split_once(['e', 'E']) {
        Some((value, exponent)) => (value, exponent.parse::<i64>().ok()?),
        None => (token, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some((false, "0".into(), 0));
    }
    let trimmed = digits.trim_end_matches('0');
    let zeros = (digits.len() - trimmed.len()) as i64;
    Some((
        negative,
        trimmed.to_owned(),
        exponent
            .checked_sub(fraction.len() as i64)?
            .checked_add(zeros)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_must_round_trip_exact_decimal_value() {
        for value in [
            "0.1",
            "1.2300",
            "1e-5",
            "-0.0",
            "9007199254740991",
            "1.25e3",
            r#"["123456789012345678901234",0.125]"#,
        ] {
            assert!(lossless_numbers(value), "{value}");
        }
        for value in [
            "0.123456789012345678901234567890",
            "18446744073709551616",
            "1e-999",
            "9007199254740992",
            "-9007199254740992",
        ] {
            assert!(!lossless_numbers(value), "{value}");
        }
    }
}
