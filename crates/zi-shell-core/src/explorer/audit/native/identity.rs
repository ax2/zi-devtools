//! Capture effective user anchors once; never use the cached predefined HKCU.
use super::*;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_NO_TOKEN, GetLastError, HANDLE},
    Security::{
        GetSidIdentifierAuthority, GetSidSubAuthority, GetSidSubAuthorityCount,
        GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn effective_token() -> Result<(Token, bool)> {
    let mut handle = ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } != 0 {
        return Ok((Token(handle), true));
    }
    // Access denied/anonymous/other failures must not silently use another user.
    ensure!(
        unsafe { GetLastError() } == ERROR_NO_TOKEN,
        "无法确认模拟用户，未读取注册入口"
    );
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } != 0,
        "无法确认当前用户，未读取注册入口"
    );
    Ok((Token(handle), false))
}

// Deliberately no Debug/Serialize: account identifiers stay out of reports.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::explorer) struct UserIdentity(pub(in crate::explorer) String);

pub(in crate::explorer) struct UserContext {
    pub(in crate::explorer) user: Key,
    pub(in crate::explorer) classes: Key,
    pub(in crate::explorer) identity: UserIdentity,
}
impl UserContext {
    pub(in crate::explorer) fn capture() -> Result<Self> {
        let (token, _) = effective_token()?;
        let name = token_sid(&token)?;
        let user =
            open(HKEY_USERS, &name)?.ok_or_else(|| anyhow::anyhow!("当前用户注册库尚未加载"))?;
        let classes = open(HKEY_USERS, &format!("{name}_Classes"))?
            .ok_or_else(|| anyhow::anyhow!("当前用户的文件类型注册库尚未加载"))?;
        Ok(Self {
            user,
            classes,
            identity: UserIdentity(name),
        })
    }
    pub(in crate::explorer) fn require_identity(&self, expected: &UserIdentity) -> Result<()> {
        ensure!(self.identity == *expected, "操作用户已变化，请重新预览");
        Ok(())
    }
}

fn token_sid(token: &Token) -> Result<String> {
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
    Ok(name)
}

#[cfg(test)]
pub(in crate::explorer) mod tests {
    use super::*;
    use windows_sys::Win32::Security::{
        DuplicateToken, ImpersonateAnonymousToken, RevertToSelf, SecurityImpersonation,
        TOKEN_DUPLICATE,
    };
    use windows_sys::Win32::System::Threading::SetThreadToken;

    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            // These tests run on fresh dedicated threads with no prior token.
            assert_ne!(unsafe { RevertToSelf() }, 0);
        }
    }

    pub(in crate::explorer) fn with_impersonated_user(f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(move || {
            let (before, uses_thread) = effective_token().unwrap();
            assert!(!uses_thread);
            let mut process = ptr::null_mut();
            assert_ne!(
                unsafe {
                    OpenProcessToken(
                        GetCurrentProcess(),
                        TOKEN_QUERY | TOKEN_DUPLICATE,
                        &mut process,
                    )
                },
                0
            );
            let process = Token(process);
            let mut duplicate = ptr::null_mut();
            assert_ne!(
                unsafe { DuplicateToken(process.0, SecurityImpersonation, &mut duplicate) },
                0
            );
            let duplicate = Token(duplicate);
            // Exercise the native HKCU cache before installing the thread token.
            let _cached = open(HKEY_CURRENT_USER, "Software").unwrap().unwrap();
            assert_ne!(unsafe { SetThreadToken(ptr::null(), duplicate.0) }, 0);
            let revert = Revert;
            let (effective, uses_thread) = effective_token().unwrap();
            assert!(uses_thread);
            assert!(token_sid(&effective).unwrap() == token_sid(&before).unwrap());
            f();
            drop(revert);
            assert!(!effective_token().unwrap().1);
        })
        .join()
        .unwrap();
    }

    // Only an explicitly invoked disposable CI test supplies these values.
    pub(in crate::explorer) fn with_ci_user<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        std::thread::spawn(move || {
            use windows_sys::Win32::Security::{
                ImpersonateLoggedOnUser, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT,
                LogonUserW,
            };
            let before = UserContext::capture().unwrap();
            let _cached = open(HKEY_CURRENT_USER, "Software").unwrap().unwrap();
            let name = wide(&std::env::var("ZI_TEST_USER").expect("disposable CI user required"));
            let mut password = wide(
                &std::env::var("ZI_TEST_PASSWORD").expect("disposable CI credential required"),
            );
            let mut token = ptr::null_mut();
            let logged_on = unsafe {
                LogonUserW(
                    name.as_ptr(),
                    wide(".").as_ptr(),
                    password.as_ptr(),
                    LOGON32_LOGON_INTERACTIVE,
                    LOGON32_PROVIDER_DEFAULT,
                    &mut token,
                )
            };
            password.fill(0);
            assert_ne!(logged_on, 0, "disposable CI logon failed");
            let token = Token(token);
            assert_ne!(unsafe { ImpersonateLoggedOnUser(token.0) }, 0);
            let revert = Revert;
            let actual = UserContext::capture().unwrap();
            assert!(effective_token().unwrap().1);
            assert!(
                actual.identity != before.identity,
                "CI must supply a distinct user"
            );
            let result = f();
            drop(revert);
            assert!(UserContext::capture().unwrap().identity == before.identity);
            result
        })
        .join()
        .unwrap()
    }
    #[test]
    fn current_user_classes_anchor_opens_without_following_alias() {
        // Read-only integration test for the real Windows alias that synthetic
        // Software\ZiCode fixtures do not exercise. No SID is logged.
        let _context = UserContext::capture().unwrap();
    }
    #[test]
    fn real_thread_token_is_preferred_and_anchors_open_after_hkcu_was_cached() {
        with_impersonated_user(|| {
            let context = UserContext::capture().unwrap();
            context.require_identity(&context.identity).unwrap();
            assert!(
                context
                    .require_identity(&UserIdentity("different-fixture-user".into()))
                    .is_err()
            );
            assert!(open(context.user.0, "Software").unwrap().is_some());
        });
    }
    #[test]
    fn anonymous_thread_never_uses_process_user_or_its_registry() {
        std::thread::spawn(|| {
            assert!(!effective_token().unwrap().1);
            assert_ne!(unsafe { ImpersonateAnonymousToken(GetCurrentThread()) }, 0);
            let revert = Revert;
            // Some Windows configurations allow querying the anonymous token.
            // Either outcome must avoid falling back to the process identity.
            if let Ok((_, uses_thread)) = effective_token() {
                assert!(uses_thread);
            }
            assert!(UserContext::capture().is_err());
            drop(revert);
            assert!(!effective_token().unwrap().1);
        })
        .join()
        .unwrap();
    }
}
