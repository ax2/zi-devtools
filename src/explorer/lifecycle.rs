//! Shared installation model with a desktop-bound maintenance adapter.
#[cfg(windows)]
pub use zi_shell_core::explorer::lifecycle::store;
pub use zi_shell_core::explorer::lifecycle::{
    Change, Eligible, Entry, Identity, Installation, Ledger, MSI_UPGRADE_CODE, Prepared,
    msi_operation, prepare,
};
#[cfg(windows)]
pub mod installer;

#[cfg(test)]
mod tests {
    #[test]
    fn installer_package_preserves_shared_upgrade_family_identity() {
        assert!(include_str!("../../packaging/windows.wxs").contains(super::MSI_UPGRADE_CODE));
    }
}
