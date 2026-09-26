use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ServiceSpec {
    #[serde(skip)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub repo: PathBuf,
    pub command: String,
    #[serde(default)]
    pub stop_command: Option<String>,
    #[serde(default = "default_stop_timeout_ms")]
    pub stop_timeout_ms: u64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub health_url: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub config_files: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DashboardConfig {
    pub path: PathBuf,
    pub state_dir: PathBuf,
    pub services: BTreeMap<String, ServiceSpec>,
    pub modified: Option<SystemTime>,
}

fn default_stop_timeout_ms() -> u64 {
    5_000
}

#[derive(Debug, Deserialize, Serialize)]
struct RawConfig {
    #[serde(default)]
    state_dir: Option<PathBuf>,
    services: BTreeMap<String, ServiceSpec>,
}

pub fn import_config(source: &Path, destination: &Path, force: bool) -> Result<PathBuf> {
    let source = expand_path(source);
    let destination = expand_path(destination);
    if source == destination {
        bail!("源配置和目标配置不能是同一个文件");
    }
    if destination.exists() && !force {
        bail!("目标配置已存在: {}", destination.display());
    }

    let text = fs::read_to_string(&source)
        .with_context(|| format!("无法读取待导入配置 {}", source.display()))?;
    let mut raw: RawConfig = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("待导入 YAML 无效 {}", source.display()))?;
    if raw.services.is_empty() {
        bail!("待导入配置必须包含非空 services 映射");
    }
    for (id, service) in &raw.services {
        if service.command.trim().is_empty() || service.repo.as_os_str().is_empty() {
            bail!("服务 {id} 缺少 repo 或 command");
        }
        validate_stop_options(id, service)?;
    }
    raw.state_dir = Some(default_state_dir());

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("目标配置缺少父目录"))?;
    fs::create_dir_all(parent)?;
    let temporary = destination.with_extension("yml.tmp");
    fs::write(&temporary, serde_yaml_ng::to_string(&raw)?)?;
    load_config(&temporary).context("导入后的配置校验失败")?;
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    fs::rename(&temporary, &destination)?;
    Ok(destination)
}

pub fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zi-devtools")
        .join("services.yml")
}

pub fn default_state_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zi-devtools")
        .join("services-state")
}

pub fn load_config(path: &Path) -> Result<DashboardConfig> {
    let path = expand_path(path);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("无法读取配置文件 {}", path.display()))?;
    let mut raw: RawConfig = serde_yaml_ng::from_str(&text)
        .with_context(|| format!("YAML 配置无效 {}", path.display()))?;
    if raw.services.is_empty() {
        bail!("配置必须包含非空 services 映射");
    }

    for (id, service) in &mut raw.services {
        service.id.clone_from(id);
        if service.name.trim().is_empty() {
            service.name.clone_from(id);
        }
        if service.command.trim().is_empty() {
            bail!("服务 {id} 缺少 command");
        }
        validate_stop_options(id, service)?;
        service.repo = expand_path(&service.repo);
        for text in service.env.values_mut().flatten() {
            *text = expand_leading_home(text);
        }
    }

    let state_dir = raw
        .state_dir
        .as_deref()
        .map(expand_path)
        .unwrap_or_else(default_state_dir);
    let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
    Ok(DashboardConfig {
        path,
        state_dir,
        services: raw.services,
        modified,
    })
}

fn validate_stop_options(id: &str, service: &ServiceSpec) -> Result<()> {
    if service
        .stop_command
        .as_ref()
        .is_some_and(|command| command.trim().is_empty())
    {
        bail!("服务 {id} 的 stop_command 不能为空");
    }
    if !(100..=60_000).contains(&service.stop_timeout_ms) {
        bail!("服务 {id} 的 stop_timeout_ms 必须为 100 到 60000 毫秒");
    }
    Ok(())
}

pub fn expand_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(expand_leading_home(&text))
}

fn expand_leading_home(value: &str) -> String {
    if value == "~" {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_service_and_normalizes_null_env() {
        let root = std::env::temp_dir().join(format!("zi-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("repo")).unwrap();
        let path = root.join("services.yml");
        fs::write(
            &path,
            format!(
                "state_dir: {}\nservices:\n  demo:\n    repo: {}\n    command: python -m http.server\n    env:\n      EMPTY:\n      VALUE: ok\n",
                root.join("state").display(),
                root.join("repo").display()
            ),
        )
        .unwrap();

        let config = load_config(&path).unwrap();
        let demo = &config.services["demo"];
        assert_eq!(demo.name, "demo");
        assert_eq!(demo.env["EMPTY"], None);
        assert_eq!(demo.env["VALUE"].as_deref(), Some("ok"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn imports_services_into_independent_state_directory() {
        let root = std::env::temp_dir().join(format!("zi-import-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("repo")).unwrap();
        let source = root.join("source.yml");
        let destination = root.join("nested").join("services.yml");
        fs::write(
            &source,
            format!(
                "state_dir: C:/legacy/state\nservices:\n  demo:\n    repo: {}\n    command: python -m http.server 8800\n",
                root.join("repo").display()
            ),
        )
        .unwrap();

        let imported = import_config(&source, &destination, false).unwrap();
        assert_eq!(imported, destination);
        let config = load_config(&destination).unwrap();
        assert!(config.services.contains_key("demo"));
        assert_eq!(config.state_dir, default_state_dir());
        assert!(import_config(&source, &destination, false).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_graceful_stop_options() {
        let root = std::env::temp_dir().join(format!("zi-stop-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("services.yml");
        fs::write(
            &path,
            format!(
                "services:\n  demo:\n    repo: {}\n    command: echo start\n    stop_command: '   '\n",
                root.display()
            ),
        )
        .unwrap();
        assert!(load_config(&path).is_err());
        fs::write(
            &path,
            format!(
                "services:\n  demo:\n    repo: {}\n    command: echo start\n    stop_command: echo stop\n    stop_timeout_ms: 0\n",
                root.display()
            ),
        )
        .unwrap();
        assert!(load_config(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
