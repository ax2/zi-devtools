use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use parking_lot::{Mutex, RwLock};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::config::{DashboardConfig, ServiceSpec, expand_path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    External,
    PortOpen,
    #[default]
    Stopped,
}

impl ServiceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "运行中",
            Self::External => "外部运行",
            Self::PortOpen => "端口占用",
            Self::Stopped => "已停止",
        }
    }

    pub fn is_available(self) -> bool {
        !matches!(self, Self::Stopped)
    }
}

#[derive(Clone, Debug, Default)]
pub struct HealthStatus {
    pub ok: Option<bool>,
    pub status_code: Option<u16>,
    pub elapsed_ms: Option<u128>,
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ConfigFileSummary {
    pub configured_path: String,
    pub absolute_path: PathBuf,
    pub exists: bool,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct ServiceStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo: PathBuf,
    pub command: String,
    pub graceful_stop_timeout_ms: Option<u64>,
    pub health_url: Option<String>,
    pub port: Option<u16>,
    pub tags: Vec<String>,
    pub state: ServiceState,
    pub pid: Option<u32>,
    pub managed: bool,
    pub port_open: Option<bool>,
    pub health: HealthStatus,
    pub log_path: PathBuf,
    pub log_size: u64,
    pub config_files: Vec<ConfigFileSummary>,
    pub env_count: usize,
}

impl ServiceStatus {
    pub fn display_state(&self) -> &'static str {
        if self.managed && self.health_url.is_some() && self.health.ok == Some(false) {
            "进程运行 · 待健康检查"
        } else {
            self.state.label()
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActionResult {
    pub service_id: String,
    pub message: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DesiredState {
    #[serde(default)]
    service_ids: BTreeSet<String>,
    #[serde(default)]
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PidRecord {
    pid: u32,
    start_time: u64,
}

pub struct ServiceManager {
    config: RwLock<DashboardConfig>,
    children: Mutex<HashMap<String, Child>>,
    desired_running: Mutex<BTreeSet<String>>,
    client: Client,
}

impl ServiceManager {
    pub fn new(config: DashboardConfig) -> Result<Arc<Self>> {
        prepare_state_dirs(&config.state_dir)?;
        let desired = load_desired_running(&config);
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(500))
            .build()
            .context("创建本地健康检查客户端失败")?;
        Ok(Arc::new(Self {
            config: RwLock::new(config),
            children: Mutex::new(HashMap::new()),
            desired_running: Mutex::new(desired),
            client,
        }))
    }

    pub fn config_snapshot(&self) -> DashboardConfig {
        self.config.read().clone()
    }

    pub fn replace_config(&self, config: DashboardConfig) -> Result<()> {
        prepare_state_dirs(&config.state_dir)?;
        let desired = load_desired_running(&config);
        *self.config.write() = config;
        *self.desired_running.lock() = desired;
        Ok(())
    }

    pub fn service_ids(&self) -> Vec<String> {
        self.config.read().services.keys().cloned().collect()
    }

    pub fn service_spec(&self, service_id: &str) -> Result<ServiceSpec> {
        self.config
            .read()
            .services
            .get(service_id)
            .cloned()
            .ok_or_else(|| anyhow!("未知服务: {service_id}"))
    }

    pub fn list_services(&self) -> Vec<ServiceStatus> {
        self.reap_children();
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        self.service_ids()
            .iter()
            .filter_map(|id| self.service_status_with_system(id, &system).ok())
            .collect()
    }

    pub fn service_status(&self, service_id: &str) -> Result<ServiceStatus> {
        self.reap_children();
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        self.service_status_with_system(service_id, &system)
    }

    fn service_status_with_system(
        &self,
        service_id: &str,
        system: &System,
    ) -> Result<ServiceStatus> {
        let spec = self.service_spec(service_id)?;
        let pid = self.validated_pid(service_id, system);
        let process_running = pid.is_some();
        let port_open = spec.port.map(is_port_open);
        let health = if spec.port.is_some() && port_open == Some(false) {
            HealthStatus {
                ok: Some(false),
                message: Some("端口未监听，已跳过健康检查".to_owned()),
                ..Default::default()
            }
        } else {
            self.health(&spec)
        };
        let state = if process_running {
            ServiceState::Running
        } else if health.ok == Some(true) {
            ServiceState::External
        } else if port_open == Some(true) {
            ServiceState::PortOpen
        } else {
            ServiceState::Stopped
        };
        let log_path = self.log_path(service_id);
        let log_size = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let config_files = spec
            .config_files
            .iter()
            .map(|configured| {
                let absolute = resolve_service_file(&spec.repo, configured);
                let metadata = fs::metadata(&absolute).ok();
                ConfigFileSummary {
                    configured_path: configured.clone(),
                    absolute_path: absolute,
                    exists: metadata.is_some(),
                    size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                }
            })
            .collect();

        Ok(ServiceStatus {
            id: spec.id,
            name: spec.name,
            description: spec.description,
            repo: spec.repo,
            command: spec.command,
            graceful_stop_timeout_ms: spec.stop_command.as_ref().map(|_| spec.stop_timeout_ms),
            health_url: spec.health_url,
            port: spec.port,
            tags: spec.tags,
            state,
            pid: process_running.then_some(pid).flatten(),
            managed: process_running,
            port_open,
            health,
            log_path,
            log_size,
            config_files,
            env_count: spec.env.len(),
        })
    }

    pub fn start(&self, service_id: &str) -> Result<ActionResult> {
        let spec = self.service_spec(service_id)?;
        let status = self.service_status(service_id)?;
        if status.state == ServiceState::Running {
            self.set_desired_running(service_id, true)?;
            return Ok(ActionResult {
                service_id: service_id.to_owned(),
                message: "服务已由本程序管理并运行".to_owned(),
            });
        }
        if matches!(
            status.state,
            ServiceState::External | ServiceState::PortOpen
        ) {
            bail!("服务端口已由外部进程占用；未启动，也不会结束该进程");
        }
        if !spec.repo.is_dir() {
            bail!("工作目录不存在: {}", spec.repo.display());
        }

        let log_path = self.log_path(service_id);
        let mut log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("无法打开日志 {}", log_path.display()))?;
        writeln!(
            log_file,
            "\n\n===== Zi DevTools start {} =====\ncwd: {}\ncmd: {}\n",
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            spec.repo.display(),
            spec.command
        )?;
        let stderr_file = log_file.try_clone()?;

        let mut command = Command::new("cmd.exe");
        command
            .current_dir(&spec.repo)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr_file));
        for (key, value) in &spec.env {
            command.env(key, value.as_deref().unwrap_or_default());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command
                .args(["/D", "/S", "/C"])
                .raw_arg(&spec.command)
                .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }
        #[cfg(not(windows))]
        command.args(["/D", "/S", "/C", &spec.command]);

        let child = command
            .spawn()
            .with_context(|| format!("启动服务 {} 失败", spec.name))?;
        let pid = child.id();
        self.children.lock().insert(service_id.to_owned(), child);
        if let Err(error) = self.write_pid(service_id, pid) {
            let _ = terminate_process_tree(pid);
            self.children.lock().remove(service_id);
            let _ = self.clear_pid(service_id);
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(250));
        if self.service_status(service_id)?.state != ServiceState::Running {
            bail!("服务启动后立即退出或脱离托管；请查看服务日志");
        }
        self.set_desired_running(service_id, true)?;
        Ok(ActionResult {
            service_id: service_id.to_owned(),
            message: format!("服务已启动，PID {pid}"),
        })
    }

    pub fn stop(&self, service_id: &str) -> Result<ActionResult> {
        let spec = self.service_spec(service_id)?;
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let pid = self.validated_pid(service_id, &system);
        self.append_log(
            service_id,
            &format!(
                "\n===== Zi DevTools stop {} pid={} =====\n",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                pid.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
            ),
        )?;

        let mut graceful = false;
        if let (Some(pid), Some(stop_command)) = (pid, spec.stop_command.as_deref()) {
            match self.try_graceful_stop(service_id, &spec, stop_command, pid) {
                Ok(stopped) => graceful = stopped,
                Err(error) => {
                    let _ = self.append_log(
                        service_id,
                        &format!("优雅停止命令失败，将回退到强制停止：{error}\n"),
                    );
                }
            }
        }
        let mut forced = false;
        if let Some(pid) = pid
            && !graceful
        {
            let mut refreshed = System::new();
            refreshed.refresh_processes(ProcessesToUpdate::All, true);
            if self.validated_pid(service_id, &refreshed) == Some(pid) {
                forced = terminate_process_tree(pid)?;
                if !forced {
                    let mut final_check = System::new();
                    final_check.refresh_processes(ProcessesToUpdate::All, true);
                    if self.validated_pid(service_id, &final_check) == Some(pid) {
                        bail!("无法停止托管 PID {pid}；已保留 PID 记录以便重试");
                    }
                }
            }
        }
        self.children.lock().remove(service_id);
        self.clear_pid(service_id)?;
        self.set_desired_running(service_id, false)?;
        let port_still_open = spec.port.is_some_and(is_port_open);
        Ok(ActionResult {
            service_id: service_id.to_owned(),
            message: if port_still_open {
                "托管进程已停止；端口仍被占用，外部进程未被结束".to_owned()
            } else if graceful {
                "服务已优雅停止".to_owned()
            } else if forced {
                "服务进程树已强制停止".to_owned()
            } else {
                "没有发现正在运行的托管进程；外部进程未被结束".to_owned()
            },
        })
    }

    fn try_graceful_stop(
        &self,
        service_id: &str,
        spec: &ServiceSpec,
        stop_command: &str,
        pid: u32,
    ) -> Result<bool> {
        let mut system = System::new();
        let target = [Pid::from_u32(pid)];
        // Only identity/liveness is needed here. Refreshing memory and all other
        // processes can consume the entire stop deadline on a busy Windows host.
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&target),
            true,
            ProcessRefreshKind::nothing(),
        );
        if self.validated_pid(service_id, &system) != Some(pid) {
            return Ok(true);
        }
        self.append_log(
            service_id,
            &format!(
                "===== Zi DevTools graceful stop {} timeout={}ms =====\n",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                spec.stop_timeout_ms
            ),
        )?;
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path(service_id))?;
        let stderr_file = log_file.try_clone()?;
        let mut command = Command::new("cmd.exe");
        command
            .current_dir(&spec.repo)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr_file));
        for (key, value) in &spec.env {
            command.env(key, value.as_deref().unwrap_or_default());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command
                .args(["/D", "/S", "/C"])
                .raw_arg(stop_command)
                .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }
        #[cfg(not(windows))]
        command.args(["/D", "/S", "/C", stop_command]);
        let mut helper = command.spawn().context("无法启动优雅停止命令")?;
        let deadline = Instant::now() + Duration::from_millis(spec.stop_timeout_ms);
        loop {
            // Re-open the target so a reused PID cannot retain cached identity.
            let mut system = System::new();
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&target),
                true,
                ProcessRefreshKind::nothing(),
            );
            if self.validated_pid(service_id, &system) != Some(pid) {
                stop_helper(&mut helper);
                self.append_log(service_id, "优雅停止成功\n")?;
                return Ok(true);
            }
            if Instant::now() >= deadline {
                stop_helper(&mut helper);
                self.append_log(service_id, "优雅停止超时，将回退到强制停止\n")?;
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn restart(&self, service_id: &str) -> Result<ActionResult> {
        self.stop(service_id)?;
        std::thread::sleep(Duration::from_millis(250));
        let mut result = self.start(service_id)?;
        result.message = format!("服务已重启；{}", result.message);
        Ok(result)
    }

    pub fn start_all(&self) -> Vec<Result<ActionResult>> {
        self.service_ids().iter().map(|id| self.start(id)).collect()
    }

    pub fn stop_all(&self) -> Vec<Result<ActionResult>> {
        self.service_ids().iter().map(|id| self.stop(id)).collect()
    }

    pub fn restore_running_services(&self) -> Vec<Result<ActionResult>> {
        let ids: Vec<String> = self.desired_running.lock().iter().cloned().collect();
        ids.iter()
            .filter(|id| self.service_spec(id).is_ok())
            .filter_map(|id| match self.service_status(id) {
                Ok(status) if status.state.is_available() => None,
                Ok(_) => Some(self.start(id)),
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn logs(&self, service_id: &str, max_lines: usize) -> Result<String> {
        self.service_spec(service_id)?;
        tail_text(&self.log_path(service_id), max_lines.clamp(1, 5_000))
    }

    pub fn clear_logs(&self, service_id: &str) -> Result<()> {
        self.service_spec(service_id)?;
        File::create(self.log_path(service_id)).context("清空日志失败")?;
        Ok(())
    }

    pub fn read_config_file(&self, service_id: &str, configured_path: &str) -> Result<String> {
        let spec = self.service_spec(service_id)?;
        if !spec.config_files.iter().any(|item| item == configured_path) {
            bail!("文件不在服务 config_files 清单中");
        }
        let path = resolve_service_file(&spec.repo, configured_path);
        read_text_limited(&path, 80_000)
    }

    fn health(&self, spec: &ServiceSpec) -> HealthStatus {
        let Some(url) = spec.health_url.as_deref() else {
            return HealthStatus {
                ok: None,
                message: Some("未配置健康检查".to_owned()),
                ..Default::default()
            };
        };
        let normalized = normalize_local_url(url);
        let started = Instant::now();
        match self.client.get(normalized).send() {
            Ok(response) => HealthStatus {
                ok: Some(response.status().as_u16() < 500),
                status_code: Some(response.status().as_u16()),
                elapsed_ms: Some(started.elapsed().as_millis()),
                message: None,
            },
            Err(error) => HealthStatus {
                ok: Some(false),
                message: Some(error.to_string()),
                ..Default::default()
            },
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.config.read().state_dir.clone()
    }

    fn log_path(&self, service_id: &str) -> PathBuf {
        self.state_dir()
            .join("logs")
            .join(format!("{}.log", safe_id(service_id)))
    }

    fn pid_path(&self, service_id: &str) -> PathBuf {
        self.state_dir()
            .join("pids")
            .join(format!("{}.pid", safe_id(service_id)))
    }

    fn desired_state_path(&self) -> PathBuf {
        self.state_dir().join("desired-running.json")
    }

    fn validated_pid(&self, service_id: &str, system: &System) -> Option<u32> {
        let record: PidRecord =
            serde_json::from_str(&fs::read_to_string(self.pid_path(service_id)).ok()?).ok()?;
        let process = system.process(Pid::from_u32(record.pid))?;
        (record.start_time > 0 && process.start_time() == record.start_time).then_some(record.pid)
    }

    fn write_pid(&self, service_id: &str, pid: u32) -> Result<()> {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let start_time = system
            .process(Pid::from_u32(pid))
            .map(|process| process.start_time())
            .filter(|time| *time > 0)
            .context("无法确认新进程的启动时间，拒绝写入不安全的 PID 记录")?;
        let record = PidRecord { pid, start_time };
        fs::write(self.pid_path(service_id), serde_json::to_vec(&record)?)
            .context("写入 PID 文件失败")
    }

    fn clear_pid(&self, service_id: &str) -> Result<()> {
        let path = self.pid_path(service_id);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("删除 PID 文件失败"),
        }
    }

    fn set_desired_running(&self, service_id: &str, running: bool) -> Result<()> {
        let mut desired = self.desired_running.lock();
        if running {
            desired.insert(service_id.to_owned());
        } else {
            desired.remove(service_id);
        }
        let state = DesiredState {
            service_ids: desired.clone(),
            updated_at: Local::now().to_rfc3339(),
        };
        let target = self.desired_state_path();
        let temporary = target.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&state)?)?;
        fs::rename(&temporary, &target).or_else(|_| {
            let _ = fs::remove_file(&target);
            fs::rename(&temporary, &target)
        })?;
        Ok(())
    }

    fn append_log(&self, service_id: &str, text: &str) -> Result<()> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path(service_id))?
            .write_all(text.as_bytes())?;
        Ok(())
    }

    fn reap_children(&self) {
        let exited: Vec<(String, std::process::ExitStatus)> = {
            let mut children = self.children.lock();
            children
                .iter_mut()
                .filter_map(|(id, child)| match child.try_wait() {
                    Ok(Some(status)) => Some((id.clone(), status)),
                    _ => None,
                })
                .collect()
        };
        if exited.is_empty() {
            return;
        }
        let mut children = self.children.lock();
        for (id, status) in exited {
            children.remove(&id);
            let _ = self.clear_pid(&id);
            let _ = self.append_log(
                &id,
                &format!(
                    "===== Zi DevTools process exited {} status={} =====\n",
                    Local::now().format("%Y-%m-%d %H:%M:%S"),
                    status
                ),
            );
        }
    }
}

fn prepare_state_dirs(state_dir: &Path) -> Result<()> {
    fs::create_dir_all(state_dir.join("logs"))?;
    fs::create_dir_all(state_dir.join("pids"))?;
    Ok(())
}

fn load_desired_running(config: &DashboardConfig) -> BTreeSet<String> {
    let path = config.state_dir.join("desired-running.json");
    if let Ok(text) = fs::read_to_string(path)
        && let Ok(state) = serde_json::from_str::<DesiredState>(&text)
    {
        return state.service_ids;
    }
    BTreeSet::new()
}

fn is_port_open(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&address, Duration::from_millis(80)).is_ok()
}

fn terminate_process_tree(pid: u32) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("无法调用 taskkill 停止 PID {pid}"))?;
    Ok(status.success())
}

fn stop_helper(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        if terminate_process_tree(child.id()).ok() != Some(true) {
            let _ = child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn resolve_service_file(repo: &Path, value: &str) -> PathBuf {
    let path = expand_path(Path::new(value));
    if path.is_absolute() {
        path
    } else {
        repo.join(path)
    }
}

fn normalize_local_url(url: &str) -> String {
    url.strip_prefix("http://localhost")
        .map(|rest| format!("http://127.0.0.1{rest}"))
        .or_else(|| {
            url.strip_prefix("https://localhost")
                .map(|rest| format!("https://127.0.0.1{rest}"))
        })
        .unwrap_or_else(|| url.to_owned())
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn read_text_limited(path: &Path, max_bytes: usize) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("读取 {} 失败", path.display()))?;
    let truncated = bytes.len() > max_bytes;
    let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(max_bytes)]).into_owned();
    if truncated {
        text.push_str(&format!("\n\n... 已截断，仅显示前 {max_bytes} 字节 ..."));
    }
    Ok(text)
}

fn tail_text(path: &Path, max_lines: usize) -> Result<String> {
    const MAX_PREVIEW_BYTES: u64 = 4 * 1024 * 1024;
    let Ok(mut file) = File::open(path) else {
        return Ok(String::new());
    };
    let size = file.metadata()?.len();
    let truncated_bytes = size.saturating_sub(MAX_PREVIEW_BYTES);
    file.seek(SeekFrom::Start(truncated_bytes))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let start = if truncated_bytes > 0 {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |index| index + 1)
    } else {
        0
    };
    let text = String::from_utf8_lossy(&bytes[start..]);
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.len().saturating_sub(max_lines);
    let mut preview = lines[first..].join("\n");
    if truncated_bytes > 0 || first > 0 {
        preview.insert_str(
            0,
            "[日志预览仅显示末尾内容；可点击“打开完整日志文件”查看原始文件]\n\n",
        );
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_ids_and_local_urls_match_legacy_contract() {
        assert_eq!(safe_id("hello/world x"), "hello_world_x");
        assert_eq!(
            normalize_local_url("http://localhost:8080/health"),
            "http://127.0.0.1:8080/health"
        );
    }

    #[test]
    fn reads_only_the_requested_log_tail() {
        let root = std::env::temp_dir().join(format!("zi-tail-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("demo.log");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        assert_eq!(tail_text(&path, 3).unwrap(), "one\ntwo\nthree");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn marks_log_preview_when_earlier_lines_are_omitted() {
        let root = std::env::temp_dir().join(format!("zi-tail-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("demo.log");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let preview = tail_text(&path, 2).unwrap();
        assert!(preview.contains("预览仅显示末尾内容"));
        assert!(preview.ends_with("two\nthree"));
        fs::remove_dir_all(root).unwrap();
    }
}
