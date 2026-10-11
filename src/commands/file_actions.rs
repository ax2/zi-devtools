//! Desktop catalog checks for shared file receivers.
pub use zi_shell_core::file_actions::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn registered_receivers_are_unique_implemented_and_bounded() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/tools.json")).unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for target in Target::ALL {
            assert!(ids.insert(target.action_id()));
            assert_eq!(Target::from_action_id(target.action_id()), Some(target));
            assert!(
                catalog["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["id"] == target.tool_id() && tool["status"] == "implemented")
            );
            assert!(target.validate_selection(&[]).is_err());
            assert!(
                target
                    .validate_selection(&vec![PathBuf::from("file"); target.max_files()])
                    .is_ok()
            );
            assert!(
                target
                    .validate_selection(&vec![PathBuf::from("file"); target.max_files() + 1])
                    .is_err()
            );
        }
        assert!(Target::from_action_id("import:unknown").is_none());
        assert!(Target::from_action_id("IMPORT:CSV").is_none());
    }
}
