use super::{java, report};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Process {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn run_with_limits(
    executable: &Path,
    args: &[&str],
    limit: usize,
    timeout: Duration,
) -> Result<String> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(std::env::temp_dir());
    // Do not allow environment JVM agents/options or Python paths to change the fixed probe.
    for name in [
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "CLASSPATH",
        "PYTHONPATH",
        "PYTHONHOME",
        "DJANGO_SETTINGS_MODULE",
    ] {
        command.env_remove(name);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let child = command.spawn().context("无法启动所选可执行文件")?;
    #[cfg(windows)]
    let mut process = Process {
        child,
        job: std::ptr::null_mut(),
    };
    #[cfg(not(windows))]
    let mut process = Process { child };
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        process.job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        ensure!(!process.job.is_null(), "无法创建检测进程 Job");
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        ensure!(
            SetInformationJobObject(
                process.job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32
            ) != 0,
            "无法设置检测进程限制"
        );
        ensure!(
            AssignProcessToJobObject(process.job, process.child.as_raw_handle()) != 0,
            "无法将检测进程加入 Job"
        );
    }
    let stdout = process.child.stdout.take().unwrap();
    let stderr = process.child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    for (index, stream) in [
        (0, Box::new(stdout) as Box<dyn Read + Send>),
        (1, Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let result = stream
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send((index, result));
        });
    }
    drop(tx);
    let start = Instant::now();
    let mut output = [None, None];
    loop {
        while let Ok((i, result)) = rx.try_recv() {
            let bytes = result?;
            ensure!(
                bytes.len() <= limit,
                "检测输出超过 {limit} 字节，请缩小输入"
            );
            output[i] = Some(bytes);
        }
        if let Some(status) = process.child.try_wait()?
            && output.iter().all(Option::is_some)
        {
            let stdout = String::from_utf8_lossy(output[0].as_ref().unwrap()).into_owned();
            let stderr = String::from_utf8_lossy(output[1].as_ref().unwrap()).into_owned();
            ensure!(
                status.success(),
                "检测失败（退出码 {:?}）：{}",
                status.code(),
                stderr.chars().take(600).collect::<String>()
            );
            return Ok(if stdout.trim().is_empty() {
                stderr
            } else {
                stdout
            });
        }
        ensure!(
            start.elapsed() < timeout,
            "检测超过 {} 秒，已结束本次检测进程",
            timeout.as_secs()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn executable(path: &str, names: &[&str]) -> Result<PathBuf> {
    let path = Path::new(path.trim());
    ensure!(
        path.is_absolute() && path.is_file(),
        "请选择存在的绝对可执行文件路径"
    );
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    ensure!(
        names.contains(&name.as_str()),
        "请选择 {}，不支持脚本或任意命令参数",
        names.join(" / ")
    );
    path.canonicalize().context("无法解析可执行文件路径")
}
pub fn path_entry(names: &[&str]) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .flat_map(|p| names.iter().map(move |name| p.join(name)))
        .find(|p| p.is_absolute() && p.is_file())
}
fn java_probe(path: &str) -> Result<Value> {
    let path = executable(path, &["java.exe", "java"])?;
    let text = run_with_limits(
        &path,
        &["-XshowSettings:properties", "-version"],
        64 * 1024,
        Duration::from_secs(15),
    )?;
    let mut properties = serde_json::Map::new();
    for line in text.lines() {
        if let Some((key, value)) = line.trim().split_once(" = ")
            && ["java.home", "java.version", "java.vendor", "os.arch"].contains(&key)
        {
            properties.insert(key.into(), json!(value));
        }
    }
    ensure!(
        properties.contains_key("java.version"),
        "未识别 JDK 版本输出"
    );
    Ok(
        json!({"executable":path,"properties":properties,"javacPresent":path.with_file_name(if cfg!(windows){"javac.exe"}else{"javac"}).is_file()}),
    )
}
pub fn java_environment(primary: &str, comparison: &str) -> Result<String> {
    let selected = java_probe(primary)?;
    let compared = if comparison.trim().is_empty() {
        Value::Null
    } else {
        java_probe(comparison)?
    };
    let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    let selected_home = selected
        .pointer("/properties/java.home")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let mismatch = java_home
        .as_ref()
        .zip(selected_home.as_ref())
        .and_then(|(a, b)| Some(a.canonicalize().ok()? != b.canonicalize().ok()?));
    report(
        json!({"selected":selected,"comparison":compared,"javaHome":java_home,"javaHomeMismatch":mismatch,"pathJava":path_entry(&["java.exe","java"]),"mavenEntry":path_entry(&["mvn.cmd","mvn"]),"gradleEntry":path_entry(&["gradle.bat","gradle"]),"versionMismatch":if compared.is_null(){Value::Null}else{json!(selected["properties"]["java.version"]!=compared["properties"]["java.version"])},"notes":"执行固定 java -XshowSettings:properties -version；仅返回选定属性。构建入口只检查 PATH，未执行 Maven/Gradle 或读取 IDE 配置。未选择比较入口时不能判断 IDE 版本。javac 缺失可能是 JRE/裁剪运行时；具体构建兼容性需结合项目插件版本。"}),
    )
}
const PYTHON_PROBE: &str = r#"import sys,json,sysconfig,importlib.metadata as m,importlib.util as u
try:
 d=m.distribution('Django'); django={'version':d.version,'location':str(d.locate_file('django'))}
except m.PackageNotFoundError: django=None
s=u.find_spec('django')
print(json.dumps({'executable':sys.executable,'version':sys.version.split()[0],'prefix':sys.prefix,'basePrefix':sys.base_prefix,'scriptsPath':sysconfig.get_path('scripts'),'virtualEnvironment':sys.prefix!=sys.base_prefix,'django':django,'djangoImportOrigin':None if s is None else s.origin}))"#;
fn python_probe(path: &str) -> Result<Value> {
    let path = executable(path, &["python.exe", "python3.exe", "python", "python3"])?;
    let output = run_with_limits(
        &path,
        &["-I", "-c", PYTHON_PROBE],
        64 * 1024,
        Duration::from_secs(15),
    )?;
    serde_json::from_str(output.trim()).context("解释器未返回预期 JSON；启动钩子可能写入了额外内容")
}
pub fn python_environment(primary: &str, comparison: &str) -> Result<String> {
    let selected = python_probe(primary)?;
    let compared = if comparison.trim().is_empty() {
        Value::Null
    } else {
        python_probe(comparison)?
    };
    let admin = path_entry(&["django-admin.exe", "django-admin"]);
    let sibling_admin = selected["scriptsPath"].as_str().map(|p| {
        if cfg!(windows) {
            Path::new(p).join("django-admin.exe")
        } else {
            Path::new(p).join("django-admin")
        }
    });
    let mismatch = admin
        .as_ref()
        .zip(sibling_admin.as_ref())
        .map(|(a, b)| a.canonicalize().ok() != b.canonicalize().ok());
    report(
        json!({"selected":selected,"comparison":compared,"pathPython":path_entry(&["python.exe","python3","python"]),"pathDjangoAdmin":admin,"expectedSiblingDjangoAdmin":sibling_admin,"entryLayoutMismatch":mismatch,"notes":"运行所选可信解释器 -I，仅检查分发元数据与模块定位，不 import django、不加载项目 settings。解释器启动仍可能执行已安装 site/.pth 钩子。CLI 目录差异只是线索，未执行 django-admin；缺失分发或模块位置需要检查所选环境。"}),
    )
}
pub fn recording(exe: &str, file: &str) -> Result<String> {
    let path = executable(exe, &["jfr.exe", "jfr"])?;
    let file = Path::new(file).canonicalize().context("JFR 文件不存在")?;
    ensure!(
        file.is_file()
            && file
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("jfr")),
        "请选择 .jfr 文件"
    );
    ensure!(
        file.metadata()?.len() <= 64 * 1024 * 1024,
        "录制超过 64 MiB；请缩小录制"
    );
    let text = run_with_limits(
        &path,
        &[
            "print",
            "--json",
            "--events",
            "jdk.ExecutionSample,jdk.GarbageCollection,jdk.JavaMonitorEnter,jdk.ThreadPark",
            file.to_str().context("文件路径无效")?,
        ],
        8 * 1024 * 1024,
        Duration::from_secs(30),
    )?;
    java::jfr(&text)
}
pub fn actuator(base: &str, endpoint: &str, token: &str) -> Result<String> {
    let valid = endpoint == "health"
        || endpoint == "info"
        || endpoint == "metrics"
        || endpoint.strip_prefix("metrics/").is_some_and(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.len() <= 180
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        });
    ensure!(valid, "只允许 health、info、metrics 或 metrics/指标名");
    ensure!(token.len() <= 8192, "令牌过长");
    let base = reqwest::Url::parse(base.trim())?;
    ensure!(
        base.username().is_empty()
            && base.password().is_none()
            && base.query().is_none()
            && base.fragment().is_none(),
        "地址不能包含凭据、查询或 fragment"
    );
    let local = matches!(
        base.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    ensure!(
        base.scheme() == "https" || (base.scheme() == "http" && local),
        "远程接口必须 HTTPS，HTTP 仅允许本机回环"
    );
    let url = format!("{}/{}", base.as_str().trim_end_matches('/'), endpoint);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()?;
    let mut request = client.get(&url);
    if !token.trim().is_empty() {
        request = request.bearer_auth(token.trim());
    }
    let response = request.send().context("连接失败或超时")?;
    let status = response.status();
    let mut bytes = vec![];
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "响应超过 2 MiB");
    let body: Value = serde_json::from_slice(&bytes).context("响应不是 JSON")?;
    // Health DOWN commonly returns 503 and remains useful diagnostic evidence.
    report(
        json!({"url":url,"httpStatus":status.as_u16(),"httpSuccess":status.is_success(),"body":body,"notes":"仅执行展示端点的单次 GET，不枚举 env/configprops、不跟随重定向。HTTP 503 的 health 结果仍保留；不代表已读取所有实例。"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "subprocess fixture invoked by bounded_process"]
    fn child_fixture() {
        println!("{}", "x".repeat(200000));
        std::thread::sleep(Duration::from_secs(2));
    }
    #[test]
    fn bounded_process() {
        let exe = std::env::current_exe().unwrap();
        let args = [
            "--ignored",
            "--exact",
            "framework::runtime::tests::child_fixture",
            "--nocapture",
        ];
        assert!(
            run_with_limits(&exe, &args, 64, Duration::from_secs(5))
                .unwrap_err()
                .to_string()
                .contains("输出超过")
        );
        assert!(
            run_with_limits(&exe, &args, 1024 * 1024, Duration::from_millis(120))
                .unwrap_err()
                .to_string()
                .contains("超过")
        );
    }
    #[test]
    fn forbids_scripts_and_actuator_other_endpoints() {
        assert!(executable("cmd.exe", &["java.exe"]).is_err());
        for endpoint in [
            "env",
            "metrics/../env",
            "metrics/..",
            "metrics/.",
            "metrics/x?tag=secret",
            "shutdown",
        ] {
            assert!(actuator("http://127.0.0.1:1/actuator", endpoint, "").is_err());
        }
        assert!(actuator("http://example.com/actuator", "health", "").is_err());
    }
    #[test]
    fn actuator_retains_down_health_and_uses_only_requested_get() {
        use std::{io::Write, net::TcpListener};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let n = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]);
            assert!(request.starts_with("GET /actuator/health HTTP/1.1"));
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: bearer fixture-only")
            );
            let body = r#"{"status":"DOWN"}"#;
            write!(stream,"HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let text = actuator(
            &format!("http://{address}/actuator"),
            "health",
            "fixture-only",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["httpStatus"], 503);
        assert_eq!(v["body"]["status"], "DOWN");
        assert!(!text.contains("fixture-only"));
        server.join().unwrap();
    }
}
