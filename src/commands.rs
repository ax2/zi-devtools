//! Shared command identities, kept independent of Windows hooks and UI focus.
pub mod profile;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Open(String),
    Search,
    RecordStart,
    RecordPause,
    RecordStop,
}
#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub sequence: String,
    pub action: Action,
}

pub fn tool_command(id: &str, title: &str) -> Command {
    let sequence = match id {
        "advanced-calculator" => "C",
        "json" => "J",
        "data" => "D",
        "memos" => "N",
        "calendar-planner" => "A",
        "files" => "F",
        "image-tools" => "G",
        "screen-recorder" => "R O",
        _ => "",
    };
    Command {
        id: format!("open:{id}"),
        title: format!("打开 {title}"),
        sequence: sequence.into(),
        action: Action::Open(id.into()),
    }
}
pub fn controls() -> Vec<Command> {
    [
        ("search", "搜索全部工具", "K", Action::Search),
        ("recorder:start", "开始录屏", "R S", Action::RecordStart),
        (
            "recorder:pause",
            "暂停 / 继续录屏",
            "R P",
            Action::RecordPause,
        ),
        ("recorder:stop", "结束录屏", "R E", Action::RecordStop),
    ]
    .into_iter()
    .map(|(id, title, sequence, action)| Command {
        id: id.into(),
        title: title.into(),
        sequence: sequence.into(),
        action,
    })
    .collect()
}
/// Absence uses the default; an empty value explicitly removes the binding.
pub type Bindings = std::collections::BTreeMap<String, String>;

pub fn normalize_sequence(input: &str) -> Result<String, String> {
    let keys: Vec<_> = input.split_whitespace().collect();
    if input.len() > 64
        || keys.len() > 4
        || keys
            .iter()
            .any(|k| k.len() != 1 || !k.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err("使用 1 至 4 个 A–Z 字母，以空格分隔；留空取消绑定".into());
    }
    Ok(keys.join(" ").to_ascii_uppercase())
}

/// Dormant bindings reserve keys without creating executable commands.
/// Disable every participant in a conflict, rather than choosing an arbitrary winner.
pub fn configured(defaults: &[Command], bindings: &Bindings) -> (Vec<Command>, Vec<String>) {
    let mut commands = defaults.to_vec();
    if bindings.len() > 4096 {
        for command in &mut commands {
            command.sequence = if command.id == "search" {
                "K".into()
            } else {
                String::new()
            };
        }
        return (
            commands,
            vec!["绑定数量超过 4096，请恢复默认后重新配置".into()],
        );
    }
    for c in &mut commands {
        if let Some(s) = bindings.get(&c.id) {
            c.sequence = s.clone();
        }
    }
    let mut slots: Vec<_> = commands
        .iter()
        .map(|c| (c.id.clone(), c.title.clone(), c.sequence.clone()))
        .collect();
    if let Some(search) = slots.iter_mut().find(|s| s.0 == "search") {
        search.2 = "K".into();
    }
    slots.extend(
        bindings
            .iter()
            .take(4096)
            .filter(|(id, _)| !defaults.iter().any(|c| &c.id == *id))
            .map(|(id, s)| (id.clone(), format!("暂不可用的工具 ({id})"), s.clone())),
    );
    let mut invalid = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    if bindings.get("search").is_some_and(|s| s != "K") {
        errors.push("K 保留给全工具搜索，不能修改".into());
    }
    let valid_sequences: Vec<_> = slots
        .iter()
        .map(|slot| normalize_sequence(&slot.2).is_ok_and(|n| n == slot.2))
        .collect();
    for ((id, title, s), valid) in slots.iter().zip(&valid_sequences) {
        if id.is_empty() || id.len() > 512 || !valid || (id == "search" && s != "K") {
            invalid.insert(id.clone());
            errors.push(format!("{title}：无效序列（K 保留给搜索）"));
        }
    }
    for (i, a) in slots.iter().enumerate() {
        for (j, b) in slots.iter().enumerate().skip(i + 1) {
            if !valid_sequences[i] || !valid_sequences[j] {
                continue;
            }
            if !a.2.is_empty()
                && !b.2.is_empty()
                && (a.2 == b.2
                    || a.2
                        .strip_prefix(&b.2)
                        .is_some_and(|suffix| suffix.starts_with(' '))
                    || b.2
                        .strip_prefix(&a.2)
                        .is_some_and(|suffix| suffix.starts_with(' ')))
            {
                invalid.insert(a.0.clone());
                invalid.insert(b.0.clone());
                if errors.len() < 128 {
                    errors.push(format!("{} [{}] 与 {} [{}] 冲突", a.1, a.2, b.1, b.2));
                }
            }
        }
    }
    for c in &mut commands {
        if invalid.contains(&c.id) {
            c.sequence.clear();
        }
    }
    if let Some(search) = commands.iter_mut().find(|c| c.id == "search") {
        search.sequence = "K".into();
    }
    (commands, errors)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Match {
    Pending,
    Run(usize),
    Invalid,
}
pub fn resolve(commands: &[Command], sequence: &str) -> Match {
    if let Some(index) = commands
        .iter()
        .position(|c| !c.sequence.is_empty() && c.sequence == sequence)
    {
        return Match::Run(index);
    }
    if commands
        .iter()
        .any(|c| !c.sequence.is_empty() && c.sequence.starts_with(&format!("{sequence} ")))
    {
        Match::Pending
    } else {
        Match::Invalid
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_and_excessive_settings_have_bounded_diagnostics_and_search_survives() {
        let defaults = controls();
        let mut bindings: Bindings = (0..32)
            .map(|i| (format!("open:plugin:{i}"), "Q".into()))
            .collect();
        let (_, errors) = configured(&defaults, &bindings);
        assert_eq!(errors.len(), 128);
        bindings.insert("recorder:start".into(), "r s".into());
        assert_eq!(
            resolve(&configured(&defaults, &bindings).0, "R S"),
            Match::Invalid
        );
        let bindings = (0..4097)
            .map(|i| (format!("open:{i}"), String::new()))
            .collect();
        let (effective, errors) = configured(&defaults, &bindings);
        assert_eq!(errors.len(), 1);
        assert_eq!(resolve(&effective, "K"), Match::Run(0));
        assert_eq!(resolve(&effective, "R E"), Match::Invalid);
    }
    #[test]
    fn overrides_reject_overlap_and_preserve_dormant_plugins() {
        assert_eq!(normalize_sequence(" c   a ").unwrap(), "C A");
        for bad in ["CA", "1", "Ctrl", "A B C D E", "é"] {
            assert!(normalize_sequence(bad).is_err());
        }
        let mut defaults = controls();
        defaults.push(tool_command("advanced-calculator", "计算器"));
        let mut b = Bindings::new();
        b.insert("open:advanced-calculator".into(), "R".into());
        let (effective, errors) = configured(&defaults, &b);
        assert!(!errors.is_empty());
        assert_eq!(resolve(&effective, "R S"), Match::Invalid);
        b.insert("open:advanced-calculator".into(), "Q A".into());
        assert!(configured(&defaults, &b).1.is_empty());
        b.insert("open:disabled-plugin".into(), "Q A".into());
        let (effective, errors) = configured(&defaults, &b);
        assert!(!errors.is_empty());
        assert_eq!(resolve(&effective, "Q A"), Match::Invalid);
        b.insert("open:advanced-calculator".into(), String::new());
        let (effective, errors) = configured(&defaults, &b);
        assert!(errors.is_empty());
        assert_eq!(resolve(&effective, "C"), Match::Invalid);
        assert_eq!(resolve(&effective, "Q A"), Match::Invalid);
        defaults.push(tool_command("disabled-plugin", "已启用插件"));
        assert_eq!(resolve(&configured(&defaults, &b).0, "Q A"), Match::Run(5));
        b.insert("search".into(), "Z".into());
        b.insert("open:advanced-calculator".into(), "K A".into());
        let (effective, errors) = configured(&defaults, &b);
        assert!(!errors.is_empty());
        assert_eq!(resolve(&effective, "K"), Match::Run(0));
        assert_eq!(resolve(&effective, "K A"), Match::Invalid);
    }
    #[test]
    fn unavailable_tools_are_not_executable_and_record_sequences_are_unambiguous() {
        let mut commands = controls();
        commands.push(tool_command("advanced-calculator", "计算器"));
        assert_eq!(resolve(&commands, "R"), Match::Pending);
        assert_eq!(resolve(&commands, "R E"), Match::Run(3));
        assert_eq!(resolve(&commands, "C"), Match::Run(4));
        assert_eq!(resolve(&commands, "J"), Match::Invalid);
        let mut ids = std::collections::HashSet::new();
        let mut sequences = std::collections::HashSet::new();
        for command in &commands {
            assert!(ids.insert(&command.id));
            assert!(sequences.insert(&command.sequence));
        }
        assert_eq!(tool_command("json", "改名后的 JSON").id, "open:json");
    }
}
