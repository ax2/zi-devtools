//! Authenticated portable replacement with retained baseline and explicit recovery.
//! Targets only the two program EXEs; never changes user data or MSI files.
use super::{delta_files, signed};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

const NAMES: [&str; 2] = ["ZiDevTools.exe", "ZiDevToolsMcp.exe"];
const MAX_PLAN: usize = 2 * signed::MAX_MANIFEST + 8192;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u32,
    pub old_version: String,
    pub old_hashes: [String; 2],
    pub manifest: String,
    pub signature: String,
    pub helper_hash: String,
}
#[derive(Clone)]
pub struct Prepared {
    pub directory: PathBuf,
    pub version: String,
    pub old_version: String,
    pub bytes: u64,
    pub changes: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub schema: u32,
    pub state: String,
    pub old_version: String,
    pub version: String,
    pub message: String,
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(std::sync::atomic::Ordering::Relaxed),
        "升级准备已取消"
    );
    Ok(())
}
fn plain(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path).context("无法检查更新位置")?;
    ensure!(
        if directory { m.is_dir() } else { m.is_file() },
        "更新位置类型错误"
    );
    ensure!(!m.file_type().is_symlink(), "更新位置不能是链接");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(m.file_attributes() & 0x400 == 0, "更新位置不能是重解析点");
    }
    Ok(())
}
fn root(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "更新目录必须为绝对路径");
    // Directory aliases may be resolved once, but the pinned physical path must be plain.
    let path = path.canonicalize()?;
    for p in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        plain(p, true)?;
    }
    Ok(path)
}
fn stage(path: &Path) -> Result<PathBuf> {
    plain(path, true)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("更新暂存目录名称无效")?;
    let uuid = name
        .strip_prefix(".zi-update-")
        .context("不是Zi DevTools更新目录")?;
    ensure!(uuid::Uuid::parse_str(uuid).is_ok(), "更新目录标识无效");
    let parent = root(path.parent().context("更新目录无效")?)?;
    let pinned = parent.join(name);
    plain(&pinned, true)?;
    Ok(pinned)
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    plain(path, false)?;
    use std::io::Read;
    let mut data = Vec::new();
    delta_files::open_regular(path, limit)?
        .take((limit + 1) as u64)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= limit, "更新文件超过限制");
    Ok(data)
}
fn plan(directory: &Path) -> Result<Plan> {
    let p: Plan = serde_json::from_slice(&read(&directory.join("plan.json"), MAX_PLAN)?)?;
    ensure!(
        p.schema == 1 && semver::Version::parse(&p.old_version).is_ok(),
        "更新计划协议无效"
    );
    ensure!(
        p.old_hashes
            .iter()
            .chain([&p.helper_hash])
            .all(|s| s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())),
        "基线摘要无效"
    );
    Ok(p)
}
fn authenticated(p: &Plan) -> Result<signed::Authenticated> {
    signed::verify_official(p.manifest.as_bytes(), p.signature.as_bytes())
}
fn hash_file(path: &Path) -> Result<String> {
    Ok(digest(&read(path, super::delta::MAX_FILE)?))
}
fn target_hashes(a: &signed::Authenticated) -> Result<[String; 2]> {
    let version = semver::Version::parse(&a.manifest.version)?;
    let mut hashes = [String::new(), String::new()];
    for (i, prefix) in ["ZiDevTools", "ZiDevToolsMcp"].iter().enumerate() {
        let name = format!("{prefix}-{}-windows-x64.exe", a.manifest.version);
        let f = a
            .manifest
            .files
            .iter()
            .find(|f| f.name == name)
            .context("缺少程序附件")?;
        signed::expected_file(a, &version, &name, f.size)?;
        hashes[i] = f.sha256.clone();
    }
    Ok(hashes)
}
fn verify_new(directory: &Path, a: &signed::Authenticated) -> Result<()> {
    let hashes = target_hashes(a)?;
    for (i, name) in NAMES.iter().enumerate() {
        let bytes = read(
            &directory.join(format!("new-{name}")),
            super::delta::MAX_FILE,
        )?;
        let f = a
            .manifest
            .files
            .iter()
            .find(|f| {
                f.name
                    == format!(
                        "{}-{}-windows-x64.exe",
                        if i == 0 {
                            "ZiDevTools"
                        } else {
                            "ZiDevToolsMcp"
                        },
                        a.manifest.version
                    )
            })
            .context("附件摘要无效")?;
        ensure!(
            bytes.len() as u64 == f.size && digest(&bytes) == hashes[i],
            "暂存程序大小或摘要不匹配"
        );
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8], cancel: &AtomicBool) -> Result<()> {
    delta_files::save_new(path, bytes, cancel)?;
    Ok(())
}
fn receipt(
    directory: &Path,
    p: &Plan,
    a: &signed::Authenticated,
    state: &str,
    message: &str,
) -> Result<()> {
    let data = serde_json::to_vec_pretty(&Receipt {
        schema: 1,
        state: state.into(),
        old_version: p.old_version.clone(),
        version: a.manifest.version.clone(),
        message: message.into(),
    })?;
    let file = directory.join(format!("receipt-{state}.json"));
    if !file.exists() {
        write_new(&file, &data, &AtomicBool::new(false))?;
    }
    Ok(())
}
pub fn current_directory() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    ensure!(
        exe.file_name().and_then(|s| s.to_str()) == Some(NAMES[0]),
        "便携更新要求主程序名为ZiDevTools.exe，先使用完整便携运行目录"
    );
    let dir = root(exe.parent().context("程序目录不可用")?)?;
    reject_msi(&dir)?;
    for name in NAMES {
        plain(&dir.join(name), false)?;
    }
    Ok(dir)
}
fn reject_msi(directory: &Path) -> Result<()> {
    #[cfg(windows)]
    super::portable_process::reject_installer_marker()?;
    // The existing MSI installs here. Do not bypass Windows Installer ownership.
    for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(variable) {
            let candidates = [
                PathBuf::from(&base).join("Programs/ZiDevTools"),
                PathBuf::from(base).join("ZiDevTools"),
            ];
            for candidate in candidates {
                if candidate.exists() && candidate.canonicalize()? == directory {
                    anyhow::bail!("这是安装器目录，请使用MSI升级；不能直接替换受管理程序");
                }
            }
        }
    }
    Ok(())
}
/// Preparation touches only a new UUID staging directory, never the live EXEs.
pub fn prepare(
    directory: &Path,
    old_version: &str,
    raw: &[u8],
    signature: &[u8],
    binaries: [&[u8]; 2],
    helper: &[u8],
    cancel: &AtomicBool,
) -> Result<Prepared> {
    cancelled(cancel)?;
    let dir = root(directory)?;
    reject_msi(&dir)?;
    let a = signed::verify_official(raw, signature)?;
    prepare_authenticated(
        &dir,
        old_version,
        raw,
        signature,
        binaries,
        helper,
        cancel,
        &a,
    )
}
#[allow(clippy::too_many_arguments)]
fn prepare_authenticated(
    dir: &Path,
    old_version: &str,
    raw: &[u8],
    signature: &[u8],
    binaries: [&[u8]; 2],
    helper: &[u8],
    cancel: &AtomicBool,
    a: &signed::Authenticated,
) -> Result<Prepared> {
    let old = semver::Version::parse(old_version)?;
    let new = semver::Version::parse(&a.manifest.version)?;
    ensure!(
        new.cmp_precedence(&old).is_gt(),
        "目标版本必须高于当前版本，不允许降级或同版替换"
    );
    let hashes = target_hashes(a)?;
    for (i, data) in binaries.iter().enumerate() {
        ensure!(digest(data) == hashes[i], "准备程序摘要与签名不符");
    }
    let old_hashes = [
        hash_file(&dir.join(NAMES[0]))?,
        hash_file(&dir.join(NAMES[1]))?,
    ];
    ensure!(
        digest(helper) == old_hashes[0],
        "更新助手必须为当前桌面程序的精确副本"
    );
    let directory = dir.join(format!(".zi-update-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory)?;
    let mut guard = PreparationGuard {
        directory: directory.clone(),
        committed: false,
    };
    // Cancelled/failed preparation removes only its own known staging files.
    for (i, name) in NAMES.iter().enumerate() {
        write_new(&directory.join(format!("new-{name}")), binaries[i], cancel)?;
    }
    write_new(&directory.join("helper.exe"), helper, cancel)?;
    let p = Plan {
        schema: 1,
        old_version: old_version.into(),
        old_hashes,
        manifest: String::from_utf8(raw.to_vec())?,
        signature: String::from_utf8(signature.to_vec())?,
        helper_hash: digest(helper),
    };
    verify_new(&directory, a)?;
    cancelled(cancel)?;
    let plan_bytes = serde_json::to_vec_pretty(&p)?;
    ensure!(plan_bytes.len() <= MAX_PLAN, "更新计划超出大小限制");
    write_new(&directory.join("plan.json"), &plan_bytes, cancel)?;
    let changes = signed::changes(&a.manifest.tools)?;
    guard.committed = true;
    Ok(Prepared {
        directory,
        version: a.manifest.version.clone(),
        old_version: old_version.into(),
        bytes: binaries.iter().map(|b| b.len() as u64).sum(),
        changes,
    })
}
struct PreparationGuard {
    directory: PathBuf,
    committed: bool,
}
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        if self.committed || stage(&self.directory).is_err() {
            return;
        }
        for name in [
            "new-ZiDevTools.exe",
            "new-ZiDevToolsMcp.exe",
            "helper.exe",
            "plan.json",
        ] {
            let path = self.directory.join(name);
            if plain(&path, false).is_ok() {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir(&self.directory);
    }
}
struct Lock(fs::File);
fn lock(directory: &Path) -> Result<Lock> {
    let file = directory.join(".zi-update.lock");
    match fs::symlink_metadata(&file) {
        Ok(_) => plain(&file, false)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    Ok(Lock(
        options
            .open(file)
            .context("另一个更新助手正在操作，或目录不可写")?,
    ))
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.sync_all();
    }
}
fn restore(directory: &Path, p: &Plan, a: &signed::Authenticated) -> Result<()> {
    let parent = root(directory.parent().context("暂存目录不可用")?)?;
    let next = target_hashes(a)?;
    // Preflight ALL files before removing any authenticated new target.
    for (i, name) in NAMES.iter().enumerate() {
        let target = parent.join(name);
        let backup = directory.join(format!("old-{name}"));
        if target.exists() {
            let hash = hash_file(&target)?;
            ensure!(
                hash == p.old_hashes[i] || hash == next[i],
                "当前程序已被外部修改，拒绝覆盖；请检查恢复副本"
            );
        }
        if backup.exists() {
            ensure!(hash_file(&backup)? == p.old_hashes[i], "恢复副本摘要不符");
        }
        if !target.exists() || hash_file(&target)? != p.old_hashes[i] {
            ensure!(backup.exists(), "缺少有效恢复副本");
        }
    }
    for (i, name) in NAMES.iter().enumerate() {
        let target = parent.join(name);
        let backup = directory.join(format!("old-{name}"));
        if target.exists() && hash_file(&target)? == p.old_hashes[i] {
            continue;
        }
        if target.exists() {
            fs::remove_file(&target).context("新程序被占用，无法恢复；恢复副本仍保留")?;
        }
        fs::hard_link(&backup, &target).context("无法恢复旧程序；恢复副本仍保留")?;
    }
    receipt(directory, p, a, "restored", "旧程序已恢复，恢复副本保留")?;
    Ok(())
}
/// Explicit recovery also handles an interrupted two-file commit. It never overwrites unknown files.
pub fn recover(directory: &Path) -> Result<()> {
    let directory = stage(directory)?;
    let parent = root(directory.parent().context("目录无效")?)?;
    reject_msi(&parent)?;
    let _lock = lock(&parent)?;
    let p = plan(&directory)?;
    let a = authenticated(&p)?;
    restore(&directory, &p, &a)
}
pub fn apply(directory: &Path) -> Result<()> {
    let directory = stage(directory)?;
    let parent = root(directory.parent().context("目录无效")?)?;
    reject_msi(&parent)?;
    let _lock = lock(&parent)?;
    let p = plan(&directory)?;
    let a = authenticated(&p)?;
    apply_authenticated(&directory, &parent, &p, &a, || Ok(()))
}
fn apply_authenticated(
    directory: &Path,
    parent: &Path,
    p: &Plan,
    a: &signed::Authenticated,
    mut between: impl FnMut() -> Result<()>,
) -> Result<()> {
    verify_new(directory, a)?;
    ensure!(
        semver::Version::parse(&a.manifest.version)?
            .cmp_precedence(&semver::Version::parse(&p.old_version)?)
            .is_gt(),
        "目标版本不能降级"
    );
    for (i, name) in NAMES.iter().enumerate() {
        ensure!(
            hash_file(&parent.join(name))? == p.old_hashes[i],
            "当前程序与准备时不同，请重新准备"
        );
        ensure!(
            !directory.join(format!("old-{name}")).exists(),
            "此更新已应用或有中断记录，请先恢复或重新准备"
        );
    }
    receipt(
        directory,
        p,
        a,
        "applying",
        "已开始替换，退出中断后可主动恢复",
    )?;
    let result = (|| {
        for (i, name) in NAMES.iter().enumerate() {
            let target = parent.join(name);
            let backup = directory.join(format!("old-{name}"));
            fs::hard_link(&target, &backup).context("无法保留旧程序")?;
            fs::remove_file(&target).context("当前程序仍被占用，不能替换")?;
            fs::hard_link(directory.join(format!("new-{name}")), &target)
                .context("无法安装新程序")?;
            if i == 0 {
                between()?;
            }
        }
        let next = target_hashes(a)?;
        for (i, name) in NAMES.iter().enumerate() {
            ensure!(hash_file(&parent.join(name))? == next[i], "替换后摘要不符");
        }
        receipt(
            directory,
            p,
            a,
            "installed",
            "新程序已安装，旧程序副本保留；可主动恢复",
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        let recovery = restore(directory, p, a);
        let message = match recovery {
            Ok(()) => format!("更新失败，旧版本已恢复：{error}"),
            Err(e) => format!("更新失败：{error}；自动恢复未完成：{e}；恢复副本保留"),
        };
        let _ = receipt(directory, p, a, "failed", &message);
        anyhow::bail!(message);
    }
    Ok(())
}
pub fn launch(prepared: &Prepared, restore: bool, config: &Path) -> Result<()> {
    let directory = stage(&prepared.directory)?;
    let p = plan(&directory)?;
    authenticated(&p)?;
    let helper = directory.join("helper.exe");
    ensure!(hash_file(&helper)? == p.helper_hash, "更新助手摘要不匹配");
    let parent = root(directory.parent().context("更新目录无效")?)?;
    ensure!(current_directory()? == parent, "更新计划不属于当前程序目录");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let stamp = super::portable_process::own_creation_time()?;
        std::process::Command::new(helper)
            .args(["--apply-portable-update"])
            .arg(directory.join("plan.json"))
            .arg(std::process::id().to_string())
            .arg(stamp.to_string())
            .arg(if restore { "restore" } else { "apply" })
            .arg(std::path::absolute(config)?)
            .creation_flags(0x08000000)
            .current_dir(&parent)
            .spawn()
            .context("无法启动更新助手，当前程序保留")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (restore, config);
        anyhow::bail!("只支持Windows更新助手")
    }
}
pub fn run_helper(
    plan_file: &Path,
    pid: u32,
    stamp: u64,
    restore: bool,
    config: &Path,
    restart: bool,
) -> Result<()> {
    ensure!(
        plan_file.file_name().and_then(|s| s.to_str()) == Some("plan.json"),
        "更新计划文件名无效"
    );
    let dir = stage(plan_file.parent().context("更新计划位置无效")?)?;
    let p = plan(&dir)?;
    let a = authenticated(&p)?;
    ensure!(
        p.old_version == env!("CARGO_PKG_VERSION"),
        "更新助手与准备时的版本不匹配"
    );
    ensure!(
        std::env::current_exe()?.canonicalize()? == dir.join("helper.exe"),
        "更新助手必须位于自身暂存目录"
    );
    ensure!(
        hash_file(&dir.join("helper.exe"))? == p.helper_hash,
        "更新助手内容改变"
    );
    #[cfg(windows)]
    let wait = if restore && pid == 0 && stamp == 0 {
        Ok(())
    } else {
        super::portable_process::wait_parent(pid, stamp)
    };
    #[cfg(not(windows))]
    let wait: Result<()> = {
        let _ = (pid, stamp);
        anyhow::bail!("只支持Windows助手")
    };
    if let Err(e) = wait {
        let _ = receipt(&dir, &p, &a, "failed", &format!("未替换：{e}"));
        return Err(e);
    }
    let result = if restore { recover(&dir) } else { apply(&dir) };
    if let Err(e) = &result {
        let _ = receipt(&dir, &p, &a, "failed", &e.to_string());
    }
    if !restart {
        return result;
    }
    // Restart the surviving app only; arguments are structured and no shell is used.
    let target = dir.parent().context("目录无效")?.join(NAMES[0]);
    let expected = if result.is_ok() && !restore {
        target_hashes(&a)?[0].clone()
    } else {
        p.old_hashes[0].clone()
    };
    if hash_file(&target).is_ok_and(|hash| hash == expected) {
        let mut cmd = std::process::Command::new(&target);
        cmd.current_dir(target.parent().context("目录无效")?)
            .arg("--no-restore")
            .arg("--config")
            .arg(config);
        if cmd.spawn().is_err() {
            let _ = receipt(
                &dir,
                &p,
                &a,
                "restart-failed",
                "程序替换已处理，但自动重启失败；请从原目录启动",
            );
        }
    }
    result
}
pub fn history(directory: &Path) -> Result<Vec<(Prepared, Receipt)>> {
    let directory = root(directory)?;
    let mut rows = Vec::new();
    for entry in fs::read_dir(&directory)?.take(4096) {
        let path = entry?.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".zi-update-"))
        {
            continue;
        }
        if rows.len() >= 32 {
            break;
        }
        let Ok(path) = stage(&path) else { continue };
        let Ok(p) = plan(&path) else { continue };
        let Ok(a) = authenticated(&p) else { continue };
        for state in [
            "restart-failed",
            "restored",
            "failed",
            "installed",
            "applying",
        ] {
            let file = path.join(format!("receipt-{state}.json"));
            let Ok(bytes) = read(&file, 8192) else {
                continue;
            };
            let Ok(r) = serde_json::from_slice::<Receipt>(&bytes) else {
                continue;
            };
            if r.schema != 1 || r.version != a.manifest.version || r.old_version != p.old_version {
                continue;
            }
            rows.push((
                Prepared {
                    directory: path.clone(),
                    version: a.manifest.version,
                    old_version: p.old_version,
                    bytes: 0,
                    changes: Vec::new(),
                },
                r,
            ));
            break;
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::Ed25519KeyPair;
    struct Fixture {
        root: PathBuf,
        prepared: Prepared,
        plan: Plan,
        auth: signed::Authenticated,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(path), Ok(temp)) = (
                self.root.canonicalize(),
                std::env::temp_dir().canonicalize(),
            ) {
                if path.starts_with(temp)
                    && path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.starts_with("zi-portable-test-"))
                {
                    let _ = fs::remove_dir_all(path);
                }
            }
        }
    }
    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!("zi-portable-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let dir = root(&dir).unwrap();
        fs::write(dir.join(NAMES[0]), b"old desktop").unwrap();
        fs::write(dir.join(NAMES[1]), b"old mcp").unwrap();
        let key = Ed25519KeyPair::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let manifest = signed::Manifest {
            schema: 1,
            version: "0.83.0".into(),
            source_commit: "a".repeat(40),
            platform: "windows-x64".into(),
            minimum_windows_major: 10,
            files: [
                ("ZiDevTools", b"new desktop".as_slice()),
                ("ZiDevToolsMcp", b"new mcp".as_slice()),
                ("ZiDevTools", b"fixture msi".as_slice()),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (prefix, data))| signed::File {
                name: format!(
                    "{prefix}-0.83.0-windows-x64.{}",
                    if i == 2 { "msi" } else { "exe" }
                ),
                size: data.len() as u64,
                sha256: digest(data),
            })
            .collect(),
            tools: signed::tools(
                include_bytes!("../../docs/tools.json"),
                env!("CARGO_PKG_VERSION"),
            )
            .unwrap(),
        };
        let (raw, sig) = signed::sign(&manifest, &key).unwrap();
        let auth = signed::verify(
            &raw,
            &sig,
            &signed::TrustStore {
                schema: 1,
                keys: vec![signed::publisher(&key)],
            },
        )
        .unwrap();
        let prepared = prepare_authenticated(
            &dir,
            "0.82.0",
            &raw,
            &sig,
            [b"new desktop", b"new mcp"],
            b"old desktop",
            &AtomicBool::new(false),
            &auth,
        )
        .unwrap();
        let plan = plan(&prepared.directory).unwrap();
        Fixture {
            root: dir,
            prepared,
            plan,
            auth,
        }
    }
    #[test]
    fn commit_and_explicit_restore_preserve_unrelated_data() {
        let f = fixture();
        fs::write(f.root.join("user-data.json"), b"keep me").unwrap();
        apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(())).unwrap();
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"new desktop");
        assert_eq!(fs::read(f.root.join(NAMES[1])).unwrap(), b"new mcp");
        restore(&f.prepared.directory, &f.plan, &f.auth).unwrap();
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
        assert_eq!(fs::read(f.root.join(NAMES[1])).unwrap(), b"old mcp");
        assert_eq!(fs::read(f.root.join("user-data.json")).unwrap(), b"keep me");
        assert!(f.prepared.directory.join("old-ZiDevTools.exe").exists());
    }
    #[test]
    fn interrupted_two_file_commit_recovers_after_partial_replacement() {
        let f = fixture();
        let p = &f.prepared.directory;
        fs::hard_link(f.root.join(NAMES[0]), p.join("old-ZiDevTools.exe")).unwrap();
        fs::remove_file(f.root.join(NAMES[0])).unwrap();
        fs::hard_link(p.join("new-ZiDevTools.exe"), f.root.join(NAMES[0])).unwrap();
        restore(p, &f.plan, &f.auth).unwrap();
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
        assert_eq!(fs::read(f.root.join(NAMES[1])).unwrap(), b"old mcp");
    }
    #[test]
    fn error_between_files_automatically_restores_first_file() {
        let f = fixture();
        let result = apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || {
            anyhow::bail!("fixture interrupted")
        });
        assert!(result.unwrap_err().to_string().contains("旧版本已恢复"));
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
        assert_eq!(fs::read(f.root.join(NAMES[1])).unwrap(), b"old mcp");
    }
    #[test]
    fn corrupted_stage_or_changed_baseline_never_replaces_live_files() {
        let f = fixture();
        fs::write(
            f.prepared.directory.join("new-ZiDevToolsMcp.exe"),
            b"corrupt",
        )
        .unwrap();
        assert!(
            apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
        let f = fixture();
        fs::write(f.root.join(NAMES[1]), b"external").unwrap();
        assert!(
            apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
    }
    #[test]
    fn recovery_refuses_unknown_target_without_mutating_other_program() {
        let f = fixture();
        apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(())).unwrap();
        fs::write(f.root.join(NAMES[1]), b"foreign").unwrap();
        assert!(restore(&f.prepared.directory, &f.plan, &f.auth).is_err());
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"new desktop");
        assert_eq!(fs::read(f.root.join(NAMES[1])).unwrap(), b"foreign");
    }
    #[test]
    fn no_duplicate_commit_and_corrupted_backup_is_retained() {
        let f = fixture();
        apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(())).unwrap();
        assert!(
            apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(()))
                .is_err()
        );
        fs::write(
            f.prepared.directory.join("old-ZiDevToolsMcp.exe"),
            b"bad backup",
        )
        .unwrap();
        assert!(restore(&f.prepared.directory, &f.plan, &f.auth).is_err());
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"new desktop");
    }
    #[test]
    fn rejected_official_key_and_cancel_and_downgrade_do_not_replace() {
        let f = fixture();
        let p = &f.plan;
        assert!(
            prepare(
                &f.root,
                "0.82.0",
                p.manifest.as_bytes(),
                p.signature.as_bytes(),
                [b"new desktop", b"new mcp"],
                b"old desktop",
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert!(
            prepare_authenticated(
                &f.root,
                "0.83.0",
                p.manifest.as_bytes(),
                p.signature.as_bytes(),
                [b"new desktop", b"new mcp"],
                b"old desktop",
                &AtomicBool::new(false),
                &f.auth
            )
            .is_err()
        );
        assert!(
            prepare(
                &f.root,
                "0.82.0",
                p.manifest.as_bytes(),
                p.signature.as_bytes(),
                [b"new desktop", b"new mcp"],
                b"old desktop",
                &AtomicBool::new(true)
            )
            .is_err()
        );
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
    }
    #[cfg(windows)]
    #[test]
    fn held_mcp_file_triggers_recovery_and_update_lock_is_exclusive() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = fixture();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(f.root.join(NAMES[1]))
            .unwrap();
        assert!(
            apply_authenticated(&f.prepared.directory, &f.root, &f.plan, &f.auth, || Ok(()))
                .is_err()
        );
        assert_eq!(fs::read(f.root.join(NAMES[0])).unwrap(), b"old desktop");
        drop(held);
        let held = lock(&f.root).unwrap();
        assert!(lock(&f.root).is_err());
        drop(held);
        assert!(lock(&f.root).is_ok());
    }
}
