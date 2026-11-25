//! Shared ZiDevTools service YAML and offline import checks.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

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

fn default_stop_timeout_ms() -> u64 {
    5_000
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RawConfig {
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    pub services: BTreeMap<String, ServiceSpec>,
}

pub fn parse(input: &str) -> Result<RawConfig, serde_yaml_ng::Error> {
    serde_yaml_ng::from_str(input.strip_prefix('\u{feff}').unwrap_or(input))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportProblem {
    MissingRepoOrCommand,
    EmptyStopCommand,
    InvalidStopTimeout,
}

pub fn import_problem(service: &ServiceSpec) -> Option<ImportProblem> {
    if service.command.trim().is_empty() || service.repo.as_os_str().is_empty() {
        return Some(ImportProblem::MissingRepoOrCommand);
    }
    stop_problem(service)
}

pub fn stop_problem(service: &ServiceSpec) -> Option<ImportProblem> {
    if service
        .stop_command
        .as_ref()
        .is_some_and(|command| command.trim().is_empty())
    {
        return Some(ImportProblem::EmptyStopCommand);
    }
    if !(100..=60_000).contains(&service.stop_timeout_ms) {
        return Some(ImportProblem::InvalidStopTimeout);
    }
    None
}
