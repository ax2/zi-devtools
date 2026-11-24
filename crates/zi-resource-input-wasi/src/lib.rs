//! Bounded reads of the Host's approved WASI resource snapshot.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "zicode_resource_input_draft1")]
unsafe extern "C" {
    #[link_name = "size"]
    fn resource_size(index: i32) -> i64;
    #[link_name = "read"]
    fn resource_read(index: i32, offset: i64, destination: *mut u8, length: i32) -> i32;
}

#[cfg(target_arch = "wasm32")]
pub fn read_prefix(
    index: i32,
    expected_size: usize,
    max_bytes: usize,
) -> Result<Vec<u8>, &'static str> {
    const FAILED: &str = "RESOURCE_READ_FAILED";
    const CHUNK: usize = 64 * 1024;

    let expected = i64::try_from(expected_size).map_err(|_| FAILED)?;
    // These imports only expose the snapshots enumerated in the Host envelope.
    if unsafe { resource_size(index) } != expected {
        return Err(FAILED);
    }
    let mut bytes = vec![0; expected_size.min(max_bytes)];
    let mut offset = 0;
    while offset < bytes.len() {
        let length = CHUNK.min(bytes.len() - offset);
        let read = unsafe {
            resource_read(
                index,
                offset as i64,
                bytes[offset..].as_mut_ptr(),
                length as i32,
            )
        };
        if !(1..=length as i32).contains(&read) {
            return Err(FAILED);
        }
        offset += read as usize;
    }
    Ok(bytes)
}
