//! Pure grouping shared by the filesystem tool and the Studio report guest.
use std::collections::BTreeMap;

pub type Buckets<Path> = BTreeMap<(u64, [u8; 32]), Vec<Path>>;

pub struct Group<Path> {
    pub size: u64,
    pub digest: [u8; 32],
    pub paths: Vec<Path>,
}

impl<Path> Group<Path> {
    pub fn redundant_bytes(&self) -> u128 {
        u128::from(self.size) * self.paths.len().saturating_sub(1) as u128
    }

    pub fn sha256(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = String::with_capacity(64);
        for byte in self.digest {
            text.push(char::from(HEX[(byte >> 4) as usize]));
            text.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
        text
    }
}

pub fn rank<Path: Ord>(buckets: Buckets<Path>) -> Vec<Group<Path>> {
    let mut groups: Vec<_> = buckets
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|((size, digest), mut paths)| {
            paths.sort_unstable();
            Group {
                size,
                digest,
                paths,
            }
        })
        .collect();
    groups.sort_by(|left, right| {
        right
            .redundant_bytes()
            .cmp(&left.redundant_bytes())
            .then_with(|| left.digest.cmp(&right.digest))
    });
    groups
}
