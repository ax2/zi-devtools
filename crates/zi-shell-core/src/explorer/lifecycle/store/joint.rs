//! A single KTM transaction covers both user hives: ledger and menu trees.
use super::*;
use crate::explorer::{
    ROOTS,
    audit::{self, Snapshot},
};

#[derive(Clone)]
pub(in crate::explorer) struct Scope {
    ledger: String,
    menus: [String; 2],
}
impl Scope {
    pub(in crate::explorer) fn production() -> Self {
        Self {
            ledger: ROOT.into(),
            menus: ROOTS.map(str::to_owned),
        }
    }
    fn roots(&self) -> [&str; 2] {
        [self.menus[0].as_str(), self.menus[1].as_str()]
    }
}

fn image(ledger: &Ledger, scope: &Scope) -> Result<Snapshot> {
    let Some(active) = ledger.active() else {
        return Ok(Snapshot::default());
    };
    let mut plan = super::super::menu_plan(active)?;
    for value in &mut plan.values {
        for (root, replacement) in ROOTS.iter().zip(&scope.menus) {
            if value.key.starts_with(root) {
                value.key = value.key.replacen(root, replacement, 1);
                break;
            }
        }
        if value.name == "ExtendedSubCommandsKey" {
            value.value = scope.menus[1]
                .strip_prefix(r"Software\Classes\")
                .ok_or_else(|| anyhow::anyhow!("菜单范围无效"))?
                .into();
        }
    }
    audit::expected(&plan, &scope.roots())
}

fn validate_owned(current: &Snapshot, expected: &Snapshot, scope: &Scope) -> Result<()> {
    ensure!(
        current.keys.iter().all(|key| expected.keys.contains(key))
            && current
                .values
                .keys()
                .all(|key| expected.values.contains_key(key)),
        "菜单含未知内容或缺少对应安装记录，请保留现状"
    );
    for root in &scope.menus {
        let root = root.to_ascii_lowercase();
        if current.keys.contains(&root) {
            for name in [
                "zicodeowner",
                "zicodemenuprotocol",
                "zicodeactiveinstall",
                "zicodeinstallgeneration",
                "zicodebinarysha256",
            ] {
                ensure!(
                    current.values.get(&(root.clone(), name.into()))
                        == expected.values.get(&(root.clone(), name.into())),
                    "菜单所有权、协议或安装代次已变化，请保留现状"
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(in crate::explorer) struct MenuPrepared {
    ledger: Prepared,
    before: Snapshot,
    after: Snapshot,
    user: audit::native::UserIdentity,
}
impl MenuPrepared {
    pub(in crate::explorer) fn current(&self) -> &Ledger {
        &self.ledger.before
    }
    pub(in crate::explorer) fn ledger(&self) -> &Ledger {
        self.ledger.result()
    }
    pub(in crate::explorer) fn counts(&self) -> (usize, usize) {
        let writes = self
            .after
            .values
            .iter()
            .filter(|(key, value)| self.before.values.get(*key) != Some(*value))
            .count();
        let removals = self
            .before
            .values
            .keys()
            .filter(|key| !self.after.values.contains_key(*key))
            .count()
            + self.before.keys.difference(&self.after.keys).count();
        (writes, removals)
    }
}

pub(in crate::explorer) fn preview(ledger: Prepared) -> Result<MenuPrepared> {
    preview_at(ledger, &Scope::production())
}
fn preview_at(ledger: Prepared, scope: &Scope) -> Result<MenuPrepared> {
    let tx = Transaction::begin()?;
    ledger.require_current(&read(&scope.ledger, &tx)?)?;
    let before = audit::native::read_in(&scope.roots(), Some(tx.handle), &tx.user)?;
    validate_owned(&before, &image(&ledger.before, scope)?, scope)?;
    let after = image(&ledger.after, scope)?;
    Ok(MenuPrepared {
        ledger,
        before,
        after,
        user: tx.user.identity.clone(),
    })
}

fn relative(path: &str) -> Result<String> {
    path.to_ascii_lowercase()
        .strip_prefix("software\\classes\\")
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("菜单路径超出当前用户Classes范围"))
}

fn apply_at(prepared: &MenuPrepared, scope: &Scope, commit: bool) -> Result<()> {
    let tx = Transaction::begin()?;
    tx.user.require_identity(&prepared.user)?;
    prepared
        .ledger
        .require_current(&read(&scope.ledger, &tx)?)?;
    let current = audit::native::read_in(&scope.roots(), Some(tx.handle), &tx.user)?;
    ensure!(current == prepared.before, "菜单在预览后已变化，请重新检查");
    validate_owned(&current, &image(&prepared.ledger.before, scope)?, scope)?;
    ensure!(
        prepared.after == image(&prepared.ledger.after, scope)?,
        "菜单计划与安装记录不一致"
    );
    let classes = &tx.user.classes;
    for ((path, name), (kind, bytes)) in &prepared.after.values {
        if current.values.get(&(path.clone(), name.clone())) == Some(&(*kind, bytes.clone())) {
            continue;
        }
        let key = open_at(classes.0, &relative(path)?, &tx, true)?
            .ok_or_else(|| anyhow::anyhow!("无法创建菜单项"))?;
        success(unsafe {
            RegSetValueExW(
                key.0,
                wide(name).as_ptr(),
                0,
                *kind,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        })?;
    }
    for (path, name) in current
        .values
        .keys()
        .filter(|key| !prepared.after.values.contains_key(*key))
    {
        let key = open_at(classes.0, &relative(path)?, &tx, true)?
            .ok_or_else(|| anyhow::anyhow!("菜单项已消失"))?;
        success(unsafe { RegDeleteValueW(key.0, wide(name).as_ptr()) })?;
    }
    let mut removed: Vec<_> = current.keys.difference(&prepared.after.keys).collect();
    removed.sort_by_key(|path| std::cmp::Reverse(path.matches('\\').count()));
    for path in removed {
        let path = relative(path)?;
        let (parent, leaf) = path
            .rsplit_once('\\')
            .ok_or_else(|| anyhow::anyhow!("菜单删除范围无效"))?;
        let parent = open_at(classes.0, parent, &tx, false)?
            .ok_or_else(|| anyhow::anyhow!("菜单父项已消失"))?;
        // Nonrecursive deletion refuses unexpected children; never RegDeleteTree.
        success(unsafe {
            RegDeleteKeyTransactedW(parent.0, wide(leaf).as_ptr(), 0, 0, tx.handle, ptr::null())
        })?;
    }
    write(&scope.ledger, &prepared.ledger.after, &tx)?;
    ensure!(
        audit::native::read_in(&scope.roots(), Some(tx.handle), &tx.user)? == prepared.after,
        "菜单事务内复核失败"
    );
    if commit {
        tx.commit()?;
    }
    Ok(())
}
pub(in crate::explorer) fn apply(prepared: &MenuPrepared) -> Result<()> {
    apply_at(prepared, &Scope::production(), true)
}

#[cfg(test)]
pub(in crate::explorer) fn unapplicable_fixture(ledger: Prepared) -> MenuPrepared {
    // Deliberately inconsistent image: if a preview-only guard regresses, the
    // plan consistency check still rejects before any product registry writes.
    assert!(ledger.result().active().is_some());
    MenuPrepared {
        ledger,
        before: Snapshot::default(),
        after: Snapshot::default(),
        user: audit::native::UserIdentity(String::new()),
    }
}

#[cfg(test)]
mod tests;
