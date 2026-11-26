//! Parse a small flat numeric subset without two general YAML passes.
use serde_json::{Map, Number, Value};
use zi_text_core::json_file::matching_decimals;

// Validate the entire subset before allocating output. Complex YAML falls
// back to the full parser without paying to convert a numeric prefix twice.
pub(super) fn parse(input: &str) -> Option<Value> {
    let mut previous = None;
    if let Some(items) = input
        .trim_start_matches([' ', '\r', '\n'])
        .strip_prefix('[')
    {
        let items = items
            .trim_end_matches([' ', '\r', '\n'])
            .strip_suffix(']')?;
        let values = if items.trim_matches(' ').is_empty() {
            Vec::new()
        } else {
            if !items
                .bytes()
                .all(|byte| matches!(byte, b' ' | b',' | b'-' | b'.' | b'0'..=b'9'))
                || !items.split(',').all(decimal_syntax)
            {
                return None;
            }
            items
                .split(',')
                .map(|item| number(item, &mut previous))
                .collect::<Option<Vec<_>>>()?
        };
        return Some(Value::Array(values));
    }

    // The generic path handles ordinary maps. Probe only dense inputs so
    // an ordinary mixed YAML document doesn't pay for a flat-map parse.
    const MIN_DENSE_MAP_SEPARATORS: usize = 511;
    if input
        .match_indices('\n')
        .nth(MIN_DENSE_MAP_SEPARATORS - 1)
        .is_none()
        || input.bytes().any(|byte| {
            matches!(
                byte,
                b'[' | b']' | b'{' | b'}' | b'&' | b'*' | b'!' | b'#' | b'\'' | b'"' | b'\t'
            )
        })
        || !input.lines().all(|line| {
            line.split_once(": ")
                .is_some_and(|(key, value)| plain_key(key) && decimal_syntax(value))
        })
    {
        return None;
    }
    let mut map = Map::new();
    for line in input.lines() {
        let (key, value) = line.split_once(": ")?;
        let value = number(value, &mut previous)?;
        if map.insert(key.to_owned(), value).is_some() {
            // Preserve the full parser's duplicate-key and syntax diagnostics.
            return None;
        }
    }
    if map.is_empty() {
        return None;
    }
    map.sort_keys();
    Some(Value::Object(map))
}

fn plain_key(key: &str) -> bool {
    key.len() <= 1024
        && key.as_bytes().split_first().is_some_and(|(&first, rest)| {
            (first.is_ascii_alphabetic() || first == b'_')
                && rest
                    .iter()
                    .all(|&byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn decimal_syntax(token: &str) -> bool {
    let token = token.trim_matches(' ');
    let unsigned = token.strip_prefix('-').unwrap_or(token);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    digits(whole) && !(whole.starts_with('0') && whole.len() > 1) && fraction.is_none_or(digits)
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn number<'a>(token: &'a str, previous: &mut Option<(&'a str, Number)>) -> Option<Value> {
    let token = token.trim_matches(' ');
    if let Some((last_token, parsed)) = previous
        && *last_token == token
    {
        return Some(Value::Number(parsed.clone()));
    }
    let parsed = if token.contains('.') {
        let number = Number::from_f64(token.parse().ok()?)?;
        let written = number.to_string();
        let same_fixed_decimal = token == written
            || (written.contains('.')
                && written.trim_end_matches('0') == token.trim_end_matches('0'));
        (same_fixed_decimal || matching_decimals(token, &written)).then_some(number)?
    } else if token.starts_with('-') {
        Number::from(token.parse::<i64>().ok()?)
    } else {
        Number::from(token.parse::<u64>().ok()?)
    };
    *previous = Some((token, parsed.clone()));
    Some(Value::Number(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_numeric_values_match_general_yaml() {
        let row = "12.3400,0.10,-0.0,18446744073709551615";
        let sequence = format!("[{}]", [row; 160].join(","));
        let map = (0..600)
            .rev()
            .map(|i| format!("n{i}: 12.34"))
            .collect::<Vec<_>>()
            .join("\r\n");
        for input in [sequence, map] {
            let optimized = serde_json::to_string_pretty(&parse(&input).unwrap()).unwrap();
            assert_eq!(optimized, zi_inspect_core::yaml_to_json(&input).unwrap());
        }
    }

    #[test]
    fn general_parser_retains_complex_or_invalid_yaml() {
        let map = (0..511)
            .map(|i| format!("n{i}: 12.34"))
            .collect::<Vec<_>>()
            .join("\n");
        for entry in [
            "other: [true, null]",
            "other: 1e30",
            "other: 001",
            "n0: 8",
            "'n0': 8",
            "other: 0.1234567890123456789",
        ] {
            assert!(parse(&format!("{map}\n{entry}")).is_none(), "{entry}");
        }
        let long_key = "a".repeat(1025);
        assert!(parse(&format!("{map}\n{long_key}: 1")).is_none());
        assert!(parse("[1,2,,3]").is_none());
        assert!(parse("[1,2,3] # comment").is_none());
    }
}
