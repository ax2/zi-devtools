//! Explicit Zi DevTools credentials only; never enumerate or import other applications' secrets.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

pub struct Target(String);
impl Target {
    /// Separate namespace, bound to the normalized complete MCP endpoint.
    pub fn mcp_http(endpoint: &str) -> Result<Self> {
        let url = crate::mcp_http::HttpConfig {
            endpoint: endpoint.into(),
        }
        .validate()?;
        Ok(Self(format!(
            "ZiDevTools/v1/mcp-http/{:x}",
            Sha256::digest(url.as_str().as_bytes())
        )))
    }

    pub fn plugin(id: &str, method: &str, endpoint: &str) -> Result<Self> {
        ensure!(
            id.starts_with("plugin:")
                && id.contains('/')
                && id.len() <= 160
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-:/".contains(&b)),
            "插件凭据标识无效"
        );
        ensure!(matches!(method, "GET" | "POST"), "不支持此请求方法");
        let endpoint = crate::plugins::endpoint(endpoint)?;
        let binding = serde_json::to_vec(&(method, endpoint.as_str()))?;
        Ok(Self(format!(
            "ZiDevTools/v1/{id}/{:x}",
            Sha256::digest(binding)
        )))
    }
}

/// Intentionally has neither Debug nor Serialize, and scrubs its owned buffer on drop.
pub struct Secret(Vec<u8>);
impl Secret {
    pub fn new(value: String) -> Result<Self> {
        ensure!(
            !value.trim().is_empty() && value.len() <= 2560 && !value.chars().any(char::is_control),
            "令牌须为 1–2560 字节且不能包含控制字符"
        );
        Ok(Self(value.into_bytes()))
    }
    pub fn expose(&self) -> &str {
        std::str::from_utf8(&self.0)
            .expect("validated UTF-8 credential")
            .trim()
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_FOUND, GetLastError},
        Security::Credentials::*,
    };
    fn name(target: &Target) -> Vec<u16> {
        target.0.encode_utf16().chain(Some(0)).collect()
    }
    fn failure(operation: &str) -> anyhow::Error {
        anyhow::anyhow!("Windows 凭据{operation}失败（错误码 {}）", unsafe {
            GetLastError()
        })
    }
    pub fn save(target: &Target, value: &str) -> Result<()> {
        let mut secret = Secret::new(value.trim().to_owned())?;
        let mut target = name(target);
        let mut username: Vec<u16> = "ZiDevTools".encode_utf16().chain(Some(0)).collect();
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: secret.0.len() as u32,
            CredentialBlob: secret.0.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(failure("保存"));
        }
        Ok(())
    }
    pub fn read(target: &Target) -> Result<Option<Secret>> {
        let target = name(target);
        let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) } == 0 {
            if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(failure("读取"));
        }
        let result = (|| {
            ensure!(!ptr.is_null(), "Windows 未返回凭据");
            let credential = unsafe { &*ptr };
            ensure!(
                credential.CredentialBlobSize > 0
                    && credential.CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
                    && !credential.CredentialBlob.is_null(),
                "Windows 凭据格式无效"
            );
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
            };
            let value = std::str::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("凭据不是有效 UTF-8 文本"))?;
            Secret::new(value.to_owned()).map(Some)
        })();
        // The allocation belongs to the Windows credential API; release it on every result path.
        if !ptr.is_null() {
            unsafe {
                CredFree(ptr.cast());
            }
        }
        result
    }
    pub fn delete(target: &Target) -> Result<()> {
        let target = name(target);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
            && unsafe { GetLastError() } != ERROR_NOT_FOUND
        {
            return Err(failure("删除"));
        }
        Ok(())
    }
}
#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn save(target: &Target, _: &str) -> Result<()> {
        let _ = &target.0;
        anyhow::bail!("当前平台不支持 Windows 凭据管理器")
    }
    pub fn read(_: &Target) -> Result<Option<Secret>> {
        anyhow::bail!("当前平台不支持 Windows 凭据管理器")
    }
    pub fn delete(_: &Target) -> Result<()> {
        anyhow::bail!("当前平台不支持 Windows 凭据管理器")
    }
}
pub use platform::{delete, read, save};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_binding_normalizes_host_and_separates_full_endpoint_and_plugins() {
        let target = Target::mcp_http("https://EXAMPLE.com:443/mcp").unwrap();
        assert_eq!(
            target.0,
            Target::mcp_http("https://example.com/mcp").unwrap().0
        );
        for endpoint in [
            "https://example.com/MCP",
            "https://example.com/other",
            "https://example.com:8443/mcp",
            "https://other.example/mcp",
        ] {
            assert_ne!(target.0, Target::mcp_http(endpoint).unwrap().0);
        }
        assert_ne!(
            target.0,
            Target::plugin("plugin:test/chat", "POST", "https://example.com/mcp")
                .unwrap()
                .0
        );
        for endpoint in [
            "https://example.com/mcp?token=x",
            "https://u:p@example.com/mcp",
            "http://example.com/mcp",
        ] {
            assert!(Target::mcp_http(endpoint).is_err());
        }
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "explicit disposable MCP Windows credential-store smoke test"]
    fn mcp_windows_store_roundtrip_and_delete() {
        let target = Target::mcp_http(&format!(
            "https://example.test/mcp/{}",
            uuid::Uuid::new_v4()
        ))
        .unwrap();
        struct Cleanup(Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        assert!(read(&cleanup.0).unwrap().is_none());
        save(&cleanup.0, "synthetic-mcp-fixture").unwrap();
        assert!(read(&cleanup.0).unwrap().unwrap().expose() == "synthetic-mcp-fixture");
        delete(&cleanup.0).unwrap();
        assert!(read(&cleanup.0).unwrap().is_none());
    }

    #[test]
    fn credential_binding_separates_tools_methods_and_destinations() {
        let target =
            Target::plugin("plugin:test/chat", "POST", "https://example.com/v1/chat").unwrap();
        for (id, method, url) in [
            ("plugin:other/chat", "POST", "https://example.com/v1/chat"),
            ("plugin:test/chat", "GET", "https://example.com/v1/chat"),
            ("plugin:test/chat", "POST", "https://example.org/v1/chat"),
            ("plugin:test/chat", "POST", "https://example.com/v2/chat"),
        ] {
            assert_ne!(target.0, Target::plugin(id, method, url).unwrap().0);
        }
        assert!(
            Target::plugin(
                "plugin:test/chat",
                "POST",
                "https://example.com/?token=secret"
            )
            .is_err()
        );
        assert!(Target::plugin("other-app", "POST", "https://example.com").is_err());
        assert!(Secret::new("test\nvalue".into()).is_err());
        assert!(Secret::new("x".repeat(2561)).is_err());
        assert!(Secret::new("  fixture  ".into()).unwrap().expose() == "fixture");
    }
    #[test]
    #[cfg(windows)]
    #[ignore = "explicit Windows credential-store smoke test with a unique disposable target"]
    fn windows_store_roundtrip_and_delete() {
        let id = format!("plugin:test-{}/fixture", uuid::Uuid::new_v4());
        let target = Target::plugin(&id, "GET", "http://127.0.0.1:1/").unwrap();
        struct Cleanup(Target);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let cleanup = Cleanup(target);
        assert!(read(&cleanup.0).unwrap().is_none());
        save(&cleanup.0, "synthetic-fixture-one").unwrap();
        assert!(read(&cleanup.0).unwrap().unwrap().expose() == "synthetic-fixture-one");
        save(&cleanup.0, "synthetic-fixture-two").unwrap();
        assert!(read(&cleanup.0).unwrap().unwrap().expose() == "synthetic-fixture-two");
        delete(&cleanup.0).unwrap();
        assert!(read(&cleanup.0).unwrap().is_none());
        delete(&cleanup.0).unwrap();
    }
}
