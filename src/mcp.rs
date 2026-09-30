//! A bounded, explicit MCP stdio inspector. Each operation owns one short-lived session.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

const PROTOCOL: &str = "2025-06-18";
const MAX_FRAME: usize = 1024 * 1024;
const MAX_SESSION_BYTES: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 128;
const MAX_PAGES: usize = 8;

#[derive(Clone, Debug)]
pub struct Config {
    pub executable: PathBuf,
    pub args: Vec<String>,
}

impl Config {
    pub fn validate(&self) -> Result<PathBuf> {
        ensure!(
            self.executable.is_absolute() && self.executable.is_file(),
            "请选择存在的绝对可执行文件路径"
        );
        #[cfg(windows)]
        ensure!(
            self.executable
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")),
            "MCP stdio 仅接受 EXE，不通过 shell 执行脚本"
        );
        ensure!(self.args.len() <= 12, "参数最多 12 项");
        ensure!(
            self.args
                .iter()
                .all(|arg| arg.len() <= 1024 && !arg.contains('\0'))
                && self.args.iter().map(String::len).sum::<usize>() <= 4096,
            "参数长度超出限制或含 NUL"
        );
        self.executable
            .canonicalize()
            .context("无法解析可执行文件路径")
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Inspect,
    Call {
        tool: String,
        arguments: Value,
        expected_tool: Value,
    },
    ReadResource {
        uri: String,
    },
    GetPrompt {
        name: String,
        arguments: Value,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub server: String,
    pub protocol: String,
    pub tools: Vec<Value>,
    pub resources: Vec<Value>,
    pub prompts: Vec<Value>,
    pub call_result: Option<Value>,
    pub resource_result: Option<(String, Value)>,
    pub prompt_result: Option<(String, Value)>,
}

enum Frame {
    Line(String),
    Error(String),
    Eof,
}

struct Outbound {
    payload: Vec<u8>,
    acknowledgement: mpsc::Sender<std::result::Result<(), String>>,
}

struct Process {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if !self.job.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(config: &Config) -> Result<Process> {
    let executable = config.validate()?;
    let mut command = Command::new(&executable);
    command
        .args(&config.args)
        .current_dir(executable.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Do not hand the application environment's API keys to a selected server.
    command.env_clear();
    for name in [
        "SystemRoot",
        "WINDIR",
        "PATH",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "HOME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = command.spawn().context("无法启动 MCP 服务进程")?;
    #[cfg(windows)]
    let mut process = Process {
        child,
        job: std::ptr::null_mut(),
    };
    #[cfg(not(windows))]
    let process = Process { child };
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        process.job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        ensure!(!process.job.is_null(), "无法创建 MCP 进程 Job");
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        ensure!(
            SetInformationJobObject(
                process.job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32
            ) != 0,
            "无法设置 MCP 进程限制"
        );
        ensure!(
            AssignProcessToJobObject(process.job, process.child.as_raw_handle()) != 0,
            "无法将 MCP 进程加入 Job"
        );
    }
    Ok(process)
}

fn reader(mut stdout: impl Read, sender: mpsc::SyncSender<Frame>) {
    let mut buffer = [0u8; 4096];
    let mut line = Vec::new();
    loop {
        let count = match stdout.read(&mut buffer) {
            Ok(count) => count,
            Err(_) => {
                let _ = sender.send(Frame::Error("MCP stdout 读取失败".into()));
                return;
            }
        };
        if count == 0 {
            let _ = sender.send(Frame::Eof);
            return;
        }
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                let text = match String::from_utf8(std::mem::take(&mut line)) {
                    Ok(text) => text,
                    Err(_) => {
                        let _ = sender.send(Frame::Error("MCP stdout 不是 UTF-8".into()));
                        return;
                    }
                };
                if sender.send(Frame::Line(text)).is_err() {
                    return;
                }
            } else {
                line.push(*byte);
                if line.len() > MAX_FRAME {
                    let _ = sender.send(Frame::Error("MCP 单条消息超过 1 MiB".into()));
                    return;
                }
            }
        }
    }
}

struct Session {
    _process: Process,
    receiver: Receiver<Frame>,
    writer: mpsc::SyncSender<Outbound>,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    next_id: u64,
    received_bytes: usize,
}

impl Session {
    fn new(config: &Config, timeout: Duration, cancelled: Arc<AtomicBool>) -> Result<Self> {
        let mut process = spawn(config)?;
        let stdout = process.child.stdout.take().context("MCP stdout 不可用")?;
        let mut stdin = process.child.stdin.take().context("MCP stdin 不可用")?;
        let (sender, receiver) = mpsc::sync_channel(16);
        std::thread::spawn(move || reader(stdout, sender));
        let (writer, outbound) = mpsc::sync_channel::<Outbound>(1);
        std::thread::spawn(move || {
            while let Ok(message) = outbound.recv() {
                let result = stdin
                    .write_all(&message.payload)
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
                    .map_err(|_| "MCP 请求写入失败".to_owned());
                let failed = result.is_err();
                let _ = message.acknowledgement.send(result);
                if failed {
                    break;
                }
            }
        });
        Ok(Self {
            _process: process,
            receiver,
            writer,
            deadline: Instant::now() + timeout,
            cancelled,
            next_id: 1,
            received_bytes: 0,
        })
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        let payload = serde_json::to_vec(message)?;
        ensure!(payload.len() <= 256 * 1024, "MCP 请求超过 256 KiB");
        let (acknowledgement, receipt) = mpsc::channel();
        self.writer
            .send(Outbound {
                payload,
                acknowledgement,
            })
            .context("MCP 请求队列已关闭")?;
        loop {
            ensure!(
                !self.cancelled.load(Ordering::Relaxed),
                "已停止 MCP 操作并结束服务进程"
            );
            ensure!(
                Instant::now() < self.deadline,
                "MCP 操作超时，服务进程已结束"
            );
            match receipt.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(message)) => bail!("{message}"),
                Err(RecvTimeoutError::Disconnected) => bail!("MCP 请求写入线程已退出"),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        loop {
            ensure!(
                !self.cancelled.load(Ordering::Relaxed),
                "已停止 MCP 操作并结束服务进程"
            );
            ensure!(
                Instant::now() < self.deadline,
                "MCP 操作超时，服务进程已结束"
            );
            match self.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(Frame::Line(line)) => {
                    self.received_bytes = self.received_bytes.saturating_add(line.len());
                    ensure!(
                        self.received_bytes <= MAX_SESSION_BYTES,
                        "MCP 本次会话输出超过 4 MiB"
                    );
                    let message: Value =
                        serde_json::from_str(&line).context("MCP stdout 含非 JSON-RPC 消息")?;
                    ensure!(
                        message.get("jsonrpc").and_then(Value::as_str) == Some("2.0"),
                        "MCP 响应不是 JSON-RPC 2.0"
                    );
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        // Server notifications and progress may arrive between responses.
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                        let detail = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("未知错误")
                            .chars()
                            .take(300)
                            .collect::<String>();
                        bail!("MCP {method} 失败（{code}）：{detail}");
                    }
                    return message
                        .get("result")
                        .cloned()
                        .context("MCP 响应缺少 result");
                }
                Ok(Frame::Error(message)) => bail!("{message}"),
                Ok(Frame::Eof) => bail!("MCP 服务在响应前退出"),
                Err(RecvTimeoutError::Disconnected) => bail!("MCP stdout 已断开"),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn initialize(&mut self) -> Result<(String, String, Value)> {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": {"name":"zi-devtools","version":env!("CARGO_PKG_VERSION")}
            }),
        )?;
        let version = response
            .get("protocolVersion")
            .and_then(Value::as_str)
            .context("MCP 服务未返回协议版本")?;
        ensure!(
            ["2024-11-05", "2025-03-26", PROTOCOL].contains(&version),
            "服务选择的 MCP 协议版本不受支持：{version}"
        );
        let name = response
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .context("MCP 服务未返回名称")?
            .chars()
            .take(120)
            .collect::<String>();
        let capabilities = response
            .get("capabilities")
            .filter(|value| value.is_object())
            .context("MCP 服务未返回 capabilities")?
            .clone();
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        Ok((name, version.to_owned(), capabilities))
    }

    fn list(&mut self, method: &str, field: &str) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for page in 0..MAX_PAGES {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor":c}));
            let response = self.request(method, params)?;
            let page_items = response
                .get(field)
                .and_then(Value::as_array)
                .context(format!("MCP {method} 缺少 {field} 列表"))?;
            ensure!(
                items.len() + page_items.len() <= MAX_ITEMS,
                "MCP {field} 超过 128 项"
            );
            ensure!(
                page_items.iter().all(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| !name.is_empty() && name.len() <= 128)
                }),
                "MCP {field} 列表包含无效名称"
            );
            if field == "resources" {
                ensure!(
                    page_items.iter().all(|item| {
                        item.get("uri")
                            .and_then(Value::as_str)
                            .is_some_and(|uri| !uri.is_empty() && uri.len() <= 2048)
                    }),
                    "MCP 资源列表包含无效 URI"
                );
            }
            items.extend(page_items.iter().cloned());
            cursor = response
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                return Ok(items);
            }
            ensure!(page + 1 < MAX_PAGES, "MCP {field} 分页超过 8 页");
            ensure!(
                cursor
                    .as_ref()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 512),
                "MCP 游标无效"
            );
        }
        unreachable!()
    }
}

pub fn run(config: Config, action: Action, cancelled: Arc<AtomicBool>) -> Result<Report> {
    match &action {
        Action::Call {
            tool,
            arguments,
            expected_tool,
        } => {
            ensure!(
                !tool.is_empty()
                    && tool.len() <= 128
                    && arguments.is_object()
                    && expected_tool.is_object()
                    && expected_tool.get("name").and_then(Value::as_str) == Some(tool),
                "工具名称或 JSON 参数无效"
            );
            ensure!(
                serde_json::to_vec(arguments)?.len() <= 240 * 1024,
                "工具参数超过 240 KiB"
            );
        }
        Action::ReadResource { uri } => {
            ensure!(
                !uri.is_empty() && uri.len() <= 2048 && !uri.contains('\0'),
                "资源 URI 无效"
            );
        }
        Action::GetPrompt { name, arguments } => {
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && arguments.as_object().is_some_and(|items| {
                        items.len() <= 32 && items.values().all(Value::is_string)
                    }),
                "提示词名称或参数无效；参数须为字符串值的 JSON 对象"
            );
            ensure!(
                serde_json::to_vec(arguments)?.len() <= 240 * 1024,
                "提示词参数超过 240 KiB"
            );
        }
        Action::Inspect => {}
    }
    let timeout = if matches!(action, Action::Inspect) {
        Duration::from_secs(15)
    } else {
        Duration::from_secs(30)
    };
    let mut session = Session::new(&config, timeout, cancelled)?;
    let (server, protocol, capabilities) = session.initialize()?;
    let mut report = Report {
        server,
        protocol,
        ..Default::default()
    };
    if capabilities.get("tools").is_some() {
        report.tools = session.list("tools/list", "tools")?;
    }
    if capabilities.get("resources").is_some() {
        report.resources = session.list("resources/list", "resources")?;
    }
    if capabilities.get("prompts").is_some() {
        report.prompts = session.list("prompts/list", "prompts")?;
    }
    match action {
        Action::Call {
            tool,
            arguments,
            expected_tool,
        } => {
            let listed = report
                .tools
                .iter()
                .find(|item| item.get("name").and_then(Value::as_str) == Some(&tool))
                .context("服务没有列出该工具，调用已拒绝")?;
            ensure!(
                *listed == expected_tool,
                "工具定义已变化，调用已拒绝；请重新检查服务能力并确认"
            );
            report.call_result =
                Some(session.request("tools/call", json!({"name":tool,"arguments":arguments}))?);
        }
        Action::ReadResource { uri } => {
            ensure!(
                report
                    .resources
                    .iter()
                    .any(|item| item.get("uri").and_then(Value::as_str) == Some(&uri)),
                "服务没有列出该资源，读取已拒绝"
            );
            let result = session.request("resources/read", json!({"uri":uri}))?;
            ensure!(
                result.get("contents").and_then(Value::as_array).is_some(),
                "MCP 资源响应缺少 contents 列表"
            );
            report.resource_result = Some((uri, result));
        }
        Action::GetPrompt { name, arguments } => {
            ensure!(
                report
                    .prompts
                    .iter()
                    .any(|item| item.get("name").and_then(Value::as_str) == Some(&name)),
                "服务没有列出该提示词，获取已拒绝"
            );
            let result =
                session.request("prompts/get", json!({"name":name,"arguments":arguments}))?;
            ensure!(
                result.get("messages").and_then(Value::as_array).is_some(),
                "MCP 提示词响应缺少 messages 列表"
            );
            report.prompt_result = Some((name, result));
        }
        Action::Inspect => {}
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nonexistent_executable_and_unbounded_arguments() {
        let config = Config {
            executable: PathBuf::from("missing.exe"),
            args: vec![],
        };
        assert!(config.validate().is_err());
        let config = Config {
            executable: std::env::current_exe().unwrap(),
            args: vec!["x".repeat(1025)],
        };
        assert!(config.validate().is_err());
    }
}
