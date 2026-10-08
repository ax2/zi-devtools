use std::io::{Read, Write};
use zi_diagnostics_wasi::{Envelope, REQUEST_LIMIT, execute_request, serialize_result};
fn main() {
    let mut bytes = Vec::new();
    let result = match std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => execute_request(&bytes),
        Err(_) => Envelope::failure("INVALID_INPUT", "无法读取请求"),
    };
    if std::io::stdout()
        .write_all(&serialize_result(&result))
        .is_err()
    {
        std::process::exit(1);
    }
}
