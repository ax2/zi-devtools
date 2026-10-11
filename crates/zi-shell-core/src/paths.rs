//! Stable layout shared with the desktop and the headless installer adapter.
use std::path::{Path, PathBuf};

pub fn default_config_path(home: &Path) -> PathBuf {
    home.join(".zi-devtools").join("services.yml")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_layout_is_independent_of_the_working_directory() {
        let home = Path::new(r"C:\Users\中文 用户");
        assert_eq!(
            default_config_path(home),
            home.join(".zi-devtools/services.yml")
        );
    }
}
