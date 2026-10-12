//! Small versioned declaration shared by the resource producer and reader.
//! A declaration describes supported inputs; it is not publisher authentication.
use crate::file_actions::Target;

pub const MAX_BYTES: usize = 4096;
const HEADER: &str = "ZiDevTools/ShellCapabilities/1\nchoose-files\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    targets: Vec<Target>,
}
impl Manifest {
    pub fn current() -> Self {
        Self {
            targets: Target::ALL.to_vec(),
        }
    }
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut text = HEADER.to_owned();
        for target in &self.targets {
            text.push_str(target.action_id());
            text.push('\n');
        }
        text.into_bytes()
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES || !bytes.ends_with(b"\n") {
            return Err("菜单能力声明过大或不完整".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "菜单能力声明不是UTF-8")?;
        let body = text
            .strip_prefix(HEADER)
            .ok_or("未知菜单能力协议或缺少文件选择入口")?;
        let mut targets = Vec::new();
        for line in body.split_terminator('\n') {
            let target = Target::from_action_id(line).ok_or("未知菜单接收动作")?;
            if targets.contains(&target) {
                return Err("菜单能力声明含重复动作".into());
            }
            targets.push(target);
        }
        // Preserve the producer's order; the byte declaration is part of identity.
        Ok(Self { targets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versioned_bounded_declarations_distinguish_supported_sets() {
        let current = Manifest::current();
        assert_eq!(Manifest::parse(&current.encode()).unwrap(), current);
        let old = Manifest::parse(format!("{HEADER}import:csv\n").as_bytes()).unwrap();
        assert_eq!(old.targets(), &[Target::Csv]);
        assert_ne!(old, current);
        assert!(
            Manifest::parse(HEADER.as_bytes())
                .unwrap()
                .targets()
                .is_empty()
        );
        for invalid in [
            HEADER.replace("/1", "/2"),
            format!("{HEADER}import:csv\nimport:csv\n"),
            format!("{HEADER}import:unknown\n"),
            format!("{HEADER}\n"),
            HEADER.replace('\n', "\r\n"),
            HEADER.trim_end().into(),
            format!("{HEADER}{}\n", "x".repeat(MAX_BYTES)),
        ] {
            assert!(Manifest::parse(invalid.as_bytes()).is_err());
        }
        assert!(Manifest::parse(&[255, 10]).is_err());
    }
}
