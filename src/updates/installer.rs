//! MSI execution adapter; identity checks live in the headless Shell core.
use anyhow::{Result, ensure};
use std::path::Path;
use windows_sys::Win32::System::ApplicationInstallationAndServicing::*;
pub use zi_shell_core::msi::*;
fn text(value: &str) -> Result<Vec<u16>> {
    ensure!(
        value.encode_utf16().count() < 32768 && !value.contains('\0'),
        "安装器字符串无效"
    );
    Ok(value.encode_utf16().chain([0]).collect())
}
/// Called only after authenticated preparation and exact process exit.
/// Windows Installer owns replacement and rollback; no manual EXE copying.
pub(super) fn install(package: &Path, directory: &Path, visible: bool) -> Result<u32> {
    let package = text(&installer_path(package)?)?;
    let directory = installer_path(directory)?;
    ensure!(
        !directory.chars().any(|c| c.is_control() || c == '"'),
        "安装路径包含保留字符"
    );
    let properties = text(&format!(
        "INSTALLFOLDER=\"{directory}\" REBOOT=ReallySuppress MSIRESTARTMANAGERCONTROL=Disable"
    ))?;
    let old_ui = unsafe {
        MsiSetInternalUI(
            if visible {
                INSTALLUILEVEL_BASIC
            } else {
                INSTALLUILEVEL_NONE
            },
            std::ptr::null_mut(),
        )
    };
    struct Ui(i32);
    impl Drop for Ui {
        fn drop(&mut self) {
            unsafe {
                MsiSetInternalUI(self.0, std::ptr::null_mut());
            }
        }
    }
    let _ui = Ui(old_ui);
    Ok(unsafe { MsiInstallProductW(package.as_ptr(), properties.as_ptr()) })
}
