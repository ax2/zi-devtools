//! Explicit cancellation of an unexecuted MENU intent, not Installer rollback.
//! A still-running Installer may subsequently fail its rejected menu action.
use super::*;

#[derive(Clone)]
pub struct Ticket {
    pub(super) user: crate::explorer::audit::native::UserIdentity,
    record: Vec<u8>,
    id: String,
    current: Ledger,
    preview_only: bool,
}
impl Ticket {
    pub fn operation_id(&self) -> &str {
        &self.id
    }
    pub fn current(&self) -> &Ledger {
        &self.current
    }
    pub fn preview_only(&self) -> bool {
        self.preview_only
    }
}
/// Read-only preview; it does not infer Installer idleness or terminate it.
pub fn preview(id: &str) -> Result<Ticket> {
    preview_at(&Scope::production(), id)
}
pub(super) fn preview_at(scope: &Scope, id: &str) -> Result<Ticket> {
    let tx = Transaction::begin()?;
    let record = read_record(scope, id, &tx)?;
    ensure!(
        matches!(record.state, State::Prepared),
        "只能取消尚未执行的菜单维护"
    );
    Ok(Ticket {
        user: tx.user.identity.clone(),
        record: serde_json::to_vec(&record)?,
        id: id.into(),
        current: read(&scope.menu.ledger, &tx)?,
        preview_only: false,
    })
}
/// Caller must explicitly confirm the Installer is closed before applying.
/// A later Installer action will reject the canceled record, possibly failing
/// that installation. This only cancels the MENU intent, not Installer files.
pub fn apply(ticket: Ticket) -> Result<Ledger> {
    apply_at(ticket, &Scope::production(), true)
}
pub(super) fn apply_at(ticket: Ticket, scope: &Scope, commit: bool) -> Result<Ledger> {
    ensure!(!ticket.preview_only, "受控取消预览禁止应用");
    let _guard = serialization::Guard::acquire(scope)?;
    let tx = Transaction::begin()?;
    tx.user.require_identity(&ticket.user)?;
    let mut record = read_record(scope, &ticket.id, &tx)?;
    ensure!(
        matches!(record.state, State::Prepared) && serde_json::to_vec(&record)? == ticket.record,
        "维护记录已变化或已经执行，请重新检查"
    );
    let current = read(&scope.menu.ledger, &tx)?;
    ensure!(current == ticket.current, "安装登记已变化，请重新预览");
    record.state = State::Canceled;
    // Only the journal changes. Never rewrite an old ledger or touch menus,
    // Installer-owned files, registration, or another operation's reservation.
    write_record(scope, &record, &tx, false)?;
    if commit {
        tx.commit()?;
    }
    Ok(current)
}

#[cfg(feature = "ui-preview")]
pub fn fixture_preview() -> Ticket {
    Ticket {
        user: crate::explorer::audit::native::UserIdentity(String::new()),
        record: Vec::new(),
        id: "374d06dc-5c86-4dbf-97fb-3985d8f37f19".into(),
        current: Ledger::default(),
        preview_only: true,
    }
}
