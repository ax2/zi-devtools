//! One bounded request/result using the shared frozen protocol, no Host access.
use std::io::{Read, Write};
use zi_text_core::{
    CONTRACT_VERSION, REQUEST_LIMIT, ResultEnvelope, Text, execute_plugin_request, serialize_result,
};

fn execute(capability: &str, input: &str) -> ResultEnvelope {
    let Some(id) = capability.strip_prefix("devtools.trace.") else {
        return ResultEnvelope::failure("UNSUPPORTED_OPERATION", "未注册该工具能力");
    };
    if !zi_trace_core::ACTIONS.iter().any(|action| action.id == id) {
        return ResultEnvelope::failure("UNSUPPORTED_OPERATION", "未注册该工具能力");
    }
    match zi_trace_core::run_plugin(id, input, REQUEST_LIMIT) {
        Ok(text) => ResultEnvelope {
            contract_version: CONTRACT_VERSION,
            ok: true,
            data: Some(Text { text }),
            error: None,
        },
        Err(error)
            if error
                .downcast_ref::<zi_trace_core::ReportTooLarge>()
                .is_some() =>
        {
            ResultEnvelope::failure("INPUT_TOO_LARGE", "结果超过 48 KiB")
        }
        Err(_) => ResultEnvelope::failure("INVALID_INPUT", "输入无法按该操作处理"),
    }
}

fn main() {
    let mut bytes = Vec::new();
    let result = match std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => execute_plugin_request(&bytes, "com.zicode.devtools.trace", execute),
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
    fn output_budget_does_not_change_standalone_or_invalid_input_semantics() {
        for count in [350, 512] {
            let input = "Error\n".repeat(count);
            let standalone = zi_trace_core::java_trace(&input).unwrap();
            assert!(standalone.len() > REQUEST_LIMIT);
            assert_eq!(
                execute("devtools.trace.java.trace", &input)
                    .error
                    .unwrap()
                    .code,
                "INPUT_TOO_LARGE"
            );
        }
        assert_eq!(
            execute("devtools.trace.java.trace", &"Error\n".repeat(513))
                .error
                .unwrap()
                .code,
            "INVALID_INPUT"
        );
    }
    #[test]
    fn full_traces_use_shared_core_and_frozen_errors() {
        for (id, input) in [
            ("java.trace", "java.lang.RuntimeException: test"),
            (
                "django.trace",
                "Traceback (most recent call last):\n  File \"views.py\", line 3, in index\nValueError: test",
            ),
        ] {
            let result = execute(&format!("devtools.trace.{id}"), input);
            assert!(result.ok);
            assert_eq!(
                result.data.unwrap().text,
                zi_trace_core::run(id, input).unwrap()
            );
            assert_eq!(
                execute(&format!("devtools.trace.{id}"), "not a trace")
                    .error
                    .unwrap()
                    .code,
                "INVALID_INPUT"
            );
        }
        assert_eq!(
            execute("devtools.inspect.java.trace", "")
                .error
                .unwrap()
                .code,
            "UNSUPPORTED_OPERATION"
        );
    }
}
