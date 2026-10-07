//! One bounded stdin request, one stdout result; no files, env or network.
use std::io::{Read, Write};
use zi_text_core::{REQUEST_LIMIT, ResultEnvelope, execute_request, serialize_result};
fn main() {
    let mut bytes = Vec::new();
    let result = match std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => execute_request(&bytes),
        Err(_) => ResultEnvelope::failure("INVALID_INPUT", "无法读取请求"),
    };
    let output = serialize_result(&result);
    if std::io::stdout().write_all(&output).is_err() {
        std::process::exit(1);
    }
}
