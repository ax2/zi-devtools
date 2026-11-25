//! Windows executable names recognized by the native integration tool.

pub const WINDOWS_ENTRIES: [(&str, &[&str]); 13] = [
    ("Ollama", &["ollama.exe"]),
    ("Codex CLI", &["codex.exe", "codex.cmd", "codex.ps1"]),
    ("Claude CLI", &["claude.exe", "claude.cmd"]),
    ("Python", &["python.exe"]),
    ("Java", &["java.exe"]),
    ("Java 编译器", &["javac.exe"]),
    ("Maven", &["mvn.cmd", "mvn.exe"]),
    ("Gradle", &["gradle.bat", "gradle.exe"]),
    ("Django CLI", &["django-admin.exe", "django-admin.cmd"]),
    ("Node.js", &["node.exe"]),
    ("uv", &["uv.exe"]),
    ("Docker", &["docker.exe"]),
    ("Git", &["git.exe"]),
];

/// Returns the known names only, in the native tool's presentation order.
pub fn match_file_names<'a>(names: impl IntoIterator<Item = &'a str>) -> [Vec<&'static str>; 13] {
    let mut index = Vec::new();
    let mut shortest = usize::MAX;
    let mut longest = 0;
    for (group, (_, alternatives)) in WINDOWS_ENTRIES.iter().enumerate() {
        for (position, candidate) in alternatives.iter().enumerate() {
            shortest = shortest.min(candidate.len());
            longest = longest.max(candidate.len());
            index.push((candidate.as_bytes(), group, 1_u8 << position));
        }
    }
    index.sort_unstable_by_key(|&(name, _, _)| name);

    let mut matches = [0_u8; WINDOWS_ENTRIES.len()];
    let mut normalized = Vec::with_capacity(longest);
    for name in names {
        if !(shortest..=longest).contains(&name.len()) {
            continue;
        }
        normalized.clear();
        normalized.extend(name.bytes().map(|byte| byte.to_ascii_lowercase()));
        if let Ok(position) =
            index.binary_search_by_key(&normalized.as_slice(), |&(candidate, _, _)| candidate)
        {
            let (_, group, flag) = index[position];
            matches[group] |= flag;
        }
    }
    std::array::from_fn(|index| {
        WINDOWS_ENTRIES[index]
            .1
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(alias, candidate)| {
                (matches[index] & (1 << alias) != 0).then_some(candidate)
            })
            .collect()
    })
}
