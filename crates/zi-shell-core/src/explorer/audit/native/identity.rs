//! Resolve only the process user's classes hive; never follow registry links.
use super::*;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{
        GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount,
        GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub(super) fn classes() -> Result<Key> {
    let mut handle = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } != 0,
        "无法确认当前用户，未读取注册入口"
    );
    let token = Token(handle);
    // Aligned storage for TOKEN_USER and its variable-length SID.
    let mut storage = [0usize; 512];
    let mut size = 0;
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                std::mem::size_of_val(&storage) as u32,
                &mut size,
            )
        } != 0
            && size as usize >= std::mem::size_of::<TOKEN_USER>()
            && size as usize <= std::mem::size_of_val(&storage),
        "无法读取当前用户标识，未读取注册入口"
    );
    let sid = unsafe { (*(storage.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    ensure!(unsafe { IsValidSid(sid) } != 0, "当前用户标识无效");
    let count = unsafe { *GetSidSubAuthorityCount(sid) };
    ensure!(count <= 15, "当前用户标识超出范围");
    let authority = unsafe { (*GetSidIdentifierAuthority(sid)).Value }
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    let revision = unsafe { *sid.cast::<u8>() };
    let mut name = format!("S-{revision}-{authority}");
    for index in 0..u32::from(count) {
        let part = unsafe { *GetSidSubAuthority(sid, index) };
        name.push_str(&format!("-{part}"));
    }
    name.push_str("_Classes");
    open(HKEY_USERS, &name)?.ok_or_else(|| anyhow::anyhow!("当前用户的文件类型注册库尚未加载"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_user_classes_anchor_opens_without_following_alias() {
        // Read-only integration test for the real Windows alias that synthetic
        // Software\ZiCode fixtures do not exercise. No SID is logged.
        let _anchor = classes().unwrap();
    }
}
