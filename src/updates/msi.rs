//! Publisher-authenticated MSI upgrade. Windows Installer performs the transaction.
use super::{delta_files, installer, portable as io, signed};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const NAMES: [&str; 2] = ["ZiDevTools.exe", "ZiDevToolsMcp.exe"];
const MAX_PLAN: usize = 2 * signed::MAX_MANIFEST + 128 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: u32,
    program_version: String,
    product_code: String,
    installed_version: String,
    directory: PathBuf,
    old_hashes: [String; 2],
    helper_hash: String,
    manifest: String,
    signature: String,
}
#[derive(Clone)]
pub struct Prepared {
    pub stage: PathBuf,
    pub directory: PathBuf,
    pub installed_version: String,
    pub version: String,
    pub bytes: u64,
    pub changes: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub state: String,
    pub status: Option<u32>,
    pub installed_version: String,
    pub version: String,
    pub message: String,
}
fn base(create: bool) -> Result<PathBuf> {
    let local = dirs::data_local_dir().context("本机用户目录不可用")?;
    let local = io::root(&local)?;
    let mut current = local;
    for name in ["ZiDevTools", "updates"] {
        current.push(name);
        if fs::symlink_metadata(&current).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            if create {
                fs::create_dir(&current)?;
            } else {
                continue;
            }
        }
        io::plain(&current, true)?;
    }
    if current.exists() {
        io::root(&current)
    } else {
        Ok(current)
    }
}
fn stage_at(path: &Path, base: &Path) -> Result<PathBuf> {
    io::plain(path, true)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("MSI暂存名无效")?;
    let uuid = name.strip_prefix("msi-").context("不是MSI升级目录")?;
    ensure!(uuid::Uuid::parse_str(uuid).is_ok(), "MSI暂存标识无效");
    ensure!(
        io::root(path.parent().context("MSI暂存位置无效")?)? == io::root(base)?,
        "MSI暂存目录越界"
    );
    Ok(io::root(base)?.join(name))
}
fn plan(stage: &Path) -> Result<Plan> {
    let p: Plan = serde_json::from_slice(&io::read(&stage.join("plan.json"), MAX_PLAN)?)?;
    ensure!(
        p.schema == 1 && p.directory.is_absolute(),
        "MSI计划协议无效"
    );
    semver::Version::parse(&p.program_version)?;
    installer::numeric_version(&p.installed_version)?;
    uuid::Uuid::parse_str(&p.product_code)?;
    ensure!(
        p.old_hashes
            .iter()
            .chain([&p.helper_hash])
            .all(|s| s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
        "MSI基线摘要无效"
    );
    Ok(p)
}
pub fn current_installation() -> Result<installer::Installed> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let directory = io::root(exe.parent().context("当前程序位置无效")?)?;
    let installed = installer::owner(&directory)?
        .context("当前是便携版，请使用便携升级入口；MSI升级只用于已有安装版")?;
    ensure!(
        installed.executable.canonicalize()? == exe,
        "当前EXE不是安装器主组件"
    );
    Ok(installed)
}
fn baseline(p: &Plan) -> Result<()> {
    let directory = io::root(&p.directory)?;
    ensure!(directory == p.directory, "安装目录的物理位置改变");
    let installed = installer::owner(&directory)?.context("原安装产品已不存在")?;
    ensure!(
        installed.product_code == p.product_code && installed.version == p.installed_version,
        "安装产品已变化，请重新准备"
    );
    for (i, name) in NAMES.iter().enumerate() {
        ensure!(
            io::hash_file(&directory.join(name))? == p.old_hashes[i],
            "已安装程序与准备时不同"
        );
    }
    Ok(())
}
fn validate_package(
    stage: &Path,
    p: &Plan,
    a: &signed::Authenticated,
) -> Result<installer::Package> {
    let version = installer::numeric_version(&a.manifest.version)?;
    ensure!(
        version
            .cmp_precedence(&semver::Version::parse(&p.program_version)?)
            .is_gt()
            && version > installer::numeric_version(&p.installed_version)?,
        "MSI升级不能同版替换或降级"
    );
    let name = format!("ZiDevTools-{version}-windows-x64.msi");
    let bytes = io::read(&stage.join("package.msi"), super::delta::MAX_FILE)?;
    let expected = signed::expected_file(a, &version, &name, bytes.len() as u64)?;
    ensure!(io::digest(&bytes) == expected.sha256, "MSI大小或摘要不匹配");
    let package = installer::inspect(&stage.join("package.msi"))?;
    ensure!(
        package.version == a.manifest.version
            && package.product_code != p.product_code
            && package.transactional_upgrade
            && package.records_install_location,
        "MSI身份或事务配置不符合升级要求"
    );
    Ok(package)
}
fn verified_target(
    p: &Plan,
    package: &installer::Package,
    a: &signed::Authenticated,
) -> Result<()> {
    let installed = installer::owner(&p.directory)?.context("升级后产品注册缺失")?;
    ensure!(
        installed.product_code == package.product_code && installed.version == a.manifest.version,
        "升级后产品身份不符"
    );
    let version = installer::numeric_version(&a.manifest.version)?;
    for (i, name) in NAMES.iter().enumerate() {
        let bytes = io::read(&p.directory.join(name), super::delta::MAX_FILE)?;
        let asset = format!(
            "{}-{version}-windows-x64.exe",
            if i == 0 {
                "ZiDevTools"
            } else {
                "ZiDevToolsMcp"
            }
        );
        let expected = signed::expected_file(a, &version, &asset, bytes.len() as u64)?;
        ensure!(
            io::digest(&bytes) == expected.sha256,
            "升级后的程序大小或摘要不符"
        );
    }
    Ok(())
}
fn save_receipt(stage: &Path, receipt: &Receipt) -> Result<()> {
    delta_files::save_new(
        &stage.join(format!("receipt-{}.json", receipt.state)),
        &serde_json::to_vec_pretty(receipt)?,
        &AtomicBool::new(false),
    )?;
    Ok(())
}
fn record(
    p: &Plan,
    a: &signed::Authenticated,
    state: &str,
    status: Option<u32>,
    message: String,
) -> Receipt {
    Receipt {
        state: state.into(),
        status,
        installed_version: p.installed_version.clone(),
        version: a.manifest.version.clone(),
        message,
    }
}
struct Incomplete(PathBuf, bool);
impl Drop for Incomplete {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        // Only our three known newly-created files; no recursive cleanup.
        for name in ["package.msi", "helper.exe", "plan.json"] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}
#[allow(clippy::too_many_arguments)]
fn prepare_checked(
    base: &Path,
    installed: &installer::Installed,
    program_version: &str,
    raw: &[u8],
    signature: &[u8],
    package: &[u8],
    helper: &[u8],
    cancel: &AtomicBool,
    a: &signed::Authenticated,
) -> Result<Prepared> {
    ensure!(!cancel.load(Ordering::Relaxed), "MSI准备已取消");
    let directory = io::root(installed.executable.parent().context("安装目录缺失")?)?;
    let stage = io::root(base)?.join(format!("msi-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage)?;
    let mut guard = Incomplete(stage.clone(), false);
    let p = Plan {
        schema: 1,
        program_version: program_version.into(),
        product_code: installed.product_code.clone(),
        installed_version: installed.version.clone(),
        directory: directory.clone(),
        old_hashes: [
            io::hash_file(&directory.join(NAMES[0]))?,
            io::hash_file(&directory.join(NAMES[1]))?,
        ],
        helper_hash: io::digest(helper),
        manifest: String::from_utf8(raw.to_vec())?,
        signature: String::from_utf8(signature.to_vec())?,
    };
    ensure!(
        p.helper_hash == p.old_hashes[0],
        "更新助手不是当前已安装主程序"
    );
    baseline(&p)?;
    delta_files::save_new(&stage.join("package.msi"), package, cancel)?;
    validate_package(&stage, &p, a)?;
    delta_files::save_new(&stage.join("helper.exe"), helper, cancel)?;
    delta_files::save_new(
        &stage.join("plan.json"),
        &serde_json::to_vec_pretty(&p)?,
        cancel,
    )?;
    let changes = signed::changes(&a.manifest.tools)?;
    guard.1 = true;
    Ok(Prepared {
        stage,
        directory,
        installed_version: p.installed_version,
        version: a.manifest.version.clone(),
        bytes: package.len() as u64,
        changes,
    })
}
pub fn prepare(
    raw: &[u8],
    signature: &[u8],
    package: &[u8],
    cancel: &AtomicBool,
) -> Result<Prepared> {
    let a = signed::verify_official(raw, signature)?;
    let installed = current_installation()?;
    let helper = delta_files::read(&std::env::current_exe()?, false, cancel)?;
    prepare_checked(
        &base(true)?,
        &installed,
        env!("CARGO_PKG_VERSION"),
        raw,
        signature,
        package,
        &helper,
        cancel,
        &a,
    )
}
fn apply_checked(
    stage: &Path,
    p: &Plan,
    a: &signed::Authenticated,
    visible: bool,
) -> Result<Receipt> {
    use std::os::windows::fs::OpenOptionsExt;
    let lock = stage
        .parent()
        .context("MSI暂存位置无效")?
        .join("msi-update.lock");
    if !fs::symlink_metadata(&lock).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        io::plain(&lock, false)?;
    }
    let _lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(lock)
        .context("另一个MSI助手正在运行")?;
    ensure!(
        !stage.join("receipt-installing.json").exists(),
        "此MSI计划已经执行，不能重复安装"
    );
    // Pin source bytes against replacement or mutation while Installer reads them.
    io::plain(&stage.join("package.msi"), false)?;
    let _package = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(stage.join("package.msi"))?;
    let package = validate_package(stage, p, a)?;
    baseline(p)?;
    save_receipt(
        stage,
        &record(
            p,
            a,
            "installing",
            None,
            "Windows Installer正在处理；不手动替换EXE".into(),
        ),
    )?;
    let status = installer::install(&stage.join("package.msi"), &p.directory, visible)?;
    let receipt = match status {
        0 => match verified_target(p, &package, a) {
            Ok(()) => record(
                p,
                a,
                "installed",
                Some(status),
                "安装器升级完成，产品与双EXE已核验".into(),
            ),
            Err(e) => record(
                p,
                a,
                "verification-failed",
                Some(status),
                format!(
                    "安装器返回成功，但目标核验失败：{e}；请使用Windows Installer修复，不自动重启"
                ),
            ),
        },
        3010 => record(
            p,
            a,
            "reboot-needed",
            Some(status),
            match verified_target(p, &package, a) {
                Ok(()) => "安装器报告需重启系统完成；当前目标已核验，不自动重启电脑或程序".into(),
                Err(e) => {
                    format!("安装器报告需重启系统完成；当前文件尚未通过核验：{e}；不自动重启")
                }
            },
        ),
        code => {
            let restored = baseline(p).is_ok();
            let reason = match code {
                1602 => "用户取消安装",
                1618 => "Windows Installer正忙",
                1603 => "安装失败",
                _ => "安装器未完成升级",
            };
            record(
                p,
                a,
                if restored { "old-verified" } else { "failed" },
                Some(code),
                format!(
                    "{reason}（{code}）；{}",
                    if restored {
                        "旧产品与双EXE已核验，使用旧版继续"
                    } else {
                        "旧版核验未通过，请使用Windows Installer修复；不覆盖文件或自动重启"
                    }
                ),
            )
        }
    };
    save_receipt(stage, &receipt)?;
    Ok(receipt)
}
pub fn launch(prepared: &Prepared, config: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    let stage = stage_at(&prepared.stage, &base(false)?)?;
    let p = plan(&stage)?;
    let a = signed::verify_official(p.manifest.as_bytes(), p.signature.as_bytes())?;
    validate_package(&stage, &p, &a)?;
    baseline(&p)?;
    ensure!(
        current_installation()?.product_code == p.product_code
            && p.program_version == env!("CARGO_PKG_VERSION"),
        "更新计划不属于当前安装程序"
    );
    ensure!(
        io::hash_file(&stage.join("helper.exe"))? == p.helper_hash,
        "MSI助手摘要不符"
    );
    ensure!(
        !stage.join("receipt-installing.json").exists(),
        "此计划已经运行，请重新准备"
    );
    std::process::Command::new(stage.join("helper.exe"))
        .arg("--apply-msi-update")
        .arg(stage.join("plan.json"))
        .arg(std::process::id().to_string())
        .arg(super::portable_process::own_creation_time()?.to_string())
        .arg(std::path::absolute(config)?)
        .creation_flags(0x08000000)
        .current_dir(&stage)
        .spawn()
        .context("MSI助手启动失败，当前程序继续运行")?;
    Ok(())
}
pub fn run_helper(plan_file: &Path, pid: u32, stamp: u64, config: &Path) -> Result<()> {
    ensure!(
        plan_file.file_name().is_some_and(|n| n == "plan.json"),
        "MSI计划文件名无效"
    );
    let stage = stage_at(
        plan_file.parent().context("MSI计划位置无效")?,
        &base(false)?,
    )?;
    let p = plan(&stage)?;
    let a = signed::verify_official(p.manifest.as_bytes(), p.signature.as_bytes())?;
    ensure!(
        p.program_version == env!("CARGO_PKG_VERSION") && p.helper_hash == p.old_hashes[0],
        "MSI助手版本或基线不符"
    );
    ensure!(
        std::env::current_exe()?.canonicalize()? == stage.join("helper.exe")
            && io::hash_file(&stage.join("helper.exe"))? == p.helper_hash,
        "MSI助手身份不符"
    );
    let result = super::portable_process::wait_parent(pid, stamp)
        .and_then(|()| apply_checked(&stage, &p, &a, true));
    let receipt = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = save_receipt(
                &stage,
                &record(
                    &p,
                    &a,
                    "failed",
                    None,
                    format!("未完成MSI升级：{e}；文件及暂存保留"),
                ),
            );
            return Err(e);
        }
    };
    if matches!(receipt.state.as_str(), "installed" | "old-verified") {
        // Recheck before restart; never start an unverified surviving file.
        let verified = if receipt.state == "installed" {
            validate_package(&stage, &p, &a).and_then(|package| verified_target(&p, &package, &a))
        } else {
            baseline(&p)
        };
        if let Err(e) = verified {
            save_receipt(
                &stage,
                &record(
                    &p,
                    &a,
                    "verification-failed",
                    receipt.status,
                    format!("重启前复核失败：{e}；未启动程序，请检查安装状态"),
                ),
            )?;
            return Err(e);
        }
        let target = p.directory.join(NAMES[0]);
        if std::process::Command::new(target)
            .arg("--no-restore")
            .arg("--config")
            .arg(config)
            .current_dir(&p.directory)
            .spawn()
            .is_err()
        {
            save_receipt(
                &stage,
                &record(
                    &p,
                    &a,
                    "restart-failed",
                    receipt.status,
                    "安装状态已处理，但重启程序失败；请从安装目录手动启动".into(),
                ),
            )?;
        }
    }
    Ok(())
}
pub fn history() -> Result<Vec<Receipt>> {
    let base = base(false)?;
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for entry in fs::read_dir(&base)?.take(4096) {
        let path = entry?.path();
        if rows.len() >= 32 {
            break;
        }
        let Ok(stage) = stage_at(&path, &base) else {
            continue;
        };
        let Ok(p) = plan(&stage) else { continue };
        let Ok(a) = signed::verify_official(p.manifest.as_bytes(), p.signature.as_bytes()) else {
            continue;
        };
        for state in [
            "restart-failed",
            "verification-failed",
            "reboot-needed",
            "failed",
            "old-verified",
            "installed",
            "installing",
        ] {
            let Ok(bytes) = io::read(&stage.join(format!("receipt-{state}.json")), 16384) else {
                continue;
            };
            let Ok(r) = serde_json::from_slice::<Receipt>(&bytes) else {
                continue;
            };
            if r.state == state
                && r.installed_version == p.installed_version
                && r.version == a.manifest.version
            {
                rows.push(r);
                break;
            }
        }
    }
    Ok(rows)
}
#[cfg(test)]
#[path = "msi_tests.rs"]
mod tests;
