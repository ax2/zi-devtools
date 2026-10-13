//! Small versioned declaration shared by the resource producer and reader.
//! A declaration describes supported inputs; it is not publisher authentication.
use crate::file_actions::Target;

pub const MAX_BYTES: usize = 4096;
const HEADER: &str = "ZiDevTools/ShellCapabilities/1\nchoose-files\n";
const MSI_PREFIX: &str = "ZiDevTools/ShellCapabilities/1|";
pub const MAX_MSI_PROPERTY_BYTES: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    targets: Vec<Target>,
    folder_contexts: bool,
}
impl Manifest {
    pub fn current() -> Self {
        Self {
            targets: Target::ALL.to_vec(),
            folder_contexts: true,
        }
    }
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }
    pub fn supports_folder_contexts(&self) -> bool {
        self.folder_contexts
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut text = HEADER.to_owned();
        if self.folder_contexts {
            text.push_str("choose-folders\n");
        }
        for target in &self.targets {
            text.push_str(target.action_id());
            text.push('\n');
        }
        text.into_bytes()
    }
    /// Compact, versioned representation for the Windows Installer Property
    /// table. The named action order is retained; choose-files is always part
    /// of the protocol header.
    pub fn encode_msi_property(&self) -> Result<String, String> {
        let mut text = MSI_PREFIX.to_owned();
        if self.folder_contexts {
            text.push_str("choose-folders");
        }
        for (index, target) in self.targets.iter().enumerate() {
            if index > 0 || self.folder_contexts {
                text.push(',');
            }
            text.push_str(target.action_id());
        }
        if text.len() > MAX_MSI_PROPERTY_BYTES {
            return Err("MSI菜单能力属性超出Windows Installer长度限制".into());
        }
        Ok(text)
    }
    pub fn parse_msi_property(property: &str) -> Result<Self, String> {
        if property.len() > MAX_MSI_PROPERTY_BYTES || !property.is_ascii() {
            return Err("MSI菜单能力属性过大或不是ASCII".into());
        }
        let body = property
            .strip_prefix(MSI_PREFIX)
            .ok_or("未知MSI菜单能力属性协议")?;
        let mut declaration = HEADER.to_owned();
        let mut folder_contexts = false;
        if !body.is_empty() {
            for action in body.split(',') {
                if action == "choose-folders" {
                    if folder_contexts {
                        return Err("MSI菜单能力属性含重复动作".into());
                    }
                    folder_contexts = true;
                    declaration.push_str("choose-folders\n");
                } else if Target::from_action_id(action).is_some() {
                    declaration.push_str(action);
                    declaration.push('\n');
                } else {
                    return Err("MSI菜单能力属性含未知动作或空项".into());
                }
            }
        }
        Self::parse(declaration.as_bytes())
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
        let mut folder_contexts = false;
        for line in body.split_terminator('\n') {
            if line == "choose-folders" {
                if folder_contexts {
                    return Err("菜单能力声明含重复动作".into());
                }
                folder_contexts = true;
                continue;
            }
            let target = Target::from_action_id(line).ok_or("未知菜单接收动作")?;
            if targets.contains(&target) {
                return Err("菜单能力声明含重复动作".into());
            }
            targets.push(target);
        }
        // Preserve the producer's order; the byte declaration is part of identity.
        Ok(Self {
            targets,
            folder_contexts,
        })
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
        assert!(!old.supports_folder_contexts());
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
