//! Shared command identities, kept independent of Windows hooks and UI focus.
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
