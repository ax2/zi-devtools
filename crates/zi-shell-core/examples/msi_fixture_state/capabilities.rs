//! Only the generic file receiver is implemented by this headless CI fixture.
const BODY: &[u8] = b"ZiDevTools/ShellCapabilities/1\nchoose-files\n";
const fn envelope() -> [u8; BODY.len() + 12] {
    let mut bytes = [0; BODY.len() + 12];
    let header = b"ZICAP001";
    let length = (BODY.len() as u32).to_le_bytes();
    let mut index = 0;
    while index < 8 {
        bytes[index] = header[index];
        index += 1;
    }
    index = 0;
    while index < 4 {
        bytes[index + 8] = length[index];
        index += 1;
    }
    index = 0;
    while index < BODY.len() {
        bytes[index + 12] = BODY[index];
        index += 1;
    }
    bytes
}
#[used]
#[unsafe(link_section = ".zicap")]
pub static DECLARATION: [u8; BODY.len() + 12] = envelope();
