//! User-requested portable maintenance; no startup/automatic registry writes.
use super::*;
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use store::joint::{self, MenuPrepared};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ};

fn canonical_location(path: &Path, allow_missing: bool) -> Result<PathBuf> {
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "安装位置不能包含上级目录跳转"
    );
    let path = std::path::absolute(path)?;
    let mut nearest = None;
    for part in path.ancestors() {
        match std::fs::symlink_metadata(part) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                    "安装路径含链接，请选择实际程序或配置位置"
                );
                if nearest.is_none() {
                    if part != path {
                        ensure!(metadata.is_dir(), "配置的既有上级位置不是目录");
                    }
                    nearest = Some(part);
                }
            }
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("程序或配置位置无法核验"),
        }
    }
    let nearest = nearest.context("安装位置没有可核验的既有上级目录")?;
    let canonical = std::fs::canonicalize(nearest)?;
    if nearest == path {
        Ok(canonical)
    } else {
        Ok(canonical.join(path.strip_prefix(nearest)?))
    }
}

fn canonical_file(path: &Path) -> Result<PathBuf> {
    let canonical = canonical_location(path, false)?;
    ensure!(
        canonical.is_file(),
        "程序或配置文件不存在，请先保存配置并重新检查"
    );
    Ok(canonical)
}

/// Resolves the effective installation user's layout without creating anything.
pub fn msi_default_config_path() -> Result<PathBuf> {
    let home = crate::explorer::audit::native::effective_profile_dir()?;
    let default = crate::paths::default_config_path(&home);
    match std::fs::symlink_metadata(&default) {
        Ok(_) => canonical_file(&default),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            canonical_location(&default, true)
        }
        Err(error) => Err(error).context("默认配置位置无法核验"),
    }
}

/// Target metadata from the package before InstallFiles; not file verification.
pub fn msi_candidate(directory: &Path, version: &str, digest: &str) -> Result<Installation> {
    ensure!(directory.is_absolute(), "MSI目标目录必须为绝对路径");
    crate::msi::numeric_version(version)?;
    let mut record = Installation::new(
        Identity::Msi,
        canonical_location(&directory.join("ZiDevTools.exe"), true)?,
        msi_default_config_path()?,
        version.into(),
    );
    record.executable_sha256 = Some(digest.into());
    record.validate()?;
    Ok(record)
}

fn canonical_config_for(path: &Path, allowed_default: Option<&Path>) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => canonical_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let allowed = allowed_default.context("缺失配置只允许当前MSI用户的默认位置")?;
            let canonical = canonical_location(path, true)?;
            ensure!(canonical == allowed, "缺失配置不是当前MSI用户的默认位置");
            Ok(canonical)
        }
        Err(error) => Err(error).context("无法核验配置位置"),
    }
}

pub(super) struct VerifiedFile {
    _file: File,
    executable: PathBuf,
    config: PathBuf,
    digest: String,
}
impl VerifiedFile {
    fn open(executable: &Path, config: &Path) -> Result<Self> {
        Self::open_with_default(executable, config, None)
    }
    fn open_with_default(executable: &Path, config: &Path, default: Option<&Path>) -> Result<Self> {
        let executable = canonical_file(executable)?;
        let config = canonical_config_for(config, default)?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&executable)
            .context("无法锁定程序用于核验，请稍后重试")?;
        let length = file.metadata()?.len();
        ensure!(
            (2..=256 * 1024 * 1024).contains(&length),
            "程序文件大小超出核验范围"
        );
        let mut hasher = Sha256::new();
        let mut signature = [0u8; 2];
        file.read_exact(&mut signature)?;
        ensure!(&signature == b"MZ", "程序不是Windows可执行文件");
        hasher.update(signature);
        let mut buffer = [0u8; 64 * 1024];
        let mut read = 2u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            ensure!(read <= length, "核验期间程序文件发生变化");
            hasher.update(&buffer[..count]);
        }
        ensure!(read == length, "核验期间程序文件发生变化");
        Ok(Self {
            _file: file,
            executable,
            config,
            digest: format!("{:x}", hasher.finalize()),
        })
    }
    fn record(&self, identity: Identity, program_version: &str) -> Installation {
        let mut result = Installation::new(
            identity,
            self.executable.clone(),
            self.config.clone(),
            program_version.into(),
        );
        result.executable_sha256 = Some(self.digest.clone());
        result
    }
    pub(super) fn verify(record: &Installation) -> Result<Self> {
        record.validate()?;
        let missing = matches!(std::fs::symlink_metadata(&record.config),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound);
        let default = if record.identity == Identity::Msi && missing {
            Some(msi_default_config_path()?)
        } else {
            None
        };
        let result =
            Self::open_with_default(&record.executable, &record.config, default.as_deref())?;
        ensure!(
            Some(&result.digest) == record.executable_sha256.as_ref()
                && result.executable == record.executable
                && result.config == record.config,
            "程序或路径身份已变化，请重新预览安装/修复"
        );
        if matches!(record.identity, Identity::Portable(_)) {
            ensure_portable_origin(&result.executable)?;
        }
        Ok(result)
    }
}

#[derive(Clone)]
pub enum Request {
    Install,
    Activate(Identity),
    Remove(Identity),
    Relocate(Identity),
}

fn validate_manual_request(request: &Request) -> Result<()> {
    ensure!(
        !matches!(
            request,
            Request::Remove(Identity::Msi) | Request::Relocate(Identity::Msi)
        ),
        "MSI安装登记的移除和迁移必须由Windows Installer维护，尚未接入"
    );
    Ok(())
}

/// Read-only check shared by manual maintenance and the native MSI fixture.
/// Component ownership is authoritative even for legacy MSI without InstallLocation.
pub fn ensure_portable_origin(executable: &Path) -> Result<()> {
    let executable = canonical_file(executable)?;
    let directory = executable.parent().context("程序目录无效")?;
    ensure!(
        crate::msi::owner(directory)?.is_none(),
        "此程序目录由MSI管理，不能登记或迁移为便携版；请通过Windows Installer维护菜单（尚未接入）"
    );
    Ok(())
}

fn binding(config: &Path) -> Result<(PathBuf, PathBuf)> {
    Ok((std::env::current_exe()?, std::path::absolute(config)?))
}

pub struct Ticket {
    prepared: MenuPrepared,
    binding: (PathBuf, PathBuf),
    title: &'static str,
    preview_only: bool,
}
impl Ticket {
    pub fn current(&self) -> &Ledger {
        self.prepared.current()
    }
    pub fn title(&self) -> &str {
        self.title
    }
    pub fn result(&self) -> &Ledger {
        self.prepared.ledger()
    }
    pub fn counts(&self) -> (usize, usize) {
        self.prepared.counts()
    }
    pub fn matches_current(&self, config: &Path) -> bool {
        binding(config).is_ok_and(|current| current == self.binding)
    }
    pub fn preview_only(&self) -> bool {
        self.preview_only
    }
}

pub fn preview(config: &Path, request: Request, program_version: &str) -> Result<Ticket> {
    semver::Version::parse(program_version)?;
    validate_manual_request(&request)?;
    let before = store::load()?;
    let binding = binding(config)?;
    let (change, eligible, title) = match request {
        Request::Install | Request::Relocate(_) => {
            let verified = VerifiedFile::open(&binding.0, &binding.1)?;
            ensure_portable_origin(&verified.executable)?;
            let current: Vec<_> = before
                .entries()
                .iter()
                .filter(|entry| entry.installation.executable == verified.executable)
                .collect();
            let existing = if let Request::Relocate(identity) = &request {
                ensure!(
                    matches!(identity, Identity::Portable(_)),
                    "MSI安装需通过安装器更新位置"
                );
                ensure!(
                    current
                        .iter()
                        .all(|entry| &entry.installation.identity == identity),
                    "此程序已属于另一条安装记录，无法合并迁移"
                );
                Some(
                    before
                        .entries()
                        .iter()
                        .find(|entry| &entry.installation.identity == identity)
                        .ok_or_else(|| anyhow::anyhow!("待迁移的安装记录不存在"))?,
                )
            } else {
                ensure!(
                    current.len() <= 1,
                    "此程序对应多份安装记录，请先选择要维护的记录"
                );
                current.first().copied()
            };
            let identity = existing.map_or_else(Identity::portable, |entry| {
                entry.installation.identity.clone()
            });
            let mut installation = verified.record(identity.clone(), program_version);
            if let Some(entry) = existing {
                if entry.installation.executable_sha256 == installation.executable_sha256
                    && entry.installation.executable == installation.executable
                    && entry.installation.config == installation.config
                    && entry.installation.version == installation.version
                {
                    installation = entry.installation.clone();
                    (
                        Change::Activate(identity),
                        Eligible(vec![installation]),
                        "修复并启用此程序入口",
                    )
                } else {
                    ensure!(
                        !matches!(identity, Identity::Msi),
                        "MSI程序更新需通过安装器维护，尚未接入"
                    );
                    (
                        Change::Put {
                            installation: installation.clone(),
                            expected_generation: Some(entry.installation.generation.clone()),
                            activate: true,
                        },
                        Eligible(vec![installation]),
                        "更新并启用此程序入口",
                    )
                }
            } else {
                (
                    Change::Put {
                        installation: installation.clone(),
                        expected_generation: None,
                        activate: true,
                    },
                    Eligible(vec![installation]),
                    "安装此程序的经典菜单入口",
                )
            }
        }
        Request::Activate(identity) => {
            let record = before
                .entries()
                .iter()
                .find(|entry| entry.installation.identity == identity)
                .ok_or_else(|| anyhow::anyhow!("安装记录不存在"))?;
            let _verified = VerifiedFile::verify(&record.installation)?;
            (
                Change::Activate(identity),
                Eligible(vec![record.installation.clone()]),
                "切换共享菜单的活动安装",
            )
        }
        Request::Remove(identity) => {
            let record = before
                .entries()
                .iter()
                .find(|entry| entry.installation.identity == identity)
                .ok_or_else(|| anyhow::anyhow!("安装记录不存在"))?;
            let mut remaining: Vec<_> = before
                .entries()
                .iter()
                .filter(|entry| entry.installation.identity != identity)
                .collect();
            remaining.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at_revision));
            // Retain the current active owner when removing an inactive copy.
            if let Some(position) = remaining
                .iter()
                .position(|entry| before.active() == Some(&entry.installation))
            {
                let active = remaining.remove(position);
                remaining.insert(0, active);
            }
            let eligible = remaining
                .into_iter()
                .find(|entry| VerifiedFile::verify(&entry.installation).is_ok())
                .map(|entry| entry.installation.clone())
                .into_iter()
                .collect();
            (
                Change::Remove {
                    identity,
                    expected_generation: record.installation.generation.clone(),
                },
                Eligible(eligible),
                "移除此安装的菜单登记",
            )
        }
    };
    let prepared = joint::preview(prepare(&before, change, &eligible)?)?;
    Ok(Ticket {
        prepared,
        binding,
        title,
        preview_only: false,
    })
}

pub fn apply(ticket: Ticket, current_config: &Path) -> Result<Ledger> {
    ensure!(!ticket.preview_only, "受控预览禁止写入实际菜单");
    ensure!(
        ticket.matches_current(current_config),
        "程序或配置已变化，请重新预览"
    );
    // Hold the executable open without write/delete sharing until KTM finishes.
    let guard = ticket
        .result()
        .active()
        .map(VerifiedFile::verify)
        .transpose()?;
    joint::apply(&ticket.prepared)?;
    let result = ticket.result().clone();
    drop(guard);
    notify_shell();
    Ok(result)
}

fn notify_shell() {
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHChangeNotify(
            event: i32,
            flags: u32,
            item1: *const std::ffi::c_void,
            item2: *const std::ffi::c_void,
        );
    }
    // SHCNE_ASSOCCHANGED | SHCNF_IDLIST/FLUSHNOWAIT. No item paths are sent.
    unsafe { SHChangeNotify(0x08000000, 0x3000, std::ptr::null(), std::ptr::null()) };
}

#[cfg(feature = "ui-preview")]
pub fn fixture_preview(config: &Path, program_version: &str) -> Result<Ticket> {
    let mut ticket = preview(config, Request::Install, program_version)?;
    ticket.preview_only = true;
    Ok(ticket)
}

#[cfg(test)]
mod tests;
