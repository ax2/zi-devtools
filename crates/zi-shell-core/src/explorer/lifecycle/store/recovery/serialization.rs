//! Serialize journal mutations across desktop and Installer processes.
//! KTM atomicity alone is not a compare-and-swap for journal-only cancellation.
use super::*;
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
};

pub(super) struct Guard(HANDLE);
impl Guard {
    pub(super) fn acquire(scope: &Scope) -> Result<Self> {
        let user = crate::explorer::audit::native::UserContext::capture()?;
        let mut hash = Sha256::new();
        hash.update(user.identity.0.as_bytes());
        hash.update([0]);
        hash.update(scope.journal.to_lowercase().as_bytes());
        let name = format!("Global\\ZiDevTools.ExplorerRecovery.{:x}", hash.finalize());
        let handle = unsafe { CreateMutexW(ptr::null(), 0, wide(&name).as_ptr()) };
        ensure!(!handle.is_null(), "无法取得安装维护锁，请稍后重试");
        let wait = unsafe { WaitForSingleObject(handle, 5000) };
        if !matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            unsafe {
                CloseHandle(handle);
            }
            anyhow::bail!("另一项安装维护正在运行，请稍后重新检查");
        }
        // Abandonment never implies success. The caller rereads and strictly
        // validates protected state after acquiring the recovered lock.
        Ok(Self(handle))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}
