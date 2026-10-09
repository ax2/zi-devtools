//! Verification-only adapter: same serializer, synthetic envelope on stdin.
//! Not included in the plugin runtime or registered as a capability.
use std::io::{Read, Write};
use zi_diagnostics_wasi::{Envelope, serialize_result};
fn main() {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .unwrap();
    let envelope: Envelope = serde_json::from_slice(&bytes).unwrap();
    std::io::stdout()
        .write_all(&serialize_result(&envelope))
        .unwrap();
}
