//! Bounded read-only snapshots; matching a plan is not Explorer acceptance.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

use super::{OWNER, Plan, ROOTS};
#[cfg(windows)]
pub(super) mod native;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) keys: BTreeSet<String>,
    pub(super) values: BTreeMap<(String, String), (u32, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Missing,
    Matches,
    OtherInstallation,
    Changed,
    Conflict,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    pub missing: usize,
    pub changed: usize,
    pub unknown: usize,
}

impl Report {
    pub fn label(&self) -> &'static str {
        match self.state {
            State::Missing => "当前用户下未发现这两处候选入口",
            State::Matches => "注册内容与当前候选计划一致",
            State::OtherInstallation => "入口与另一份已登记程序或配置一致",
            State::Changed => "属于本程序的标记存在，但内容缺失或有变化",
            State::Conflict => "发现所有权或未知内容冲突，请保留现状",
            State::Unsupported => "发现本程序的其他菜单协议版本",
        }
    }
}

fn normalize(value: &str) -> String {
    value.to_ascii_lowercase()
}
fn string_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub(super) fn expected(plan: &Plan, roots: &[&str]) -> Result<Snapshot> {
    ensure!(
        plan.scope == "HKEY_CURRENT_USER",
        "检查只支持当前用户候选计划"
    );
    let mut result = Snapshot::default();
    for entry in &plan.values {
        let root = roots
            .iter()
            .find(|root| {
                entry.key.eq_ignore_ascii_case(root)
                    || entry
                        .key
                        .to_ascii_lowercase()
                        .starts_with(&(root.to_ascii_lowercase() + "\\"))
            })
            .ok_or_else(|| anyhow::anyhow!("候选计划包含范围外路径"))?;
        let mut key = entry.key.as_str();
        loop {
            result.keys.insert(normalize(key));
            if key.eq_ignore_ascii_case(root) {
                break;
            }
            key = key
                .rsplit_once('\\')
                .ok_or_else(|| anyhow::anyhow!("候选路径无效"))?
                .0;
        }
        ensure!(
            result
                .values
                .insert(
                    (normalize(&entry.key), normalize(&entry.name)),
                    (1, string_bytes(&entry.value))
                )
                .is_none(),
            "候选计划有重复注册值"
        );
    }
    Ok(result)
}

fn classify(actual: &Snapshot, wanted: &Snapshot, roots: &[&str]) -> Report {
    let mut report = Report {
        state: State::Missing,
        missing: 0,
        changed: 0,
        unknown: 0,
    };
    if actual.keys.is_empty() {
        report.missing = wanted.keys.len() + wanted.values.len();
        return report;
    }
    report.missing = wanted.keys.difference(&actual.keys).count();
    for (key, value) in &wanted.values {
        match actual.values.get(key) {
            None => report.missing += 1,
            Some(current) if current != value => report.changed += 1,
            _ => {}
        }
    }
    report.unknown = actual.keys.difference(&wanted.keys).count()
        + actual
            .values
            .keys()
            .filter(|key| !wanted.values.contains_key(*key))
            .count();
    let mut unsupported = false;
    for root in roots {
        let root = normalize(root);
        if actual.keys.contains(&root) {
            if actual.values.get(&(root.clone(), "zicodeowner".into()))
                != Some(&(1, string_bytes(OWNER)))
            {
                report.state = State::Conflict;
                return report;
            }
            if actual
                .values
                .get(&(root.clone(), "zicodemenuprotocol".into()))
                != wanted
                    .values
                    .get(&(root.clone(), "zicodemenuprotocol".into()))
            {
                unsupported = true;
            }
        }
    }
    if unsupported {
        report.state = State::Unsupported;
        return report;
    }
    report.state = if report.unknown > 0 {
        State::Conflict
    } else if report.missing > 0 || report.changed > 0 {
        State::Changed
    } else {
        State::Matches
    };
    report
}

fn consistent(first: Snapshot, second: Snapshot) -> Result<Snapshot> {
    ensure!(
        first == second,
        "入口内容正在变化，请稍后重新检查；未改动任何注册内容"
    );
    Ok(first)
}

pub fn inspect(plan: &Plan) -> Result<Report> {
    let wanted = expected(plan, &ROOTS)?;
    #[cfg(windows)]
    {
        let ledger = super::lifecycle::store::load()?;
        let active = ledger.active();
        let wanted = if let Some(active) = active {
            expected(&super::lifecycle::menu_plan(active)?, &ROOTS)?
        } else {
            wanted
        };
        let current = consistent(native::read(&ROOTS)?, native::read(&ROOTS)?)?;
        let mut report = classify(&current, &wanted, &ROOTS);
        if report.state == State::Matches
            && active.is_some_and(|active| {
                !same_path(&active.executable, &plan.executable)
                    || !same_path(&active.config, &plan.config)
            })
        {
            report.state = State::OtherInstallation;
        }
        Ok(report)
    }
    #[cfg(not(windows))]
    {
        let _ = wanted;
        anyhow::bail!("注册状态检查仅支持Windows")
    }
}
#[cfg(windows)]
fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    std::fs::canonicalize(left).unwrap_or_else(|_| left.into())
        == std::fs::canonicalize(right).unwrap_or_else(|_| right.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_and_unknown_content_prevent_false_matches() {
        let dir = std::env::current_dir().unwrap();
        let plan =
            super::super::plan(&dir.join("ZiDevTools.exe"), &dir.join("config.yml")).unwrap();
        let wanted = expected(&plan, &ROOTS).unwrap();
        let absent = classify(&Snapshot::default(), &wanted, &ROOTS);
        assert_eq!(absent.state, State::Missing);
        assert_eq!(absent.missing, wanted.keys.len() + wanted.values.len());
        assert_eq!(classify(&wanted, &wanted, &ROOTS).state, State::Matches);
        let mut changed = wanted.clone();
        let command = changed
            .values
            .keys()
            .find(|(key, _)| key.ends_with("\\command"))
            .unwrap()
            .clone();
        changed
            .values
            .insert(command, (1, string_bytes("other.exe")));
        assert_eq!(classify(&changed, &wanted, &ROOTS).state, State::Changed);
        changed
            .keys
            .insert(format!("{}\\foreign", normalize(ROOTS[0])));
        assert_eq!(classify(&changed, &wanted, &ROOTS).state, State::Conflict);
        assert!(consistent(wanted.clone(), changed).is_err());
        let mut foreign = wanted.clone();
        foreign
            .values
            .insert((normalize(ROOTS[0]), "zicodeowner".into()), (3, vec![0]));
        assert_eq!(classify(&foreign, &wanted, &ROOTS).state, State::Conflict);
        let mut newer = wanted.clone();
        newer.values.insert(
            (normalize(ROOTS[0]), "zicodemenuprotocol".into()),
            (1, string_bytes("2")),
        );
        assert_eq!(classify(&newer, &wanted, &ROOTS).state, State::Unsupported);
        newer
            .values
            .insert((normalize(ROOTS[1]), "zicodeowner".into()), (3, vec![0]));
        assert_eq!(classify(&newer, &wanted, &ROOTS).state, State::Conflict);
        let mut partial = wanted.clone();
        partial
            .keys
            .retain(|key| !key.starts_with(&normalize(ROOTS[1])));
        partial
            .values
            .retain(|(key, _), _| !key.starts_with(&normalize(ROOTS[1])));
        assert_eq!(classify(&partial, &wanted, &ROOTS).state, State::Changed);
    }
}
