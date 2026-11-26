//! File conversion validates that the original data survives the output format.
mod flat_numbers;

use std::collections::HashSet;
use yaml_rust2::{
    parser::{Event, Parser, Tag},
    scanner::TScalarStyle,
};
use zi_text_core::json_file::{lossless_decimal, matching_decimals, strict_wide_value};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Syntax,
    Policy,
}

pub(super) fn convert(id: &str, input: &str) -> Result<String, Error> {
    if id == "yaml.from_json" {
        let value = strict_wide_value(input, |source, value| {
            // serde_yaml_ng emits finite f64 scalars with the same Ryu formatter.
            matching_decimals(source, ryu::Buffer::new().format_finite(value))
        })
        .map_err(|code| {
            if code == "JSON_POLICY_REJECTED" {
                Error::Policy
            } else {
                Error::Syntax
            }
        })?;
        return serde_yaml_ng::to_string(&value).map_err(|_| Error::Policy);
    }

    let value = if let Some(value) = flat_numbers::parse(input) {
        value
    } else {
        check_yaml_numbers(input)?;
        zi_text_core::deserialize_with_wide_integers(serde_yaml_ng::Deserializer::from_str(input))
            .map_err(|_| Error::Policy)?
    };
    serde_json::to_string_pretty(&value).map_err(|_| Error::Policy)
}

enum Collection {
    Sequence,
    Mapping { key: bool },
}

fn consume_node(collection: Option<&mut Collection>) -> bool {
    if let Some(Collection::Mapping { key }) = collection {
        let was_key = *key;
        *key = !was_key;
        was_key
    } else {
        false
    }
}

fn check_yaml_numbers(input: &str) -> Result<(), Error> {
    let mut parser = Parser::new_from_str(input);
    let mut collections = Vec::new();
    let mut invalid_key_anchors = HashSet::new();
    let mut documents = 0;
    loop {
        let (event, _) = parser.next_token().map_err(|_| Error::Syntax)?;
        match event {
            Event::StreamEnd => return Ok(()),
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 {
                    return Err(Error::Syntax);
                }
            }
            Event::MappingStart(..) | Event::SequenceStart(..) => {
                if collections.len() >= 128 {
                    return Err(Error::Policy);
                }
                consume_node(collections.last_mut());
                collections.push(if matches!(event, Event::MappingStart(..)) {
                    Collection::Mapping { key: true }
                } else {
                    Collection::Sequence
                });
            }
            Event::MappingEnd | Event::SequenceEnd => {
                collections.pop();
            }
            Event::Alias(anchor) => {
                if !consume_node(collections.last_mut()) && invalid_key_anchors.contains(&anchor) {
                    return Err(Error::Policy);
                }
            }
            Event::Scalar(text, style, anchor, tag) => {
                let key = consume_node(collections.last_mut());
                // Untagged keys remain strings unless an alias uses them as values.
                if (!key || anchor != 0) && !scalar_round_trips(&text, style, tag.as_ref()) {
                    if !key {
                        return Err(Error::Policy);
                    }
                    invalid_key_anchors.insert(anchor);
                }
            }
            _ => {}
        }
    }
}

fn scalar_round_trips(text: &str, style: TScalarStyle, tag: Option<&Tag>) -> bool {
    let is_tag = |name: &str| {
        tag.is_some_and(|tag| name.strip_prefix(&tag.handle) == Some(tag.suffix.as_str()))
    };
    let float_tag = is_tag("tag:yaml.org,2002:float");
    let integer_tag = is_tag("tag:yaml.org,2002:int");
    if (style == TScalarStyle::Plain && tag.is_none() || integer_tag)
        && let Some(exact) = radix_integer_round_trips(text)
    {
        return exact;
    }
    if !float_tag {
        if style != TScalarStyle::Plain || tag.is_some() {
            // The serde pass validates explicitly typed integers, which use
            // i64/u64 and reject unsupported magnitudes; strings stay strings.
            return true;
        }
        let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
        if (digits.len() > 1
            && digits.starts_with('0')
            && digits.bytes().all(|b| b.is_ascii_digit()))
            || text.parse::<i64>().is_ok()
            || text.parse::<u64>().is_ok()
        {
            return true;
        }
    }
    let decimal = text.strip_prefix('+').unwrap_or(text);
    if [".nan", ".inf", "-.inf"]
        .iter()
        .any(|nonfinite| decimal.eq_ignore_ascii_case(nonfinite))
    {
        return false;
    }
    if !float_tag
        && !decimal
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'.' | b'+' | b'-' | b'e' | b'E'))
    {
        return true;
    }
    match decimal.parse::<f64>() {
        Ok(value) => lossless_decimal(text, value),
        Err(_) => !float_tag,
    }
}

fn radix_integer_round_trips(text: &str) -> Option<bool> {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (digits, radix) = if let Some(digits) = unsigned.strip_prefix("0x") {
        (digits, 16)
    } else if let Some(digits) = unsigned.strip_prefix("0o") {
        (digits, 8)
    } else {
        (unsigned.strip_prefix("0b")?, 2)
    };
    if digits.is_empty() || !digits.chars().all(|digit| digit.is_digit(radix)) {
        return None;
    }
    Some(if text.starts_with('-') {
        i64::from_str_radix(&format!("-{digits}"), radix).is_ok()
    } else {
        u64::from_str_radix(digits, radix).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_lossy_numbers_and_duplicate_projected_keys_before_export() {
        for (direction, inputs) in [
            (
                "yaml.from_json",
                [
                    r#"{"v":0.123456789012345678901234567890}"#,
                    r#"{"v":18446744073709551616}"#,
                    r#"{"v":1e-700}"#,
                    r#"{"v":1,"\u0076":2}"#,
                    r#"{"v":[{"inside":1,"inside":2}]}"#,
                ],
            ),
            (
                "yaml.to_json",
                [
                    "v: 0.123456789012345678901234567890",
                    "v: 18446744073709551616",
                    "v: 1e-700",
                    "v: 1\n'v': 2\n",
                    "v: { nested: [ { inside: 1, inside: 2 } ] }\n",
                ],
            ),
        ] {
            for input in inputs {
                assert_eq!(convert(direction, input), Err(Error::Policy), "{input}");
            }
        }
        for input in [
            "v: .inf",
            "v: .nan",
            "v: !!float '0.1234567890123456789'",
            "v: &rounded {value: 1e-700}\ncopy: *rounded",
            "1e-700: data\nv: 0.1234567890123456789",
            "1: a\n'1': b",
            "v: 0x100000000000000000000000000000000",
            "v: !<tag:yaml.org,2002:float> '1e-700'",
            "&a 1e-700: value\nv: *a",
        ] {
            assert_eq!(
                convert("yaml.to_json", input),
                Err(Error::Policy),
                "{input}"
            );
        }
    }

    #[test]
    fn keeps_exact_scalars_and_existing_text_styles() {
        for input in [
            "large: 18446744073709551615\nsmall: -9223372036854775808\ndecimal: 12.3400\nwide: 9007199254740993\nnested: [true, 中, null, .5]\n",
            "quoted: '0.123456789012345678901234567890'\ntext: !!str 1e-700\nblock: |-\n  1e-700\nfolded: >-\n  12.34000\n",
            "key: &shared { value: +1.25e1 }\ncopy: *shared\noctal: 0o10\nhex: 0x10\nleading: 001\n",
            "1e-700: leave the mapping key unchanged\nscientific: 1e30\n",
            "a: { style: true, sub: [1, 2, 3] }\n",
            "&a 1e-700: unchanged mapping key\n",
            "%TAG !foo! tag:example.net,2026:\n---\nv: !foo!test 1e-700",
            "hex_max: 0xffffffffffffffff\nhex_min: -0x8000000000000000\n",
            "---\r\nwide: 9007199254740993\r\n",
        ] {
            assert_eq!(
                convert("yaml.to_json", input).unwrap(),
                zi_inspect_core::yaml_to_json(input).unwrap(),
                "{input}"
            );
        }
        for input in [
            r#"{"small":-9223372036854775808,"large":18446744073709551615,"wide":9007199254740993,"v":12.3400,"quoted":"1e-700"}"#,
            "[0,0.1,1e30,null,true]",
        ] {
            assert_eq!(
                convert("yaml.from_json", input).unwrap(),
                zi_inspect_core::json_to_yaml(input).unwrap(),
                "{input}"
            );
        }
        assert_eq!(
            convert("yaml.to_json", "---\na: 1\n---\nb: 2"),
            Err(Error::Syntax)
        );
    }
}
