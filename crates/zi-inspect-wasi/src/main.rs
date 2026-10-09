//! One bounded request/result using the shared frozen protocol, no Host access.
use std::io::{Read, Write};
use zi_text_core::{
    CONTRACT_VERSION, REQUEST_LIMIT, ResultEnvelope, Text, execute_plugin_request, serialize_result,
};

fn execute(capability: &str, input: &str) -> ResultEnvelope {
    let Some(id) = capability.strip_prefix("devtools.inspect.") else {
        return ResultEnvelope::failure("UNSUPPORTED_OPERATION", "未注册该工具能力");
    };
    if !zi_inspect_core::ACTIONS
        .iter()
        .any(|action| action.id == id)
    {
        return ResultEnvelope::failure("UNSUPPORTED_OPERATION", "未注册该工具能力");
    }
    // Five fixed keys/values (at least 82 bytes), the report's pretty indentation
    // and line breaks, then outer string quote/newline escaping exceed 150 bytes
    // per point. Only reject
    // reports already guaranteed to exceed the result budget, avoiding wasted
    // fuel constructing output that the serializer must subsequently refuse.
    if id == "unicode.inspect" && input.chars().count() > REQUEST_LIMIT / 150 {
        return ResultEnvelope::failure("INPUT_TOO_LARGE", "结果超过 48 KiB");
    }
    match zi_inspect_core::run(id, input) {
        Ok(text) => ResultEnvelope {
            contract_version: CONTRACT_VERSION,
            ok: true,
            data: Some(Text { text }),
            error: None,
        },
        Err(_) => ResultEnvelope::failure("INVALID_INPUT", "输入无法按该操作处理"),
    }
}

fn main() {
    let mut bytes = Vec::new();
    let result = match std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => execute_plugin_request(&bytes, "com.zicode.devtools.inspect", execute),
        Err(_) => ResultEnvelope::failure("INVALID_INPUT", "无法读取请求"),
    };
    if std::io::stdout()
        .write_all(&serialize_result(&result))
        .is_err()
    {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guaranteed_oversize_reports_are_refused_but_normalization_and_small_reports_remain() {
        let input = "a".repeat(1024);
        assert_eq!(
            execute("devtools.inspect.unicode.inspect", &input)
                .error
                .unwrap()
                .code,
            "INPUT_TOO_LARGE"
        );
        assert_eq!(
            execute("devtools.inspect.unicode.nfc", &input)
                .data
                .unwrap()
                .text,
            input
        );
        assert!(execute("devtools.inspect.unicode.inspect", &"a".repeat(64)).ok);
        for c in ['a', '\0', '中', '🙂', '\u{200b}'] {
            let report: serde_json::Value =
                serde_json::from_str(&zi_inspect_core::unicode(&c.to_string(), 0).unwrap())
                    .unwrap();
            assert!(serde_json::to_vec(&report["codePoints"][0]).unwrap().len() >= 80);
            let empty = serialize_result(&execute("devtools.inspect.unicode.inspect", ""));
            let single =
                serialize_result(&execute("devtools.inspect.unicode.inspect", &c.to_string()));
            assert!(single.len() >= empty.len() + 150);
        }
    }
}
