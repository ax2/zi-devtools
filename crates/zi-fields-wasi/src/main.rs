//! Experimental one-request WASI command; no preopened directories or Host calls.
use std::io::{Read, Write};
fn main() {
    let mut bytes = Vec::new();
    let output = match std::io::stdin()
        .take((zi_fields_wasi::REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => zi_fields_wasi::execute_request(&bytes),
        Err(_) => zi_fields_wasi::execute_request(b""),
    };
    if std::io::stdout().write_all(&output).is_err() {
        std::process::exit(1);
    }
}
