//! Check exact decimal round trips before a JSON file may be rewritten.
pub(crate) fn lossless_numbers(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut depth = 0usize;
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
                if depth > 64 {
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
                let Ok(number) = serde_json::from_str::<serde_json::Number>(token) else {
                    return false;
                };
                let Some(float) = number.as_f64() else {
                    return false;
                };
                if float.fract() == 0.0 && float.abs() > 9_007_199_254_740_991.0 {
                    return false;
                }
                let Some(original) = decimal(token) else {
                    return false;
                };
                if Some(original) != decimal(&number.to_string()) {
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

fn decimal(token: &str) -> Option<(bool, String, i64)> {
    let (negative, token) = token
        .strip_prefix('-')
        .map_or((false, token), |value| (true, value));
    let (mantissa, exponent) = match token.split_once(['e', 'E']) {
        Some((value, exponent)) => (value, exponent.parse::<i64>().ok()?),
        None => (token, 0),
    };
    let fraction = mantissa.split_once('.').map_or(0, |(_, value)| value.len()) as i64;
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some((false, "0".into(), 0));
    }
    let trimmed = digits.trim_end_matches('0');
    let zeros = (digits.len() - trimmed.len()) as i64;
    Some((
        negative,
        trimmed.to_owned(),
        exponent.checked_sub(fraction)?.checked_add(zeros)?,
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
