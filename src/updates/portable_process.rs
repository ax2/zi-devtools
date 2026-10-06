//! Pin the exact parent process lifetime. Never terminate a running process.
use anyhow::{Result, ensure};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};
fn stamp(handle: HANDLE) -> Result<u64> {
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = created;
    let mut kernel = created;
    let mut user = created;
    ensure!(
        unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) } != 0,
        "无法确认主程序身份"
    );
    Ok((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
}
pub fn own_creation_time() -> Result<u64> {
    stamp(unsafe { GetCurrentProcess() })
}
/// Existing MSI does not persist InstallLocation. Conservatively refuse direct
/// EXE replacement while this user's product marker is installed.
pub fn reject_installer_marker() -> Result<()> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let path: Vec<u16> = "Software\\ZiCode\\ZiDevTools\0".encode_utf16().collect();
    let name: Vec<u16> = "Installed\0".encode_utf16().collect();
    let mut installed = 0u32;
    let mut length = 4u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut installed as *mut u32).cast(),
            &mut length,
        )
    };
    ensure!(
        matches!(status, 0 | 2 | 3),
        "无法确认MSI所有权，拒绝便携替换"
    );
    ensure!(
        status != 0 || installed == 0,
        "当前用户已安装MSI版，请通过Windows Installer升级；便携替换暂时禁用"
    );
    Ok(())
}
pub fn wait_parent(pid: u32, expected: u64) -> Result<()> {
    ensure!(pid > 0 && expected > 0, "主程序标识无效");
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        // ERROR_INVALID_PARAMETER: parent already exited before helper opened it.
        ensure!(
            error.raw_os_error() == Some(87),
            "无法等待主程序退出：{error}"
        );
        return Ok(());
    }
    struct Owned(HANDLE);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let handle = Owned(handle);
    ensure!(stamp(handle.0)? == expected, "进程标识已被复用，拒绝操作");
    match unsafe { WaitForSingleObject(handle.0, 180000) } {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => anyhow::bail!("主程序三分钟内未退出，更新取消；没有中断工作"),
        _ => anyhow::bail!("等待主程序失败，更新取消"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_parent_identity_is_rejected_without_waiting_or_killing() {
        let now = own_creation_time().unwrap();
        assert!(wait_parent(std::process::id(), now + 1).is_err());
        assert_eq!(own_creation_time().unwrap(), now);
        assert!(wait_parent(0, now).is_err());
    }
    #[test]
    fn actual_owned_child_exit_is_waited_for() {
        use std::os::windows::process::CommandExt;
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Milliseconds 300",
            ])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id()) };
        assert!(!handle.is_null());
        let created = stamp(handle).unwrap();
        unsafe {
            CloseHandle(handle);
        }
        wait_parent(child.id(), created).unwrap();
        assert!(child.wait().unwrap().success());
    }
}
