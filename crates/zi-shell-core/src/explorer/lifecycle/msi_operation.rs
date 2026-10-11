//! MSI domain operations and bounded undo credentials, not Windows Installer actions.
//! Callers must protect credentials and freshly verify eligible executable files.
use super::*;

const MAX_UNDO_BYTES: usize = 512 * 1024;

fn msi(ledger: &Ledger) -> Option<&Installation> {
    ledger
        .entries
        .iter()
        .find(|e| e.installation.identity == Identity::Msi)
        .map(|e| &e.installation)
}

fn validate_msi(record: &Installation) -> Result<()> {
    record.validate()?;
    ensure!(
        record.identity == Identity::Msi && record.executable_sha256.is_some(),
        "MSI操作需要本家族身份和程序摘要"
    );
    Ok(())
}

pub struct Operation {
    plan: Prepared,
    undo: Undo,
}
impl Operation {
    pub fn plan(&self) -> &Prepared {
        &self.plan
    }
    pub fn undo(&self) -> &Undo {
        &self.undo
    }
    fn new(plan: Prepared) -> Result<Self> {
        let undo = Undo {
            schema_version: 1,
            operation_id: uuid::Uuid::new_v4().to_string(),
            before: msi(&plan.before).cloned(),
            after: msi(&plan.after).cloned(),
            previous_active: plan.before.active.clone(),
            applied_revision: plan.after.revision,
            applied_msi_cursor: plan
                .after
                .msi_cursor
                .clone()
                .ok_or_else(|| anyhow::anyhow!("MSI操作缺少游标"))?,
            applied_activation_cursor: plan.after.activation_cursor.clone(),
        };
        undo.validate()?;
        Ok(Self { plan, undo })
    }
}

pub fn install(
    before: &Ledger,
    installation: Installation,
    activate: bool,
    eligible: &Eligible,
) -> Result<Operation> {
    before.validate()?;
    validate_msi(&installation)?;
    let expected_generation = msi(before).map(|record| record.generation.clone());
    Operation::new(prepare(
        before,
        Change::Put {
            installation,
            expected_generation,
            activate,
        },
        eligible,
    )?)
}

/// None only when both the record and expected generation are absent.
pub fn remove(
    before: &Ledger,
    expected_generation: Option<&str>,
    eligible: &Eligible,
) -> Result<Option<Operation>> {
    before.validate()?;
    let current = msi(before);
    ensure!(
        current.map(|record| record.generation.as_str()) == expected_generation,
        "MSI移除代次已变化"
    );
    let Some(current) = current else {
        return Ok(None);
    };
    Operation::new(prepare(
        before,
        Change::Remove {
            identity: Identity::Msi,
            expected_generation: current.generation.clone(),
        },
        eligible,
    )?)
    .map(Some)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Undo {
    schema_version: u32,
    operation_id: String,
    before: Option<Installation>,
    after: Option<Installation>,
    previous_active: Option<Identity>,
    applied_revision: u64,
    applied_msi_cursor: String,
    applied_activation_cursor: Option<String>,
}
impl Undo {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.applied_revision > 0,
            "MSI逆操作协议无效"
        );
        for id in [&self.operation_id, &self.applied_msi_cursor]
            .into_iter()
            .chain(self.applied_activation_cursor.iter())
        {
            Identity::Portable(id.clone()).validate()?;
        }
        for record in [&self.before, &self.after].into_iter().flatten() {
            validate_msi(record)?;
        }
        ensure!(self.before != self.after, "MSI逆操作必须对应实际代次变更");
        if let Some(active) = &self.previous_active {
            active.validate()?;
        }
        ensure!(
            self.previous_active != Some(Identity::Msi) || self.before.is_some(),
            "MSI逆操作的原入口不存在"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_UNDO_BYTES,
            "MSI逆操作数据超限"
        );
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_UNDO_BYTES,
            "MSI逆操作数据超限"
        );
        let result: Self = serde_json::from_slice(bytes)?;
        result.validate()?;
        Ok(result)
    }
    /// Restore only this MSI operation. Never replace a complete historical ledger.
    pub fn rollback(&self, current: &Ledger, eligible: &Eligible) -> Result<Prepared> {
        self.validate()?;
        current.validate()?;
        ensure!(
            current.schema_version == 2
                && current.revision >= self.applied_revision
                && current.msi_cursor.as_ref() == Some(&self.applied_msi_cursor)
                && msi(current) == self.after.as_ref(),
            "MSI操作已过期、未执行或已恢复，拒绝旧逆操作"
        );
        let mut after = current.clone();
        after.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("安装修订号耗尽"))?;
        after
            .entries
            .retain(|entry| entry.installation.identity != Identity::Msi);
        if let Some(record) = &self.before {
            after.entries.push(Entry {
                installation: record.clone(),
                updated_at_revision: after.revision,
            });
        }
        after.msi_cursor = Some(uuid::Uuid::new_v4().to_string());
        if current.activation_cursor == self.applied_activation_cursor {
            after.active = self.previous_active.clone();
        }
        if !after
            .active()
            .is_some_and(|record| eligible.0.contains(record))
        {
            after.active = after
                .entries
                .iter()
                .filter(|entry| eligible.0.contains(&entry.installation))
                .max_by_key(|entry| entry.updated_at_revision)
                .map(|entry| entry.installation.identity.clone());
        }
        // Consume the alias choice even when its identity is unchanged.
        after.activation_cursor = Some(uuid::Uuid::new_v4().to_string());
        after.validate()?;
        Ok(Prepared {
            before: current.clone(),
            after,
        })
    }
}

#[cfg(test)]
mod tests;
