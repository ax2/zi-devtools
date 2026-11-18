//! Resource-sized strict JSON actions, sharing the inline parser and serde formatter.
use serde_json::json;
use std::io::{self, Write};

pub const MAX_INPUT: usize = 1024 * 1024;
pub const MAX_OUTPUT: usize = 8 * MAX_INPUT;

#[derive(Clone, Copy)]
pub enum Action {
    Format,
    Minify,
    Validate,
}

pub fn run(input: &str, action: Action) -> Result<String, &'static str> {
    if input.len() > MAX_INPUT {
        return Err("INPUT_TOO_LARGE");
    }
    let value = crate::parse_strict_json(input).map_err(|error| {
        if error.is_data() || error.to_string().starts_with("number out of range") {
            "JSON_POLICY_REJECTED"
        } else {
            "INVALID_JSON"
        }
    })?;
    if !crate::json_file_numbers::lossless_numbers(input) {
        return Err("JSON_POLICY_REJECTED");
    }
    let value = if matches!(action, Action::Validate) {
        json!({"valid": true})
    } else {
        value
    };
    let mut writer = Bounded(Vec::new());
    let result = if matches!(action, Action::Format) {
        serde_json::to_writer_pretty(&mut writer, &value)
    } else {
        serde_json::to_writer(&mut writer, &value)
    };
    result.map_err(|_| "RESULT_TOO_LARGE")?;
    String::from_utf8(writer.0).map_err(|_| "INVALID_UTF8")
}

struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_OUTPUT - self.0.len() {
            return Err(io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    #[test]
    fn matches_standalone_on_safe_values_and_refuses_lossy_inputs() {
        for source in [
            r#"{"a":[1,true,null,"字"],"b":-2.5}"#,
            "0",
            "false",
            "null",
            "[]",
        ] {
            for (action, pretty) in [(Action::Format, true), (Action::Minify, false)] {
                assert_eq!(
                    run(source, action).unwrap(),
                    crate::json(source, pretty).unwrap()
                );
            }
            assert_eq!(run(source, Action::Validate).unwrap(), r#"{"valid":true}"#);
        }
        for source in [
            "",
            "{",
            r#"{"x":1,"\u0078":2}"#,
            "9007199254740992",
            "1e999",
            "[] []",
        ] {
            assert!(run(source, Action::Format).is_err(), "{source}");
        }
    }
    #[test]
    fn exact_input_limit_and_bounded_writer() {
        let input = format!("\"{}\"", "a".repeat(MAX_INPUT - 2));
        assert_eq!(run(&input, Action::Minify).unwrap(), input);
        assert_eq!(run(&(input + " "), Action::Minify), Err("INPUT_TOO_LARGE"));
        let mut writer = Bounded(vec![0; MAX_OUTPUT - 1]);
        assert_eq!(writer.write(b"x").unwrap(), 1);
        assert_eq!(
            writer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
    }
    #[test]
    fn depth_and_oversize_output_refuse_instead_of_truncating() {
        let nested = |n| format!("{}0{}", "[".repeat(n), "]".repeat(n));
        assert!(run(&nested(64), Action::Format).is_ok());
        assert_eq!(
            run(&nested(65), Action::Format),
            Err("JSON_POLICY_REJECTED")
        );
        let mut value = Value::Array(vec![json!(0); 70000]);
        for _ in 0..63 {
            value = json!([value]);
        }
        assert_eq!(
            run(&value.to_string(), Action::Format).map(|text| text.len()),
            Err("RESULT_TOO_LARGE")
        );
    }
}

#[cfg(test)]
mod legacy_examples {
    #[test]
    fn native_legacy_examples_show_why_file_mode_refuses_lossy_inputs() {
        for source in [
            r#"{"x":1,"\u0078":2}"#,
            "18446744073709551616",
            "0.123456789012345678901234567890",
        ] {
            let legacy = crate::json(source, false).unwrap();
            assert_ne!(legacy, source);
            eprintln!(
                "standalone legacy input={source} output={legacy}; file mode explicitly refuses"
            );
            for action in [
                super::Action::Format,
                super::Action::Minify,
                super::Action::Validate,
            ] {
                assert_eq!(super::run(source, action), Err("JSON_POLICY_REJECTED"));
            }
        }
        assert_eq!(
            super::run("{", super::Action::Validate),
            Err("INVALID_JSON")
        );
    }
}
