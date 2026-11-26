//! Bound JSON parsing work separately from the approved file size.
use crate::Failure;

const MAX_WORK_BYTES: usize = 352 * 1024;
const MAX_DECIMAL_MARKS: usize = 10_000;
const MAX_SEPARATORS: usize = 16_416;
const MARK_COST: usize = 24;
const TOO_COMPLEX: Failure = (
    "INPUT_TOO_COMPLEX",
    "这组向量的小数、指数或 JSON 内容超过预览工作量上限。",
);

pub(super) fn container(raw: &[u8], invalid: Failure) -> Result<&[u8], Failure> {
    let start = memchr::memchr2(b'[', b'{', raw).ok_or(invalid)?;
    let end = memchr::memrchr2(b']', b'}', raw).ok_or(invalid)?;
    if start >= end || !is_padding(&raw[..start]) || !is_padding(&raw[end + 1..]) {
        return Err(invalid);
    }
    Ok(&raw[start..=end])
}

pub(super) fn check_work(files: &[&[u8]]) -> Result<(), Failure> {
    let mut bytes = 0;
    let mut marks = 0;
    let mut separators = 0;
    for file in files {
        bytes += file.len();
        if bytes > MAX_WORK_BYTES {
            return Err(TOO_COMPLEX);
        }
        let (extra_marks, extra_separators) = count_work(file);
        marks += extra_marks;
        separators += extra_separators;
    }
    if marks > MAX_DECIMAL_MARKS
        || separators > MAX_SEPARATORS
        || bytes + marks * MARK_COST > MAX_WORK_BYTES
    {
        return Err(TOO_COMPLEX);
    }
    Ok(())
}

fn json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\n' | b'\t' | b'\r')
}

fn scalar_work(bytes: &[u8]) -> (usize, usize) {
    let mut marks = 0;
    let mut separators = 0;
    for byte in bytes {
        marks += usize::from(matches!(byte, b'.' | b'e' | b'E'));
        separators += usize::from(*byte == b',');
    }
    (marks, separators)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_padding(bytes: &[u8]) -> bool {
    bytes.iter().copied().all(json_whitespace)
}

#[cfg(not(target_arch = "wasm32"))]
fn count_work(bytes: &[u8]) -> (usize, usize) {
    scalar_work(bytes)
}

#[cfg(target_arch = "wasm32")]
fn is_padding(bytes: &[u8]) -> bool {
    // The Studio Worker uses a SIMD-capable Wasmtime, as with the clock backup tool.
    unsafe { wasm::padding(bytes) }
}

#[cfg(target_arch = "wasm32")]
fn count_work(bytes: &[u8]) -> (usize, usize) {
    unsafe { wasm::work(bytes) }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{json_whitespace, scalar_work};
    use core::arch::wasm32::*;

    #[target_feature(enable = "simd128")]
    pub(super) unsafe fn padding(bytes: &[u8]) -> bool {
        let (chunks, tail) = bytes.as_chunks::<16>();
        chunks.iter().all(|chunk| {
            // Each SIMD read stays inside an entire chunk.
            let word = unsafe { v128_load(chunk.as_ptr().cast()) };
            let horizontal = v128_or(
                u8x16_eq(word, u8x16_splat(b' ')),
                u8x16_eq(word, u8x16_splat(b'\t')),
            );
            let vertical = v128_or(
                u8x16_eq(word, u8x16_splat(b'\r')),
                u8x16_eq(word, u8x16_splat(b'\n')),
            );
            u8x16_all_true(v128_or(horizontal, vertical))
        }) && tail.iter().copied().all(json_whitespace)
    }

    #[target_feature(enable = "simd128")]
    pub(super) unsafe fn work(bytes: &[u8]) -> (usize, usize) {
        let (chunks, tail) = bytes.as_chunks::<16>();
        let mut marks = 0;
        let mut separators = 0;
        for chunk in chunks {
            let word = unsafe { v128_load(chunk.as_ptr().cast()) };
            let exp = v128_or(
                u8x16_eq(word, u8x16_splat(b'e')),
                u8x16_eq(word, u8x16_splat(b'E')),
            );
            let decimal = v128_or(exp, u8x16_eq(word, u8x16_splat(b'.')));
            marks += u8x16_bitmask(decimal).count_ones() as usize;
            separators += u8x16_bitmask(u8x16_eq(word, u8x16_splat(b','))).count_ones() as usize;
        }
        let (tail_marks, tail_separators) = scalar_work(tail);
        (marks + tail_marks, separators + tail_separators)
    }
}
