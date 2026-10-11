//! Explicit menu-only restoration after the Installer has restored its files.
use super::*;
use anyhow::Context;
type OwnershipCheck = fn(&Undo, &Intent) -> Result<Vec<u8>>;
type Verified = super::super::super::installer::VerifiedFile;

#[derive(Clone)]
pub struct Ticket {
    pub(super) user: crate::explorer::audit::native::UserIdentity,
    record: Vec<u8>,
    ownership: Vec<u8>,
    id: String,
    plan: Prepared,
    previous: Option<Installation>,
    preview_only: bool,
}
impl Ticket {
    pub fn operation_id(&self) -> &str {
        &self.id
    }
    pub fn current(&self) -> &Ledger {
        &self.plan.before
    }
    pub fn result(&self) -> &Ledger {
        self.plan.result()
    }
    pub fn previous(&self) -> Option<&Installation> {
        self.previous.as_ref()
    }
    pub fn preview_only(&self) -> bool {
        self.preview_only
    }
}
fn undo(record: &Record) -> Result<&Undo> {
    match &record.state {
        State::Applied { undo } | State::CommitReady { undo } => Ok(undo),
        _ => anyhow::bail!("只有已执行且尚未确认结束的菜单维护允许恢复"),
    }
}
fn verified(undo: &Undo, intent: &Intent, current: &Ledger) -> Result<(Vec<Verified>, Eligible)> {
    let mut guards = Vec::new();
    let mut eligible = Vec::new();
    if let Some(previous) = undo.previous_installation() {
        guards.push(
            Verified::verify(previous)
                .context("旧程序尚未恢复或无法核验，请先通过安装器修复或回滚文件")?,
        );
        eligible.push(previous.clone());
    } else {
        let Intent::Install { installation, .. } = intent else {
            anyhow::bail!("缺少原菜单恢复目标");
        };
        ensure!(
            matches!(std::fs::symlink_metadata(&installation.executable),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound),
            "首次安装的程序文件仍存在或无法核验，请先通过安装器完成修复或回滚"
        );
    }
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
        if let Ok(guard) = Verified::verify(&entry.installation) {
            guards.push(guard);
            eligible.push(entry.installation.clone());
            break;
        }
    }
    Ok((guards, Eligible(eligible)))
}
pub(super) fn ownership(undo: &Undo, intent: &Intent) -> Result<Vec<u8>> {
    let expected = undo.previous_installation();
    let location = if let Some(previous) = expected {
        previous
    } else {
        let Intent::Install { installation, .. } = intent else {
            anyhow::bail!("缺少安装位置");
        };
        installation
    };
    let directory = location
        .executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("安装目录无效"))?;
    let actual = crate::msi::owner(directory)?;
    match (expected, &actual) {
        (Some(previous), Some(actual)) => ensure!(
            crate::msi::numeric_version(&actual.version)?
                == crate::msi::numeric_version(&previous.version)?
                && std::fs::canonicalize(&actual.executable)? == previous.executable,
            "Windows Installer的版本或组件位置尚未恢复，请先处理安装器"
        ),
        (None, None) => {}
        _ => anyhow::bail!("Windows Installer组件归属尚未恢复，请先处理安装器"),
    }
    Ok(serde_json::to_vec(&actual)?)
}
pub fn preview(id: &str) -> Result<Ticket> {
    preview_at(&Scope::production(), id, ownership)
}
pub(super) fn preview_at(scope: &Scope, id: &str, owner: OwnershipCheck) -> Result<Ticket> {
    let _guard = serialization::Guard::acquire(scope)?;
    // Declared before tx so files remain locked through rollback on errors too.
    let _files;
    let tx = Transaction::begin()?;
    let record = read_record(scope, id, &tx)?;
    let undo = undo(&record)?;
    let current = read(&scope.menu.ledger, &tx)?;
    undo.require_applied(&current)?;
    let (files, eligible) = verified(undo, &record.intent, &current)?;
    _files = files;
    let ownership = owner(undo, &record.intent)?;
    let plan = undo.rollback(&current, &eligible)?;
    joint::preview_in(plan.clone(), &scope.menu, &tx)?;
    Ok(Ticket {
        user: tx.user.identity.clone(),
        record: serde_json::to_vec(&record)?,
        ownership,
        id: id.into(),
        plan,
        previous: undo.previous_installation().cloned(),
        preview_only: false,
    })
}
/// Caller must explicitly confirm the Installer is closed. Restores only menu
/// registration, never Installer-owned files or product registration.
pub fn apply(ticket: Ticket) -> Result<Ledger> {
    apply_at(ticket, &Scope::production(), true, ownership)
}
pub(super) fn apply_at(
    ticket: Ticket,
    scope: &Scope,
    commit: bool,
    owner: OwnershipCheck,
) -> Result<Ledger> {
    ensure!(!ticket.preview_only, "受控恢复预览禁止应用");
    let _guard = serialization::Guard::acquire(scope)?;
    let _files;
    let tx = Transaction::begin()?;
    tx.user.require_identity(&ticket.user)?;
    let mut record = read_record(scope, &ticket.id, &tx)?;
    ensure!(
        serde_json::to_vec(&record)? == ticket.record,
        "维护记录已变化，请重新预览"
    );
    let undo = undo(&record)?;
    let current = read(&scope.menu.ledger, &tx)?;
    ticket.plan.require_current(&current)?;
    let (files, eligible) = verified(undo, &record.intent, &current)?;
    _files = files;
    ensure!(
        owner(undo, &record.intent)? == ticket.ownership,
        "安装器归属已变化，请重新预览"
    );
    ensure!(
        undo.rollback(&current, &eligible)?.result().active() == ticket.plan.result().active(),
        "可用程序或恢复入口已变化，请重新预览"
    );
    let menu = joint::preview_in(ticket.plan.clone(), &scope.menu, &tx)?;
    joint::apply_in(&menu, &scope.menu, &tx)?;
    record.state = State::Reverted;
    write_record(scope, &record, &tx, false)?;
    let result = ticket.plan.result().clone();
    if commit {
        tx.commit()?;
    }
    Ok(result)
}

#[cfg(feature = "ui-preview")]
pub fn fixture_preview() -> Result<Ticket> {
    let id = "374d06dc-5c86-4dbf-97fb-3985d8f37f19";
    let mut previous = Installation::new(
        Identity::Msi,
        r"C:\Demo\ZiDevTools.exe".into(),
        r"C:\Demo\services.yml".into(),
        "0.81.0".into(),
    );
    previous.executable_sha256 = Some("a".repeat(64));
    let before = super::super::super::prepare(
        &Ledger::default(),
        Change::Put {
            installation: previous.clone(),
            expected_generation: None,
            activate: true,
        },
        &Eligible(vec![previous.clone()]),
    )?;
    let mut next = previous.clone();
    next.generation = uuid::Uuid::new_v4().to_string();
    next.version = "0.82.0".into();
    next.executable_sha256 = Some("b".repeat(64));
    let op = msi_operation::install_with_id(
        before.result(),
        next.clone(),
        true,
        &Eligible(vec![next]),
        id,
    )?;
    let plan = op
        .undo()
        .rollback(op.plan().result(), &Eligible(vec![previous.clone()]))?;
    Ok(Ticket {
        user: crate::explorer::audit::native::UserIdentity(String::new()),
        record: Vec::new(),
        ownership: Vec::new(),
        id: id.into(),
        plan,
        previous: Some(previous),
        preview_only: true,
    })
}
