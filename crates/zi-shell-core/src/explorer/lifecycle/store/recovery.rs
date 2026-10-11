//! Fixed per-user protected recovery journal. Not a Windows Installer DLL.
//! Callers must verify actual files and Eligible records immediately before use.
use super::super::msi_operation::{self, Undo};
use super::*;
pub mod cancellation;
pub mod completion;
pub mod restoration;
mod security;
mod serialization;
const JOURNAL: &str = r"Software\ZiCode\ZiDevTools\ExplorerRecovery";
const MAX_RECORDS: usize = 32;
const MAX_RECORD_BYTES: usize = 512 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Intent {
    Install {
        installation: Installation,
        expected_generation: Option<String>,
        activate: bool,
    },
    Remove {
        expected_generation: String,
    },
}
impl Intent {
    fn validate(&self) -> Result<()> {
        let generation = match self {
            Self::Install {
                installation,
                expected_generation,
                ..
            } => {
                installation.validate()?;
                ensure!(
                    installation.identity == Identity::Msi
                        && installation.executable_sha256.is_some(),
                    "日志安装身份或摘要无效"
                );
                expected_generation.as_deref()
            }
            Self::Remove {
                expected_generation,
            } => Some(expected_generation.as_str()),
        };
        if let Some(generation) = generation {
            Identity::Portable(generation.into()).validate()?;
        }
        Ok(())
    }
    fn require_current(&self, current: &Ledger) -> Result<()> {
        current.validate()?;
        ensure!(
            current.pending_msi_operation().is_none(),
            "已有待处理MSI操作"
        );
        let actual = current
            .entries()
            .iter()
            .find(|e| e.installation.identity == Identity::Msi)
            .map(|e| e.installation.generation.as_str());
        let expected = match self {
            Self::Install {
                expected_generation,
                ..
            } => expected_generation.as_deref(),
            Self::Remove {
                expected_generation,
            } => Some(expected_generation.as_str()),
        };
        ensure!(actual == expected, "准备后的MSI代次已变化");
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Prepared,
    Applied,
    CommitReady,
    Committed,
    Reverted,
    Completed,
    Canceled,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Install { version: String },
    Remove,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub operation_id: String,
    pub status: Status,
    pub operation: Operation,
}
/// Bounded, validated metadata only. Reading never creates a journal root.
pub fn list() -> Result<Vec<Summary>> {
    list_at(&Scope::production())
}
fn list_at(scope: &Scope) -> Result<Vec<Summary>> {
    let tx = Transaction::begin()?;
    let Some((_, ids)) = root(scope, &tx, false)? else {
        return Ok(Vec::new());
    };
    let mut summaries = Vec::with_capacity(ids.len());
    for id in ids {
        let record = read_record(scope, &id, &tx)?;
        summaries.push(Summary {
            operation_id: id,
            status: record.state.status(),
            operation: match record.intent {
                Intent::Install { installation, .. } => Operation::Install {
                    version: installation.version,
                },
                Intent::Remove { .. } => Operation::Remove,
            },
        });
    }
    summaries.sort_by(|a, b| a.operation_id.cmp(&b.operation_id));
    Ok(summaries)
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "kebab-case", deny_unknown_fields)]
enum State {
    Prepared,
    Applied { undo: Box<Undo> },
    CommitReady { undo: Box<Undo> },
    Committed { undo: Box<Undo> },
    Reverted,
    Completed,
    Canceled,
}
impl State {
    fn status(&self) -> Status {
        match self {
            Self::Prepared => Status::Prepared,
            Self::Applied { .. } => Status::Applied,
            Self::CommitReady { .. } => Status::CommitReady,
            Self::Committed { .. } => Status::Committed,
            Self::Reverted => Status::Reverted,
            Self::Completed => Status::Completed,
            Self::Canceled => Status::Canceled,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    owner: String,
    operation_id: String,
    intent: Intent,
    state: State,
}
impl Record {
    fn validate(&self, id: &str) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.owner == crate::explorer::OWNER
                && self.operation_id == id,
            "恢复日志协议或归属无效"
        );
        validate_id(id)?;
        self.intent.validate()?;
        if let State::Applied { undo } | State::CommitReady { undo } | State::Committed { undo } =
            &self.state
        {
            ensure!(undo.operation_id() == id, "日志逆操作身份不符");
            undo.to_bytes()?;
            match &self.intent {
                Intent::Install {
                    installation,
                    expected_generation,
                    ..
                } => undo.require_transition(expected_generation.as_deref(), Some(installation))?,
                Intent::Remove {
                    expected_generation,
                } => undo.require_transition(Some(expected_generation), None)?,
            }
        }
        Ok(())
    }
}
struct Scope {
    menu: joint::Scope,
    journal: String,
}
impl Scope {
    fn production() -> Self {
        Self {
            menu: joint::Scope::production(),
            journal: JOURNAL.into(),
        }
    }
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(id.len() == 36, "恢复操作标识长度无效");
    Identity::Portable(id.into()).validate()
}
fn no_values(key: &Key) -> Result<()> {
    let mut name = [0u16; 256];
    let mut size = name.len() as u32;
    ensure!(
        unsafe {
            RegEnumValueW(
                key.0,
                0,
                name.as_mut_ptr(),
                &mut size,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == ERROR_NO_MORE_ITEMS,
        "恢复日志根含未知值"
    );
    Ok(())
}
fn root(scope: &Scope, tx: &Transaction, create: bool) -> Result<Option<(Key, Vec<String>)>> {
    let Some(key) = security::open_protected(&scope.journal, tx, create)? else {
        return Ok(None);
    };
    no_values(&key)?;
    let mut ids = Vec::new();
    loop {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        let code = unsafe {
            RegEnumKeyExW(
                key.0,
                ids.len() as u32,
                name.as_mut_ptr(),
                &mut len,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if code == ERROR_NO_MORE_ITEMS {
            break;
        }
        success(code)?;
        ensure!(
            ids.len() < MAX_RECORDS,
            "恢复日志记录超限，请处理未完成记录"
        );
        let id = String::from_utf16(&name[..len as usize])?;
        validate_id(&id)?;
        ids.push(id);
    }
    Ok(Some((key, ids)))
}
fn read_record(scope: &Scope, id: &str, tx: &Transaction) -> Result<Record> {
    validate_id(id)?;
    let (root, _) =
        root(scope, tx, false)?.ok_or_else(|| anyhow::anyhow!("恢复日志缺失，不能确认恢复结果"))?;
    let key = security::open_child(root.0, id, tx, false)?
        .ok_or_else(|| anyhow::anyhow!("恢复日志记录缺失"))?;
    let mut name = [0u16; 256];
    let mut len = name.len() as u32;
    ensure!(
        unsafe {
            RegEnumKeyExW(
                key.0,
                0,
                name.as_mut_ptr(),
                &mut len,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == ERROR_NO_MORE_ITEMS,
        "恢复日志含未知子项"
    );
    let mut len = name.len() as u32;
    success(unsafe {
        RegEnumValueW(
            key.0,
            0,
            name.as_mut_ptr(),
            &mut len,
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    })?;
    ensure!(
        String::from_utf16(&name[..len as usize])? == "State",
        "恢复日志含未知值"
    );
    let mut len = name.len() as u32;
    ensure!(
        unsafe {
            RegEnumValueW(
                key.0,
                1,
                name.as_mut_ptr(),
                &mut len,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == ERROR_NO_MORE_ITEMS,
        "恢复日志含未知值"
    );
    let mut kind = 0;
    let mut len = 0;
    success(unsafe {
        RegQueryValueExW(
            key.0,
            wide("State").as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut len,
        )
    })?;
    ensure!(
        kind == REG_BINARY && len > 0 && len as usize <= MAX_RECORD_BYTES,
        "恢复日志编码或大小无效"
    );
    let mut bytes = vec![0u8; len as usize];
    success(unsafe {
        RegQueryValueExW(
            key.0,
            wide("State").as_ptr(),
            ptr::null(),
            &mut kind,
            bytes.as_mut_ptr(),
            &mut len,
        )
    })?;
    ensure!(
        kind == REG_BINARY && len as usize == bytes.len(),
        "恢复日志读取发生变化"
    );
    let record: Record = serde_json::from_slice(&bytes)?;
    record.validate(id)?;
    Ok(record)
}
fn write_record(scope: &Scope, record: &Record, tx: &Transaction, fresh: bool) -> Result<()> {
    record.validate(&record.operation_id)?;
    let bytes = serde_json::to_vec(record)?;
    ensure!(bytes.len() <= MAX_RECORD_BYTES, "恢复日志大小超限");
    let (root, ids) = root(scope, tx, fresh)?.ok_or_else(|| anyhow::anyhow!("恢复日志根缺失"))?;
    if fresh {
        ensure!(
            !ids.contains(&record.operation_id) && ids.len() < MAX_RECORDS,
            "恢复操作已存在或日志已满"
        );
    } else {
        ensure!(ids.contains(&record.operation_id), "恢复记录已消失");
    }
    let key = security::open_child(root.0, &record.operation_id, tx, fresh)?
        .ok_or_else(|| anyhow::anyhow!("恢复记录已消失"))?;
    success(unsafe {
        RegSetValueExW(
            key.0,
            wide("State").as_ptr(),
            0,
            REG_BINARY,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    })?;
    drop(key);
    drop(root);
    ensure!(
        serde_json::to_vec(&read_record(scope, &record.operation_id, tx)?)? == bytes,
        "恢复日志事务内复核失败"
    );
    Ok(())
}
/// Persist a fixed intent before file changes. No menus or ledger are changed.
pub fn prepare(id: &str, intent: Intent) -> Result<()> {
    prepare_at(&Scope::production(), id, intent)
}
fn prepare_at(scope: &Scope, id: &str, intent: Intent) -> Result<()> {
    let _guard = serialization::Guard::acquire(scope)?;
    validate_id(id)?;
    intent.validate()?;
    let tx = Transaction::begin()?;
    intent.require_current(&read(&scope.menu.ledger, &tx)?)?;
    write_record(
        scope,
        &Record {
            schema_version: 1,
            owner: crate::explorer::OWNER.into(),
            operation_id: id.into(),
            intent,
            state: State::Prepared,
        },
        &tx,
        true,
    )?;
    tx.commit()
}
/// Rehash and lock required executables until the registry transaction ends.
/// Call only after Installer has put the new files in their final locations.
pub fn execute_verified(id: &str) -> Result<Ledger> {
    transition_verified(&Scope::production(), id, Action::Execute)
}
/// Call only after Installer has restored the old files.
pub fn rollback_verified(id: &str) -> Result<Ledger> {
    transition_verified(&Scope::production(), id, Action::Rollback)
}
fn transition_verified(scope: &Scope, id: &str, action: Action) -> Result<Ledger> {
    let _guard = serialization::Guard::acquire(scope)?;
    let tx = Transaction::begin()?;
    let record = read_record(scope, id, &tx)?;
    let current = read(&scope.menu.ledger, &tx)?;
    let required = match (action, &record.state, &record.intent) {
        (Action::Execute, State::Prepared, Intent::Install { installation, .. }) => {
            Some(installation)
        }
        (Action::Rollback, State::Applied { undo } | State::CommitReady { undo }, _) => {
            undo.previous_installation()
        }
        _ => None,
    };
    let mut guards = Vec::new();
    let mut eligible = Vec::new();
    if let Some(required) = required {
        guards.push(super::super::installer::VerifiedFile::verify(required)?);
        eligible.push(required.clone());
    }
    // Only the best remaining usable installation is needed for fallback.
    // Preserve a user's currently selected portable installation first.
    let mut remaining: Vec<_> = current
        .entries()
        .iter()
        .filter(|e| e.installation.identity != Identity::Msi)
        .collect();
    remaining.sort_by_key(|e| {
        (
            current.active() != Some(&e.installation),
            std::cmp::Reverse(e.updated_at_revision),
        )
    });
    for entry in remaining {
        if let Ok(guard) = super::super::installer::VerifiedFile::verify(&entry.installation) {
            guards.push(guard);
            eligible.push(entry.installation.clone());
            break;
        }
    }
    let after = transition_in(scope, id, &Eligible(eligible), action, &tx)?;
    tx.commit()?;
    drop(guards);
    Ok(after)
}
/// Only call after Windows Installer has successfully committed.
pub fn complete(id: &str) -> Result<Ledger> {
    transition(
        &Scope::production(),
        id,
        &Eligible(vec![]),
        Action::Complete,
        true,
    )
}
pub fn status(id: &str) -> Result<Status> {
    Ok(
        read_record(&Scope::production(), id, &Transaction::begin()?)?
            .state
            .status(),
    )
}
#[derive(Clone, Copy)]
enum Action {
    Execute,
    Rollback,
    Complete,
}
fn transition(
    scope: &Scope,
    id: &str,
    eligible: &Eligible,
    action: Action,
    commit: bool,
) -> Result<Ledger> {
    let _guard = serialization::Guard::acquire(scope)?;
    let tx = Transaction::begin()?;
    let after = transition_in(scope, id, eligible, action, &tx)?;
    if commit {
        tx.commit()?;
    }
    Ok(after)
}
fn transition_in(
    scope: &Scope,
    id: &str,
    eligible: &Eligible,
    action: Action,
    tx: &Transaction,
) -> Result<Ledger> {
    let mut record = read_record(scope, id, tx)?;
    let current = read(&scope.menu.ledger, tx)?;
    let plan = match (action, &record.state) {
        (Action::Execute, State::Prepared) => {
            record.intent.require_current(&current)?;
            let op = match &record.intent {
                Intent::Install {
                    installation,
                    activate,
                    ..
                } => {
                    ensure!(
                        eligible.0.contains(installation),
                        "新安装文件未通过本次核验"
                    );
                    msi_operation::install_with_id(
                        &current,
                        installation.clone(),
                        *activate,
                        eligible,
                        id,
                    )?
                }
                Intent::Remove {
                    expected_generation,
                } => msi_operation::remove_with_id(
                    &current,
                    Some(expected_generation),
                    eligible,
                    id,
                )?
                .ok_or_else(|| anyhow::anyhow!("待移除MSI记录不存在"))?,
            };
            let plan = op.plan().clone();
            record.state = State::Applied {
                undo: Box::new(op.undo().clone()),
            };
            Some(plan)
        }
        (Action::Execute, State::Applied { undo }) => {
            undo.require_applied(&current)?;
            return Ok(current);
        }
        (Action::Rollback, State::Prepared) => {
            record.state = State::Reverted;
            None
        }
        (Action::Rollback, State::Applied { undo } | State::CommitReady { undo }) => {
            let plan = undo.rollback(&current, eligible)?;
            record.state = State::Reverted;
            Some(plan)
        }
        (Action::Complete, State::Applied { undo } | State::Committed { undo }) => {
            let plan = undo.complete(&current)?;
            record.state = State::Completed;
            Some(plan)
        }
        (Action::Rollback, State::Reverted | State::Canceled)
        | (Action::Complete, State::Completed) => {
            return Ok(current);
        }
        _ => anyhow::bail!("恢复日志状态不允许此操作"),
    };
    let after = if let Some(plan) = plan {
        let result = plan.result().clone();
        let menu = joint::preview_in(plan, &scope.menu, tx)?;
        joint::apply_in(&menu, &scope.menu, tx)?;
        result
    } else {
        current
    };
    write_record(scope, &record, tx, false)?;
    Ok(after)
}
/// Explicitly remove one terminal record. Never clear a root or pending entry.
pub fn forget(id: &str) -> Result<()> {
    forget_at(&Scope::production(), id)
}
fn forget_at(scope: &Scope, id: &str) -> Result<()> {
    let _guard = serialization::Guard::acquire(scope)?;
    let tx = Transaction::begin()?;
    let record = read_record(scope, id, &tx)?;
    ensure!(
        matches!(
            record.state,
            State::Reverted | State::Completed | State::Canceled
        ),
        "未完成记录不能清理"
    );
    let (root, _) = root(scope, &tx, false)?.ok_or_else(|| anyhow::anyhow!("恢复日志根缺失"))?;
    success(unsafe {
        RegDeleteKeyTransactedW(root.0, wide(id).as_ptr(), 0, 0, tx.handle, ptr::null())
    })?;
    drop(root);
    tx.commit()
}

#[cfg(test)]
pub(super) mod tests;
