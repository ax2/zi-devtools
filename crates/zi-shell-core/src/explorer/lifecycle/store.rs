//! Current-user ledger storage and joint Shell-menu transactions in `joint`.
//! MSI installer integration is pending; legacy ledger-only commits reject menu records.
//! KTM commits the registry update atomically; errors drop/rollback the transaction.
use super::*;
use std::ptr;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND,
        ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{CommitTransaction, CreateTransaction, RollbackTransaction},
    System::Registry::*,
};

const ROOT: &str = r"Software\ZiCode\ZiDevTools\ExplorerInstallations";
const MAX_BYTES: usize = 256 * 1024;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn success(code: u32) -> Result<()> {
    ensure!(
        code == ERROR_SUCCESS,
        "安装记录事务失败（系统错误{code}），未提交更改"
    );
    Ok(())
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}
struct Transaction {
    handle: HANDLE,
    committed: bool,
    user: crate::explorer::audit::native::UserContext,
}
impl Transaction {
    fn begin() -> Result<Self> {
        let user = crate::explorer::audit::native::UserContext::capture()?;
        let handle = unsafe {
            CreateTransaction(
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                0,
                0,
                10000,
                ptr::null(),
            )
        };
        ensure!(
            !handle.is_null() && handle != INVALID_HANDLE_VALUE,
            "本机注册事务不可用，未进行更改"
        );
        Ok(Self {
            handle,
            committed: false,
            user,
        })
    }
    fn commit(mut self) -> Result<()> {
        ensure!(
            unsafe { CommitTransaction(self.handle) } != 0,
            "安装记录提交未确认，请重新检查当前状态"
        );
        self.committed = true;
        Ok(())
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        unsafe {
            if !self.committed {
                RollbackTransaction(self.handle);
            }
            CloseHandle(self.handle);
        }
    }
}

fn open(path: &str, tx: &Transaction, create: bool) -> Result<Option<Key>> {
    open_at(tx.user.user.0, path, tx, create)
}
fn open_at(base: HKEY, path: &str, tx: &Transaction, create: bool) -> Result<Option<Key>> {
    let mut parent = base;
    let mut held = None;
    let parts: Vec<_> = path.split('\\').collect();
    for (index, part) in parts.iter().enumerate() {
        ensure!(!part.is_empty() && !part.contains('\0'), "安装记录路径无效");
        let mut handle = ptr::null_mut();
        let access = if create && index + 1 == parts.len() {
            KEY_READ | KEY_SET_VALUE
        } else {
            KEY_READ
        };
        let code = unsafe {
            RegOpenKeyTransactedW(
                parent,
                wide(part).as_ptr(),
                REG_OPTION_OPEN_LINK,
                access,
                &mut handle,
                tx.handle,
                ptr::null(),
            )
        };
        if matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            if !create {
                return Ok(None);
            }
            success(unsafe {
                RegCreateKeyTransactedW(
                    parent,
                    wide(part).as_ptr(),
                    0,
                    ptr::null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_READ | KEY_WRITE,
                    ptr::null(),
                    &mut handle,
                    ptr::null_mut(),
                    tx.handle,
                    ptr::null(),
                )
            })?;
        } else {
            success(code)?;
        }
        let key = Key(handle);
        let mut kind = 0;
        let code = unsafe {
            RegQueryValueExW(
                handle,
                wide("SymbolicLinkValue").as_ptr(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if code != ERROR_FILE_NOT_FOUND {
            success(code)?;
            ensure!(kind != REG_LINK, "拒绝安装记录路径中的注册表链接");
        }
        parent = handle;
        held = Some(key);
    }
    Ok(held)
}

fn read(root: &str, tx: &Transaction) -> Result<Ledger> {
    let Some(key) = open(root, tx, false)? else {
        return Ok(Ledger::default());
    };
    let mut subkey = [0u16; 256];
    let mut length = subkey.len() as u32;
    ensure!(
        unsafe {
            RegEnumKeyExW(
                key.0,
                0,
                subkey.as_mut_ptr(),
                &mut length,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == ERROR_NO_MORE_ITEMS,
        "安装记录含未知子项，保留现状"
    );
    let mut name = [0u16; 256];
    let mut length = name.len() as u32;
    let mut bytes = vec![0u8; MAX_BYTES];
    let mut size = bytes.len() as u32;
    let mut kind = 0;
    success(unsafe {
        RegEnumValueW(
            key.0,
            0,
            name.as_mut_ptr(),
            &mut length,
            ptr::null(),
            &mut kind,
            bytes.as_mut_ptr(),
            &mut size,
        )
    })?;
    ensure!(
        length as usize <= name.len() && size as usize <= bytes.len(),
        "安装记录大小无效"
    );
    ensure!(
        String::from_utf16(&name[..length as usize])?.eq_ignore_ascii_case("State")
            && kind == REG_SZ,
        "安装记录含未知内容，保留现状"
    );
    length = name.len() as u32;
    ensure!(
        unsafe {
            RegEnumValueW(
                key.0,
                1,
                name.as_mut_ptr(),
                &mut length,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == ERROR_NO_MORE_ITEMS,
        "安装记录含未知值，保留现状"
    );
    let bytes = &bytes[..size as usize];
    ensure!(bytes.len() >= 2 && bytes.len() % 2 == 0, "安装记录编码无效");
    let mut units: Vec<_> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    ensure!(units.pop() == Some(0), "安装记录缺少结束标记");
    let ledger: Ledger = serde_json::from_str(&String::from_utf16(&units)?)?;
    ledger.validate()?;
    Ok(ledger)
}

fn save(root: &str, prepared: &Prepared, commit: bool) -> Result<()> {
    prepared.before.validate()?;
    prepared.after.validate()?;
    let tx = Transaction::begin()?;
    prepared.require_current(&read(root, &tx)?)?;
    write(root, &prepared.after, &tx)?;
    if commit {
        tx.commit()?;
    }
    Ok(())
}
fn write(root: &str, ledger: &Ledger, tx: &Transaction) -> Result<()> {
    ledger.validate()?;
    let bytes: Vec<_> = wide(&serde_json::to_string(ledger)?)
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    ensure!(bytes.len() <= MAX_BYTES, "安装记录超出存储上限");
    let key = open(root, tx, true)?.ok_or_else(|| anyhow::anyhow!("无法创建安装记录"))?;
    success(unsafe {
        RegSetValueExW(
            key.0,
            wide("State").as_ptr(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    })?;
    // Verify through the same transaction before allowing a commit.
    ensure!(&read(root, tx)? == ledger, "事务内复核失败");
    drop(key);
    Ok(())
}

/// Read only. Unknown content, ownership/schema and size errors are explicit.
pub fn load() -> Result<Ledger> {
    let tx = Transaction::begin()?;
    read(ROOT, &tx)
}

/// Persist only a prepared lifecycle transition. Must be called after explicit
/// user approval by the future UI/installer; there is no automatic caller today.
/// This commits the ledger only, not a menu or a Windows Installer transaction.
pub fn commit(prepared: &Prepared) -> Result<()> {
    ensure!(
        !prepared
            .before
            .entries
            .iter()
            .chain(&prepared.after.entries)
            .any(|entry| entry.installation.executable_sha256.is_some()),
        "菜单记录必须通过账本与菜单联合事务维护"
    );
    save(ROOT, prepared, true)
}

pub(in crate::explorer) mod joint;

#[cfg(test)]
mod tests;
