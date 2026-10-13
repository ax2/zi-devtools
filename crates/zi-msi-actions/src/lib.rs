//! Candidate DLL; not wired into the public installer until full acceptance.
#![cfg(windows)]
use anyhow::{Result, ensure};
use std::path::Path;
use windows_sys::Win32::System::ApplicationInstallationAndServicing::*;
use zi_shell_core::explorer::lifecycle::{
    Identity, installer,
    store::{self, recovery},
};

const FAILURE: u32 = 1603;
const MAX_TEXT: usize = 32767;
const ACTIONS: [&str; 3] = ["ZiMenuExecute", "ZiMenuRollback", "ZiMenuCommitReady"];

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn property(session: u32, name: &str) -> Result<String> {
    // A bounded single read. No input or property values are logged.
    let mut buffer = vec![0u16; MAX_TEXT + 1];
    let mut size = buffer.len() as u32;
    ensure!(
        unsafe { MsiGetPropertyW(session, wide(name).as_ptr(), buffer.as_mut_ptr(), &mut size) }
            == 0
            && size as usize <= MAX_TEXT,
        "安装器属性读取失败或超限"
    );
    Ok(String::from_utf16(&buffer[..size as usize])?)
}
fn set(session: u32, name: &str, value: &str) -> Result<()> {
    ensure!(
        value.encode_utf16().count() <= MAX_TEXT && !value.contains('\0'),
        "安装器参数无效"
    );
    ensure!(
        unsafe { MsiSetPropertyW(session, wide(name).as_ptr(), wide(value).as_ptr()) } == 0,
        "安装器动作参数设置失败"
    );
    Ok(())
}
fn mode(session: u32, mode: MSIRUNMODE) -> bool {
    unsafe { MsiGetMode(session, mode) != 0 }
}
fn enabled(value: &str, inherited: bool) -> Result<bool> {
    match value {
        "" => Ok(inherited),
        "0" => Ok(false),
        "1" => Ok(true),
        _ => anyhow::bail!("ZIMENU只接受0或1"),
    }
}
fn valid_id(id: &str) -> Result<()> {
    ensure!(id.len() == 36, "恢复操作标识长度无效");
    let parsed = uuid::Uuid::parse_str(id)?;
    ensure!(
        !parsed.is_nil() && parsed.to_string() == id,
        "恢复操作标识无效"
    );
    Ok(())
}
fn report(session: u32, message: &str) {
    let record = unsafe { MsiCreateRecord(0) };
    if record != 0 {
        unsafe {
            MsiRecordSetStringW(record, 0, wide(message).as_ptr());
            MsiProcessMessage(session, INSTALLMESSAGE_INFO, record);
            MsiCloseHandle(record);
        }
    }
}
fn boundary(session: u32, action: fn(u32) -> Result<()>) -> u32 {
    match std::panic::catch_unwind(|| action(session)) {
        Ok(Ok(())) => 0,
        _ => {
            report(
                session,
                "ZiMenu: maintenance failed; state retained for recovery",
            );
            FAILURE
        }
    }
}
fn prepare(session: u32) -> Result<()> {
    ensure!(
        !mode(session, MSIRUNMODE_SCHEDULED)
            && !mode(session, MSIRUNMODE_ROLLBACK)
            && !mode(session, MSIRUNMODE_COMMIT),
        "准备动作模式错误"
    );
    ensure!(
        property(session, "UpgradeCode")?.eq_ignore_ascii_case(zi_shell_core::msi::UPGRADE_CODE),
        "安装器家族不符"
    );
    let product = property(session, "ProductCode")?;
    ensure!(
        product.len() == 38
            && product.starts_with('{')
            && product.ends_with('}')
            && !uuid::Uuid::parse_str(&product)?.is_nil(),
        "安装器产品标识无效"
    );
    ensure!(
        property(session, "ALLUSERS")?.is_empty(),
        "只支持当前用户安装"
    );
    for action in ACTIONS {
        set(session, action, "")?;
    }
    // The old package in a major upgrade must never remove the new family's alias.
    if !property(session, "UPGRADINGPRODUCTCODE")?.is_empty() {
        report(session, "ZiMenu: old upgrade removal skipped");
        return Ok(());
    }
    let before = store::load()?;
    ensure!(
        before.pending_msi_operation().is_none(),
        "存在未结束的安装维护"
    );
    let current = before
        .entries()
        .iter()
        .find(|e| e.installation.identity == Identity::Msi);
    let selected = enabled(&property(session, "ZIMENU")?, current.is_some())?;
    let remove = property(session, "REMOVE")?;
    ensure!(remove.is_empty() || remove == "ALL", "不支持部分功能移除");
    let intent = if remove == "ALL" || !selected {
        let Some(current) = current else {
            report(session, "ZiMenu: disabled; no changes");
            return Ok(());
        };
        recovery::Intent::Remove {
            expected_generation: current.installation.generation.clone(),
        }
    } else {
        let candidate = installer::msi_candidate(
            Path::new(&property(session, "INSTALLFOLDER")?),
            &property(session, "ProductVersion")?,
            &property(session, "ZiMenuBinarySha256")?,
            &property(session, "ZiMenuCapabilities")?,
        )?;
        recovery::Intent::Install {
            installation: candidate,
            expected_generation: current.map(|e| e.installation.generation.clone()),
            activate: true,
        }
    };
    ensure!(
        property(session, "RollbackDisabled")?.is_empty(),
        "安装维护要求启用回滚"
    );
    let id = uuid::Uuid::new_v4().to_string();
    // Populate the rollback data first. A preparation failure never writes menus.
    for action in ACTIONS {
        set(session, action, &id)?;
    }
    recovery::prepare(&id, intent)?;
    report(session, "ZiMenu: prepared");
    Ok(())
}
fn execute(session: u32) -> Result<()> {
    ensure!(
        mode(session, MSIRUNMODE_SCHEDULED)
            && !mode(session, MSIRUNMODE_ROLLBACK)
            && !mode(session, MSIRUNMODE_COMMIT),
        "执行动作模式错误"
    );
    let id = property(session, "CustomActionData")?;
    valid_id(&id)?;
    recovery::execute_verified(&id)?;
    report(session, "ZiMenu: applied");
    Ok(())
}
fn rollback(session: u32) -> Result<()> {
    ensure!(
        mode(session, MSIRUNMODE_ROLLBACK) && !mode(session, MSIRUNMODE_COMMIT),
        "回滚动作模式错误"
    );
    let id = property(session, "CustomActionData")?;
    valid_id(&id)?;
    recovery::rollback_verified(&id)?;
    report(session, "ZiMenu: reverted");
    Ok(())
}
fn completion_id(session: u32) -> Result<Option<String>> {
    // This immediate action is sequenced AFTER InstallFinalize, never as a
    // commit action: a commit action can still be followed by MSI rollback.
    ensure!(
        !mode(session, MSIRUNMODE_SCHEDULED)
            && !mode(session, MSIRUNMODE_ROLLBACK)
            && !mode(session, MSIRUNMODE_COMMIT),
        "收尾动作模式错误"
    );
    ensure!(
        property(session, "UpgradeCode")?.eq_ignore_ascii_case(zi_shell_core::msi::UPGRADE_CODE)
            && property(session, "ALLUSERS")?.is_empty(),
        "收尾安装器家族或作用域不符"
    );
    let id = property(session, "ZiMenuExecute")?;
    if id.is_empty() {
        return Ok(None);
    }
    valid_id(&id)?;
    Ok(Some(id))
}
fn commit_ready(session: u32) -> Result<()> {
    ensure!(
        mode(session, MSIRUNMODE_COMMIT) && !mode(session, MSIRUNMODE_ROLLBACK),
        "提交准备动作模式错误"
    );
    let id = property(session, "CustomActionData")?;
    valid_id(&id)?;
    recovery::completion::mark_commit_ready(&id)?;
    report(session, "ZiMenu: commit ready; Undo retained");
    Ok(())
}
fn receipt(session: u32) -> Result<()> {
    if let Some(id) = completion_id(session)? {
        if recovery::status(&id)? == recovery::Status::Applied {
            report(session, "ZiMenu: outer transaction pending; Undo retained");
            return Ok(());
        }
        recovery::completion::mark_committed(&id)?;
        report(session, "ZiMenu: committed receipt after InstallFinalize");
    }
    Ok(())
}
fn finalize(session: u32) -> Result<()> {
    let Some(id) = completion_id(session)? else {
        return Ok(());
    };
    if recovery::status(&id)? == recovery::Status::Applied {
        report(session, "ZiMenu: outer transaction pending; Undo retained");
        return Ok(());
    }
    recovery::completion::mark_committed(&id)?;
    recovery::complete(&id)?;
    report(session, "ZiMenu: completed after InstallFinalize");
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuPrepare(session: u32) -> u32 {
    boundary(session, prepare)
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuExecute(session: u32) -> u32 {
    boundary(session, execute)
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuRollback(session: u32) -> u32 {
    boundary(session, rollback)
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuFinalize(session: u32) -> u32 {
    boundary(session, finalize)
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuCommitReceipt(session: u32) -> u32 {
    boundary(session, receipt)
}
#[unsafe(no_mangle)]
pub extern "system" fn ZiMenuCommitReady(session: u32) -> u32 {
    boundary(session, commit_ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_session_is_rejected_by_all_exports() {
        for action in [
            ZiMenuPrepare,
            ZiMenuExecute,
            ZiMenuRollback,
            ZiMenuFinalize,
            ZiMenuCommitReceipt,
            ZiMenuCommitReady,
        ] {
            assert_eq!(action(0), FAILURE);
        }
    }
    #[test]
    fn opt_in_is_strict_and_inherits_only_when_unspecified() {
        for inherited in [false, true] {
            assert_eq!(enabled("", inherited).unwrap(), inherited);
            assert!(!enabled("0", inherited).unwrap());
            assert!(enabled("1", inherited).unwrap());
        }
        for bad in ["yes", "true", "01", " 1", "2"] {
            assert!(enabled(bad, true).is_err());
        }
    }
    #[test]
    fn callback_ids_are_bounded_canonical_and_non_nil() {
        assert!(valid_id(&uuid::Uuid::new_v4().to_string()).is_ok());
        for bad in ["", "{}", "00000000-0000-0000-0000-000000000000", "--force"] {
            assert!(valid_id(bad).is_err());
        }
    }
}
