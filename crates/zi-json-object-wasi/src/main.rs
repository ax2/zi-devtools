//! One bounded stdin request, one stdout response; no files, env or network.
use std::io::{Read, Write};

fn main() {
    let mut bytes = Vec::new();
    let response = match std::io::stdin()
        .take((zi_json_object_wasi::REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Ok(_) => zi_json_object_wasi::execute_request(&bytes),
        Err(_) => b"{\"contractVersion\":\"1.0.0-rc.1\",\"ok\":false,\"data\":null,\"error\":{\"code\":\"INVALID_INPUT\",\"message\":\"could not read request\"}}".to_vec(),
    };
    if std::io::stdout().write_all(&response).is_err() {
        std::process::exit(1);
    }
}
