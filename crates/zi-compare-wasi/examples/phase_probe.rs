//! Instrumented diagnostic only. The production command has no phase import.
use std::io::{Read, Write};
use zi_compare_wasi::{Phase, REQUEST_LIMIT, execute_request_observed};

#[cfg(target_os = "wasi")]
#[link(wasm_import_module = "zicode_diagnostic")]
unsafe extern "C" {
    fn phase(id: u32);
}

fn observe(value: Phase) {
    #[cfg(target_os = "wasi")]
    unsafe {
        phase(value as u32);
    }
    #[cfg(not(target_os = "wasi"))]
    eprintln!("diagnostic-phase:{}", value as u32);
}

fn main() {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .expect("diagnostic stdin");
    let output = execute_request_observed(&bytes, observe);
    std::io::stdout()
        .write_all(&output)
        .expect("diagnostic stdout");
}
