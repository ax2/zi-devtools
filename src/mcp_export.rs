//! Local, credential-free snippets for connecting trusted MCP clients.
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
pub use zi_mcp_export_core::Exports;

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
    Ok(zi_mcp_export_core::render(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
