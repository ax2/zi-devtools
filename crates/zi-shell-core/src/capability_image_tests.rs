use super::*;
use std::io::Cursor;
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 1024];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 60, 128);
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[134..136].copy_from_slice(&1u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[392..400].copy_from_slice(b".zicap\0\0");
    put32(&mut bytes, 408, 512);
    put32(&mut bytes, 412, 512);
    put32(&mut bytes, 428, 0x4000_0040);
    let payload = Manifest::current().encode();
    bytes[512..520].copy_from_slice(b"ZICAP001");
    put32(&mut bytes, 520, payload.len() as u32);
    bytes[524..524 + payload.len()].copy_from_slice(&payload);
    bytes
}
fn parse(bytes: &[u8]) -> Result<Option<Manifest>> {
    read(&mut Cursor::new(bytes), bytes.len() as u64)
}
#[test]
fn declared_sets_and_missing_sections_are_distinct() {
    let mut bytes = fixture();
    assert_eq!(parse(&bytes).unwrap(), Some(Manifest::current()));
    bytes[392..400].copy_from_slice(b".legacy\0");
    assert!(parse(&bytes).unwrap().is_none());
    bytes[152..154].copy_from_slice(&0x10bu16.to_le_bytes());
    assert!(parse(&bytes).unwrap().is_none());
}
#[test]
fn hostile_offsets_duplicates_overlap_and_executable_data_are_rejected() {
    for (offset, value) in [
        (60, u32::MAX),
        (408, u32::MAX),
        (412, 0),
        (428, 0x6000_0040),
        (428, 0xc000_0040),
        (520, 4097),
        (520, 0),
    ] {
        let mut bytes = fixture();
        put32(&mut bytes, offset, value);
        assert!(parse(&bytes).is_err(), "offset {offset}");
    }
    let mut bytes = fixture();
    bytes[134..136].copy_from_slice(&2u16.to_le_bytes());
    let section = bytes[392..432].to_vec();
    bytes[432..472].copy_from_slice(&section);
    assert!(parse(&bytes).is_err());
    bytes[432..440].copy_from_slice(b".other\0\0");
    assert!(parse(&bytes).is_err());
    let bytes = fixture();
    for length in 0..1024 {
        assert!(parse(&bytes[..length]).is_err(), "truncation {length}");
    }
}
#[test]
fn unknown_protocol_and_malformed_payload_do_not_become_legacy() {
    let mut bytes = fixture();
    bytes[519] = b'2';
    assert!(parse(&bytes).is_err());
    let mut bytes = fixture();
    bytes[524] = 255;
    assert!(parse(&bytes).is_err());
}
