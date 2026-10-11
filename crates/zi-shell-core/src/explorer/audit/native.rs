use super::*;
use std::ptr;
use windows_sys::Win32::{
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND,
        ERROR_SUCCESS, HANDLE,
    },
    System::Registry::*,
};
mod identity;
#[cfg(test)]
pub(in crate::explorer) use identity::tests::{with_ci_user, with_impersonated_user};
pub(in crate::explorer) use identity::{UserContext, UserIdentity};

const MAX_KEYS: usize = 128;
const MAX_VALUES: usize = 1024;
const MAX_BYTES: usize = 1024 * 1024;

pub(in crate::explorer) struct Key(pub(in crate::explorer) HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn success(status: u32) -> Result<()> {
    ensure!(
        status != ERROR_MORE_DATA,
        "注册内容超出检查上限，请保留现状；未改动注册内容"
    );
    ensure!(
        status == ERROR_SUCCESS,
        "注册状态读取失败（系统错误{status}）；未改动注册内容"
    );
    Ok(())
}

fn open(base: HKEY, path: &str) -> Result<Option<Key>> {
    open_tx(base, path, None)
}
fn open_tx(base: HKEY, path: &str, transaction: Option<HANDLE>) -> Result<Option<Key>> {
    let mut parent = base;
    let mut held = None;
    for part in path.split('\\') {
        ensure!(!part.is_empty() && !part.contains('\0'), "注册路径无效");
        let mut handle = ptr::null_mut();
        let status = unsafe {
            if let Some(transaction) = transaction {
                RegOpenKeyTransactedW(
                    parent,
                    wide(part).as_ptr(),
                    REG_OPTION_OPEN_LINK,
                    KEY_READ,
                    &mut handle,
                    transaction,
                    ptr::null(),
                )
            } else {
                RegOpenKeyExW(
                    parent,
                    wide(part).as_ptr(),
                    REG_OPTION_OPEN_LINK,
                    KEY_READ,
                    &mut handle,
                )
            }
        };
        if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return Ok(None);
        }
        success(status)?;
        let key = Key(handle);
        let mut kind = 0;
        let status = unsafe {
            RegQueryValueExW(
                handle,
                wide("SymbolicLinkValue").as_ptr(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status != ERROR_FILE_NOT_FOUND {
            success(status)?;
            ensure!(kind != REG_LINK, "拒绝跟随注册表链接；未改动注册内容");
        }
        parent = handle;
        held = Some(key);
    }
    Ok(held)
}

fn scan(
    key: &Key,
    path: &str,
    depth: usize,
    result: &mut Snapshot,
    bytes: &mut usize,
    transaction: Option<HANDLE>,
) -> Result<()> {
    ensure!(
        depth <= 8 && result.keys.len() < MAX_KEYS,
        "注册入口树超出检查范围，未截断为成功"
    );
    ensure!(
        result.keys.insert(normalize(path)),
        "注册内容正在变化，请重新检查"
    );
    let mut buffer = vec![0u8; 128 * 1024];
    for index in 0..=MAX_VALUES as u32 {
        let mut name = [0u16; 1024];
        let mut length = name.len() as u32;
        let mut size = buffer.len() as u32;
        let mut kind = 0;
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                ptr::null(),
                &mut kind,
                buffer.as_mut_ptr(),
                &mut size,
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        success(status)?;
        ensure!(
            result.values.len() < MAX_VALUES
                && size as usize <= buffer.len()
                && length as usize <= name.len(),
            "注册值超出检查范围"
        );
        *bytes = bytes
            .checked_add(size as usize)
            .ok_or_else(|| anyhow::anyhow!("注册数据大小无效"))?;
        ensure!(
            *bytes <= MAX_BYTES,
            "注册数据超过1 MiB检查范围，未截断为成功"
        );
        let name = String::from_utf16(&name[..length as usize])
            .map_err(|_| anyhow::anyhow!("注册值名称编码无效"))?;
        ensure!(
            result
                .values
                .insert(
                    (normalize(path), normalize(&name)),
                    (kind, buffer[..size as usize].to_vec())
                )
                .is_none(),
            "注册内容正在变化，请重试"
        );
    }
    for index in 0..=MAX_KEYS as u32 {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        success(status)?;
        ensure!(length as usize <= name.len(), "注册键名称超出范围");
        let name = String::from_utf16(&name[..length as usize])
            .map_err(|_| anyhow::anyhow!("注册键名称编码无效"))?;
        let child = open_tx(key.0, &name, transaction)?
            .ok_or_else(|| anyhow::anyhow!("注册内容正在变化，请重试"))?;
        scan(
            &child,
            &format!("{path}\\{name}"),
            depth + 1,
            result,
            bytes,
            transaction,
        )?;
    }
    Ok(())
}

pub(super) fn read(roots: &[&str]) -> Result<Snapshot> {
    read_tx(roots, None)
}
#[cfg(test)]
pub(in crate::explorer) fn classes() -> Result<Key> {
    Ok(UserContext::capture()?.classes)
}
pub(in crate::explorer) fn read_tx(
    roots: &[&str],
    transaction: Option<HANDLE>,
) -> Result<Snapshot> {
    read_in(roots, transaction, &UserContext::capture()?)
}
pub(in crate::explorer) fn read_in(
    roots: &[&str],
    transaction: Option<HANDLE>,
    context: &UserContext,
) -> Result<Snapshot> {
    let mut result = Snapshot::default();
    let mut bytes = 0;
    // Windows makes HKCU\Software\Classes a link to the current user's
    // separate classes hive. Open that known hive directly, never an arbitrary
    // link target or the merged HKCR view (which also includes machine data).
    for root in roots {
        let key = if let Some(relative) = root.strip_prefix(r"Software\Classes\") {
            open_tx(context.classes.0, relative, transaction)?
        } else {
            open_tx(context.user.0, root, transaction)?
        };
        if let Some(key) = key {
            scan(&key, root, 0, &mut result, &mut bytes, transaction)?;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(String);
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(
                self.0
                    .starts_with(r"Software\ZiCode\ZiDevTools\Tests\ExplorerAudit-")
            );
            unsafe {
                RegDeleteTreeW(HKEY_CURRENT_USER, wide(&self.0).as_ptr());
            }
        }
    }
    #[test]
    fn real_registry_snapshot_is_read_only_and_fixture_is_removed() {
        let fixture = Fixture(format!(
            r"Software\ZiCode\ZiDevTools\Tests\ExplorerAudit-{}",
            uuid::Uuid::new_v4()
        ));
        let roots = [
            format!("{}\\parent", fixture.0),
            format!("{}\\actions", fixture.0),
        ];
        let refs = [roots[0].as_str(), roots[1].as_str()];
        let dir = std::env::current_dir().unwrap();
        let mut plan =
            super::super::super::plan(&dir.join("ZiDevTools.exe"), &dir.join("config.yml"))
                .unwrap();
        for value in &mut plan.values {
            for (original, replacement) in ROOTS.iter().zip(&roots) {
                if value.key.starts_with(original) {
                    value.key = value.key.replacen(original, replacement, 1);
                    break;
                }
            }
            let mut handle = ptr::null_mut();
            unsafe {
                success(RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(&value.key).as_ptr(),
                    0,
                    ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_WRITE,
                    ptr::null(),
                    &mut handle,
                    ptr::null_mut(),
                ))
                .unwrap();
                let key = Key(handle);
                let bytes = string_bytes(&value.value);
                success(RegSetValueExW(
                    key.0,
                    wide(&value.name).as_ptr(),
                    0,
                    REG_SZ,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                ))
                .unwrap();
            }
        }
        let first = read(&refs).unwrap();
        let second = read(&refs).unwrap();
        let stable = consistent(first, second).unwrap();
        assert_eq!(
            classify(&stable, &expected(&plan, &refs).unwrap(), &refs).state,
            State::Matches
        );
        assert_eq!(read(&refs).unwrap(), stable);
        let key = open(HKEY_CURRENT_USER, &roots[0]).unwrap().unwrap();
        let too_large = vec![0u8; 128 * 1024 + 1];
        unsafe {
            let mut writable = ptr::null_mut();
            success(RegOpenKeyExW(
                key.0,
                wide("").as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut writable,
            ))
            .unwrap();
            let writable = Key(writable);
            success(RegSetValueExW(
                writable.0,
                wide("OversizedFixture").as_ptr(),
                0,
                REG_BINARY,
                too_large.as_ptr(),
                too_large.len() as u32,
            ))
            .unwrap();
        }
        assert!(
            read(&refs).is_err(),
            "oversized data must not be reported as a complete snapshot"
        );
        drop(key);
        unsafe {
            success(RegDeleteTreeW(HKEY_CURRENT_USER, wide(&fixture.0).as_ptr())).unwrap();
        }
        assert!(open(HKEY_CURRENT_USER, &fixture.0).unwrap().is_none());
    }
}
