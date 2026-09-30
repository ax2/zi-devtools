//! Durable, bounded grants for manually selected MCP stdio tools.
//! The selected server remains trusted code; annotations are untrusted claims.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::mcp::Config;

const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_STORE: u64 = 64 * 1024;
const MAX_RULES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    Deny,
    AllowDeclaredReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Deny,
    Confirm,
    Direct,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    tool_digest: String,
    rule: Rule,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    rules: BTreeMap<String, Entry>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            rules: BTreeMap::new(),
        }
    }
}

pub struct Store {
    path: PathBuf,
    data: Document,
    pub error: Option<String>,
}

fn hash_file(path: &Path, digest: &mut Sha256) -> Result<()> {
    let mut file =
        fs::File::open(path).with_context(|| format!("无法读取 MCP 程序：{}", path.display()))?;
    let length = file.metadata()?.len();
    ensure!(length <= MAX_FILE, "MCP 程序或参数文件过大，无法保存权限");
    digest.update(length.to_le_bytes());
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(())
}

/// Path and arguments are hashed, not stored. Direct file arguments are also
/// hashed so editing a script invalidates a grant tied to its interpreter.
pub fn server_scope(config: &Config) -> Result<String> {
    let executable = config.validate()?;
    let mut digest = Sha256::new();
    digest.update(b"zi-mcp-server-v1\0");
    digest.update(executable.to_string_lossy().as_bytes());
    digest.update([0]);
    hash_file(&executable, &mut digest)?;
    for arg in &config.args {
        digest.update((arg.len() as u64).to_le_bytes());
        digest.update(arg.as_bytes());
        let candidate = Path::new(arg);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            executable
                .parent()
                .unwrap_or(Path::new("."))
                .join(candidate)
        };
        if candidate.is_file() {
            let resolved = candidate.canonicalize()?;
            digest.update(b"\0file\0");
            digest.update(resolved.to_string_lossy().as_bytes());
            digest.update([0]);
            hash_file(&resolved, &mut digest)?;
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn rule_key(scope: &str, tool: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{scope}\0{tool}").as_bytes())
    )
}

fn tool_digest(tool: &Value) -> Result<String> {
    ensure!(
        tool.get("name").and_then(Value::as_str).is_some(),
        "MCP 工具定义缺少名称"
    );
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(tool)?)))
}

pub fn declared_low_impact(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true))
        && tool.pointer("/annotations/destructiveHint") == Some(&Value::Bool(false))
        && tool.pointer("/annotations/openWorldHint") == Some(&Value::Bool(false))
}

fn validate_document(data: &Document) -> Result<()> {
    ensure!(data.version == 1, "MCP 权限文件版本不受支持");
    ensure!(data.rules.len() <= MAX_RULES, "MCP 权限规则过多");
    for (key, entry) in &data.rules {
        ensure!(
            key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "MCP 权限规则标识无效"
        );
        ensure!(
            entry.tool_digest.len() == 64
                && entry
                    .tool_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "MCP 工具定义摘要无效"
        );
    }
    Ok(())
}

fn read_document(path: &Path) -> Result<Document> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STORE + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_STORE, "MCP 权限文件过大");
    let data: Document = serde_json::from_slice(&bytes).context("MCP 权限文件不是有效 JSON")?;
    validate_document(&data)?;
    Ok(data)
}

fn decision(data: &Document, scope: &str, tool: &Value) -> Result<Decision> {
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .context("MCP 工具定义缺少名称")?;
    let Some(entry) = data.rules.get(&rule_key(scope, name)) else {
        return Ok(Decision::Confirm);
    };
    if entry.rule == Rule::Deny {
        return Ok(Decision::Deny);
    }
    if entry.tool_digest == tool_digest(tool)? && declared_low_impact(tool) {
        Ok(Decision::Direct)
    } else {
        Ok(Decision::Confirm)
    }
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        match read_document(&path) {
            Ok(data) => Self {
                path,
                data,
                error: None,
            },
            Err(_) => Self {
                path,
                data: Document::default(),
                error: Some("MCP 权限文件读取失败：已禁止调用及保存；请修复原文件后重启。".into()),
            },
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn decision(&self, scope: &str, tool: &Value) -> Result<Decision> {
        ensure!(self.error.is_none(), "MCP 权限文件异常，调用已拒绝");
        decision(&self.data, scope, tool)
    }

    pub fn set(&mut self, scope: &str, tool: &Value, rule: Option<Rule>) -> Result<()> {
        ensure!(self.error.is_none(), "MCP 权限文件异常，未覆盖原文件");
        ensure!(
            scope.len() == 64 && scope.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "MCP 服务身份无效"
        );
        if rule == Some(Rule::AllowDeclaredReadOnly) {
            ensure!(
                declared_low_impact(tool),
                "该工具未声明完整的低影响只读行为"
            );
        }
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .context("MCP 工具定义缺少名称")?;
        let key = rule_key(scope, name);
        // A second process may have changed or corrupted the file since load.
        // Re-read it before writing so stale UI state cannot overwrite it.
        let mut next = read_document(&self.path)?;
        if let Some(rule) = rule {
            next.rules.insert(
                key,
                Entry {
                    tool_digest: tool_digest(tool)?,
                    rule,
                },
            );
        } else {
            next.rules.remove(&key);
        }
        validate_document(&next)?;
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(bytes.len() as u64 <= MAX_STORE, "MCP 权限文件过大");
        let parent = self.path.parent().context("MCP 权限路径无效")?;
        fs::create_dir_all(parent)?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let write_result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result.context("MCP 权限保存失败")?;
        self.data = next;
        Ok(())
    }
}

/// Re-read immediately before sending tools/call, so stale in-memory grants
/// cannot survive a revoke or an edited permission file.
pub fn authorize(path: &Path, scope: &str, tool: &Value, manual_confirmed: bool) -> Result<()> {
    match decision(&read_document(path)?, scope, tool)? {
        Decision::Deny => bail!("当前 MCP 服务的该工具已禁止调用"),
        Decision::Direct => Ok(()),
        Decision::Confirm if manual_confirmed => Ok(()),
        Decision::Confirm => bail!("MCP 权限已变化，需要重新人工确认"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_scope_changes_with_program_script_and_arguments() {
        let root = std::env::temp_dir().join(format!("zi-mcp-scope-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("fixture.exe");
        let script = root.join("server.py");
        fs::write(&executable, b"program-one").unwrap();
        fs::write(&script, b"script-one").unwrap();
        let mut config = Config {
            executable: executable.clone(),
            args: vec![script.to_string_lossy().into_owned()],
        };
        let initial = server_scope(&config).unwrap();
        fs::write(&script, b"script-two").unwrap();
        let script_changed = server_scope(&config).unwrap();
        assert_ne!(initial, script_changed);
        fs::write(&executable, b"program-two").unwrap();
        let program_changed = server_scope(&config).unwrap();
        assert_ne!(script_changed, program_changed);
        config.args.push("--read-only".into());
        assert_ne!(program_changed, server_scope(&config).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
