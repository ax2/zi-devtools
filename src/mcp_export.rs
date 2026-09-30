//! Local, credential-free snippets for connecting trusted MCP clients.
use anyhow::{Result, ensure};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exports {
    pub codex_command: String,
    pub codex_toml: String,
    pub generic_json: String,
}

pub fn sibling_server() -> Result<PathBuf> {
    let app = std::env::current_exe()?;
    let installed = app.with_file_name("ZiDevToolsMcp.exe");
    let candidate = if installed.is_file() {
        installed
    } else {
        app.with_file_name(format!(
            "ZiDevToolsMcp-{}-windows-x64.exe",
            env!("CARGO_PKG_VERSION")
        ))
    };
    ensure!(candidate.is_file(), "当前程序目录未找到 MCP 服务 EXE");
    Ok(candidate)
}

pub fn generate(executable: &Path) -> Result<Exports> {
    ensure!(executable.is_absolute(), "MCP 程序必须使用绝对路径");
    let meta = std::fs::symlink_metadata(executable)?;
    ensure!(
        meta.file_type().is_file()
            && executable
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")),
        "MCP 程序必须是普通 EXE 文件，不能是链接"
    );
    let raw = executable.to_string_lossy();
    ensure!(
        !raw.chars().any(char::is_control),
        "MCP 程序路径包含控制字符"
    );
    let powershell_literal = raw.replace('\'', "''");
    let codex_command = format!("codex mcp add zi-knowledge -- '{powershell_literal}'");
    let toml_path = if raw.contains('\'') {
        serde_json::to_string(raw.as_ref())?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_preserve_windows_path_without_credentials() {
        let root = std::env::temp_dir().join(format!("zi-mcp-export-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("Zi DevTools's MCP.exe");
        std::fs::write(&path, b"fixture").unwrap();
        let output = generate(&path).unwrap();
        assert!(output.codex_command.contains("DevTools''s MCP.exe"));
        assert!(
            output
                .codex_toml
                .starts_with("[mcp_servers.zi-knowledge]\ncommand = \"")
        );
        assert!(
            output
                .codex_toml
                .contains("default_tools_approval_mode = \"writes\"")
        );
        assert!(
            output
                .codex_toml
                .contains("enabled_tools = [\"list_knowledge_sources\", \"search_knowledge\"]")
        );
        let parsed: serde_json::Value = serde_json::from_str(&output.generic_json).unwrap();
        assert_eq!(
            parsed.pointer("/mcpServers/zi-knowledge/command"),
            Some(&json!(path.to_string_lossy()))
        );
        assert_eq!(
            parsed.pointer("/mcpServers/zi-knowledge/args"),
            Some(&json!([]))
        );
        assert!(!output.codex_toml.contains("token"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
