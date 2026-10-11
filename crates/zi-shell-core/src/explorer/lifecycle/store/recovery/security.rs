//! Exact protected owner/DACL; never repair or adopt unexpected permissions.
use super::*;
use windows_sys::Win32::{
    Foundation::{ERROR_INSUFFICIENT_BUFFER, LocalFree},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, IsValidAcl,
        IsValidSecurityDescriptor, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    },
};

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
impl Descriptor {
    fn expected(tx: &Transaction) -> Result<Self> {
        // Not logged: identity is used only to construct a native descriptor.
        let sid = &tx.user.identity.0;
        let sddl = wide(&format!("O:{sid}D:P(A;;KA;;;{sid})(A;;KA;;;SY)"));
        let mut sd = ptr::null_mut();
        ensure!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut sd,
                    ptr::null_mut(),
                )
            } != 0,
            "无法创建恢复日志保护描述符"
        );
        Ok(Self(sd))
    }
}
fn acl(sd: PSECURITY_DESCRIPTOR) -> Result<Vec<u8>> {
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    ensure!(
        unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted) } != 0
            && present != 0
            && !acl.is_null()
            && unsafe { IsValidAcl(acl) } != 0,
        "恢复日志DACL无效"
    );
    let len = unsafe { (*acl).AclSize } as usize;
    ensure!(len <= 65536, "恢复日志DACL超限");
    Ok(unsafe { std::slice::from_raw_parts(acl.cast(), len) }.to_vec())
}
fn validate(key: &Key, expected: &Descriptor) -> Result<()> {
    let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut size = 0;
    ensure!(
        unsafe { RegGetKeySecurity(key.0, info, ptr::null_mut(), &mut size) }
            == ERROR_INSUFFICIENT_BUFFER
            && size > 0
            && size <= 65536,
        "无法核验恢复日志权限"
    );
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    let sd = buffer.as_mut_ptr().cast();
    success(unsafe { RegGetKeySecurity(key.0, info, sd, &mut size) })?;
    ensure!(
        unsafe { IsValidSecurityDescriptor(sd) } != 0,
        "恢复日志保护描述符无效"
    );
    let mut owner = ptr::null_mut();
    let mut wanted = ptr::null_mut();
    let mut defaulted = 0;
    ensure!(
        unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) } != 0
            && unsafe { GetSecurityDescriptorOwner(expected.0, &mut wanted, &mut defaulted) } != 0
            && !owner.is_null()
            && !wanted.is_null()
            && unsafe { EqualSid(owner, wanted) } != 0,
        "恢复日志所有者不符，保留现状"
    );
    let mut control = 0;
    let mut revision = 0;
    ensure!(
        unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } != 0
            && control & SE_DACL_PROTECTED != 0
            && acl(sd)? == acl(expected.0)?,
        "恢复日志权限不符，保留现状"
    );
    Ok(())
}

pub(super) fn open_protected(path: &str, tx: &Transaction, create: bool) -> Result<Option<Key>> {
    let (parent, leaf) = path
        .rsplit_once('\\')
        .ok_or_else(|| anyhow::anyhow!("恢复日志范围无效"))?;
    ensure!(!leaf.is_empty() && !leaf.contains('\0'), "恢复日志名称无效");
    let Some(parent) = open(parent, tx, create)? else {
        return Ok(None);
    };
    open_child(parent.0, leaf, tx, create)
}
pub(super) fn open_child(
    parent: HKEY,
    leaf: &str,
    tx: &Transaction,
    create: bool,
) -> Result<Option<Key>> {
    ensure!(
        !leaf.is_empty() && !leaf.contains(['\\', '\0']),
        "恢复日志名称无效"
    );
    let descriptor = Descriptor::expected(tx)?;
    let mut handle = ptr::null_mut();
    let code = unsafe {
        RegOpenKeyTransactedW(
            parent,
            wide(leaf).as_ptr(),
            REG_OPTION_OPEN_LINK,
            KEY_READ | KEY_WRITE,
            &mut handle,
            tx.handle,
            ptr::null(),
        )
    };
    if matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        if !create {
            return Ok(None);
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        success(unsafe {
            RegCreateKeyTransactedW(
                parent,
                wide(leaf).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                &attributes,
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
            key.0,
            wide("SymbolicLinkValue").as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if code != ERROR_FILE_NOT_FOUND {
        success(code)?;
        ensure!(kind != REG_LINK, "拒绝恢复日志注册链接");
    }
    validate(&key, &descriptor)?;
    Ok(Some(key))
}
