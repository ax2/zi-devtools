//! Durable post-InstallFinalize receipt and explicit, optimistic completion.
//! This is not authentication against malicious code running as the same user.
use super::*;

/// Commit action ran, but a later commit action can still trigger rollback.
/// This never releases Undo or grants the desktop permission to complete.
pub fn mark_commit_ready(id: &str) -> Result<()> {
    ready_at(&Scope::production(), id, true)
}
pub(super) fn ready_at(scope: &Scope, id: &str, commit: bool) -> Result<()> {
    let tx = Transaction::begin()?;
    let mut record = read_record(scope, id, &tx)?;
    let current = read(&scope.menu.ledger, &tx)?;
    match &record.state {
        State::Applied { undo } => {
            undo.require_applied(&current)?;
            record.state = State::CommitReady { undo: undo.clone() };
            write_record(scope, &record, &tx, false)?;
        }
        State::CommitReady { undo } => undo.require_applied(&current)?,
        _ => anyhow::bail!("安装维护不允许记录提交准备状态"),
    }
    if commit {
        tx.commit()?;
    }
    Ok(())
}

/// Installer-only contract: call AFTER the entire owning transaction ended.
/// For a standalone package this is after InstallFinalize. In a multi-package
/// transaction, package InstallFinalize is insufficient: the owner must first
/// successfully commit MsiEndTransaction. CommitReady alone is not proof; this
/// API is a trusted transaction-owner callback, never exposed in desktop UI.
/// Persist the receipt before releasing Undo or the ledger reservation.
pub fn mark_committed(id: &str) -> Result<()> {
    mark_at(&Scope::production(), id, true)
}
pub(super) fn mark_at(scope: &Scope, id: &str, commit: bool) -> Result<()> {
    let tx = Transaction::begin()?;
    let mut record = read_record(scope, id, &tx)?;
    let current = read(&scope.menu.ledger, &tx)?;
    match &record.state {
        State::CommitReady { undo } => {
            undo.require_applied(&current)?;
            record.state = State::Committed { undo: undo.clone() };
            write_record(scope, &record, &tx, false)?;
        }
        State::Committed { undo } => undo.require_applied(&current)?,
        State::Completed => return Ok(()),
        _ => anyhow::bail!("没有可记录提交凭据的已应用安装维护"),
    }
    if commit {
        tx.commit()?;
    }
    Ok(())
}

#[derive(Clone)]
pub struct Ticket {
    pub(super) user: crate::explorer::audit::native::UserIdentity,
    record: Vec<u8>,
    id: String,
    plan: Prepared,
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
    pub fn preview_only(&self) -> bool {
        self.preview_only
    }
}
/// Preview only a receipt-backed completion. Applied/Prepared are not proof
/// of Installer success and cannot be promoted by the desktop UI.
pub fn preview(id: &str) -> Result<Ticket> {
    preview_at(&Scope::production(), id)
}
pub(super) fn preview_at(scope: &Scope, id: &str) -> Result<Ticket> {
    let tx = Transaction::begin()?;
    let record = read_record(scope, id, &tx)?;
    let State::Committed { undo } = &record.state else {
        anyhow::bail!("没有安装事务结束凭据，请先完成或回滚安装程序");
    };
    let current = read(&scope.menu.ledger, &tx)?;
    let plan = undo.complete(&current)?;
    // Reject an unknown or changed menu already during preview.
    joint::preview_in(plan.clone(), &scope.menu, &tx)?;
    Ok(Ticket {
        user: tx.user.identity.clone(),
        record: serde_json::to_vec(&record)?,
        id: id.into(),
        plan,
        preview_only: false,
    })
}
pub fn apply(ticket: Ticket) -> Result<Ledger> {
    apply_at(ticket, &Scope::production(), true)
}
pub(super) fn apply_at(ticket: Ticket, scope: &Scope, commit: bool) -> Result<Ledger> {
    ensure!(!ticket.preview_only, "受控恢复预览禁止应用");
    let tx = Transaction::begin()?;
    tx.user.require_identity(&ticket.user)?;
    let mut record = read_record(scope, &ticket.id, &tx)?;
    ensure!(
        serde_json::to_vec(&record)? == ticket.record
            && matches!(record.state, State::Committed { .. }),
        "维护记录已变化，请重新预览"
    );
    let current = read(&scope.menu.ledger, &tx)?;
    ticket.plan.require_current(&current)?;
    let menu = joint::preview_in(ticket.plan.clone(), &scope.menu, &tx)?;
    joint::apply_in(&menu, &scope.menu, &tx)?;
    record.state = State::Completed;
    write_record(scope, &record, &tx, false)?;
    let result = ticket.plan.result().clone();
    if commit {
        tx.commit()?;
    }
    Ok(result)
}

#[cfg(feature = "ui-preview")]
pub fn fixture_preview() -> Result<Ticket> {
    // Pure synthetic model; never read production registry or configuration.
    let id = "374d06dc-5c86-4dbf-97fb-3985d8f37f19";
    let mut installation = Installation::new(
        Identity::Msi,
        r"C:\Demo\ZiDevTools.exe".into(),
        r"C:\Demo\services.yml".into(),
        "0.82.0".into(),
    );
    installation.executable_sha256 = Some("a".repeat(64));
    let op = msi_operation::install_with_id(
        &Ledger::default(),
        installation.clone(),
        true,
        &Eligible(vec![installation]),
        id,
    )?;
    Ok(Ticket {
        user: crate::explorer::audit::native::UserIdentity(String::new()),
        record: Vec::new(),
        id: id.into(),
        plan: op.undo().complete(op.plan().result())?,
        preview_only: true,
    })
}
