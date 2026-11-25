//! Shared literal formatting and validation for the ZiDevTools MCP client snippets.
use serde_json::json;

pub const MAX_PATH_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exports {
    pub codex_command: String,
    pub codex_toml: String,
    pub generic_json: String,
}

pub fn contains_path_control(path: &str) -> bool {
    path.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            )
    })
}

pub fn render(raw: &str) -> serde_json::Result<Exports> {
    let powershell_literal = raw.replace('\'', "''");
    let codex_command = format!("codex mcp add zi-knowledge -- '{powershell_literal}'");
    let toml_path = if raw.contains('\'') {
        serde_json::to_string(raw)?
    } else {
        format!("'{raw}'")
    };
    let codex_toml = format!(
        "[mcp_servers.zi-knowledge]\ncommand = {toml_path}\nenabled_tools = [\"list_knowledge_sources\", \"search_knowledge\"]\ndefault_tools_approval_mode = \"writes\"\n"
    );
    let generic_json = serde_json::to_string_pretty(&json!({
        "mcpServers": {"zi-knowledge": {"command": raw, "args": []}}
    }))?;
    Ok(Exports {
        codex_command,
        codex_toml,
        generic_json,
    })
}

fn allows_windows_component(component: &str) -> bool {
    if component.is_empty()
        || component.ends_with([' ', '.'])
        || component.contains(['<', '>', ':', '"', '|', '?', '*'])
        || component.encode_utf16().count() > 255
    {
        return false;
    }
    let stem = component.split('.').next().unwrap_or("").trim_end();
    let device = stem.to_ascii_uppercase();
    !matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].into_iter().any(|prefix| {
        device.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// Validate the explicitly typed Windows path. The WASI plugin has no access to
/// the file, drive mapping, or client settings associated with this literal.
pub fn valid_manual_path(path: &str) -> bool {
    if path.len() > MAX_PATH_BYTES || contains_path_control(path) {
        return false;
    }
    let [drive, b':', b'\\' | b'/', ..] = path.as_bytes() else {
        return false;
    };
    if !drive.is_ascii_alphabetic() {
        return false;
    }

    let mut components = path[3..].split(['\\', '/']);
    let Some(file_name) = components.next_back() else {
        return false;
    };
    if !allows_windows_component(file_name) || !components.all(allows_windows_component) {
        return false;
    }

    let name = file_name.to_ascii_lowercase();
    name == "zidevtoolsmcp.exe"
        || name
            .strip_prefix("zidevtoolsmcp-")
            .and_then(|version| version.strip_suffix("-windows-x64.exe"))
            .is_some_and(|version| semver::Version::parse(version).is_ok())
}
