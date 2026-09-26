use std::{
    fs,
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use zi_devtools::{
    config::load_config,
    service::{ServiceManager, ServiceState},
};

#[test]
fn starts_health_checks_logs_and_stops_a_windows_service_tree() {
    let root = std::env::temp_dir().join(format!("zi-lifecycle-{}", uuid::Uuid::new_v4()));
    let repo = root.join("repo");
    let state = root.join("state");
    fs::create_dir_all(&repo).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let exe = env!("CARGO_BIN_EXE_ZiDevTools").replace('\\', "/");
    let config_path = root.join("local-services.yml");
    fs::write(
        &config_path,
        format!(
            "state_dir: {}\nservices:\n  fixture:\n    name: Fixture\n    repo: {}\n    command: '\"{}\" --fixture-server {}'\n    health_url: http://127.0.0.1:{}/health\n    port: {}\n    tags: [test]\n",
            state.display(),
            repo.display(),
            exe,
            port,
            port,
            port
        ),
    )
    .unwrap();

    let manager = ServiceManager::new(load_config(&config_path).unwrap()).unwrap();
    manager.start("fixture").unwrap();

    let mut ready = false;
    for _ in 0..30 {
        let status = manager.service_status("fixture").unwrap();
        if status.state == ServiceState::Running && status.health.ok == Some(true) {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(ready, "fixture service did not become healthy");

    let logs = manager.logs("fixture", 100).unwrap();
    assert!(logs.contains("fixture listening"));
    assert!(logs.contains("GET /health"));

    manager.stop("fixture").unwrap();
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        manager.service_status("fixture").unwrap().state,
        ServiceState::Stopped
    );
    let desired = fs::read_to_string(state.join("desired-running.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&desired).unwrap()["service_ids"],
        serde_json::json!([])
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn does_not_kill_external_port_owner_or_reused_pid() {
    let root = std::env::temp_dir().join(format!("zi-external-{}", uuid::Uuid::new_v4()));
    let repo = root.join("repo");
    let state = root.join("state");
    fs::create_dir_all(&repo).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let exe = env!("CARGO_BIN_EXE_ZiDevTools");
    let mut external = ExternalFixture(
        Command::new(exe)
            .args(["--fixture-server", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut listening = false;
    for _ in 0..40 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            listening = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(listening, "external fixture did not start");

    let config_path = root.join("services.yml");
    fs::write(
        &config_path,
        format!(
            "state_dir: {}\nservices:\n  external:\n    repo: {}\n    command: echo managed\n    stop_command: echo stop > external-stop-marker.txt\n    health_url: http://127.0.0.1:{}/health\n    port: {}\n  stale:\n    repo: {}\n    command: echo stale\n",
            state.display(),
            repo.display(),
            port,
            port,
            repo.display()
        ),
    )
    .unwrap();
    let manager = ServiceManager::new(load_config(&config_path).unwrap()).unwrap();
    assert_eq!(
        manager.service_status("external").unwrap().state,
        ServiceState::External
    );
    assert!(manager.start("external").is_err());
    manager.stop("external").unwrap();
    assert!(external.0.try_wait().unwrap().is_none());
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
    assert!(!repo.join("external-stop-marker.txt").exists());

    fs::write(
        state.join("pids").join("stale.pid"),
        format!("{{\"pid\":{},\"start_time\":1}}", std::process::id()),
    )
    .unwrap();
    assert_eq!(
        manager.service_status("stale").unwrap().state,
        ServiceState::Stopped
    );
    manager.stop("stale").unwrap();
    assert!(!state.join("pids").join("stale.pid").exists());

    fs::write(
        state.join("pids").join("stale.pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    assert_eq!(
        manager.service_status("stale").unwrap().state,
        ServiceState::Stopped
    );
    manager.stop("stale").unwrap();
    assert!(!state.join("pids").join("stale.pid").exists());

    drop(external);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stop_command_exits_gracefully_before_timeout() {
    run_stop_scenario(true);
}

#[test]
fn stop_command_timeout_forces_only_the_managed_tree() {
    run_stop_scenario(false);
}

fn run_stop_scenario(graceful: bool) {
    let root = std::env::temp_dir().join(format!("zi-stop-{}", uuid::Uuid::new_v4()));
    let repo = root.join("repo");
    let state = root.join("state");
    fs::create_dir_all(&repo).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let exe = env!("CARGO_BIN_EXE_ZiDevTools").replace('\\', "/");
    let stop_command = if graceful {
        format!("\"{exe}\" --fixture-shutdown {port}")
    } else {
        format!("\"{exe}\" --fixture-server 0")
    };
    let config_path = root.join("services.yml");
    fs::write(
        &config_path,
        format!(
            "state_dir: {}\nservices:\n  fixture:\n    repo: {}\n    command: '\"{}\" --fixture-server {}'\n    stop_command: '{}'\n    stop_timeout_ms: {}\n    health_url: http://127.0.0.1:{}/health\n    port: {}\n",
            state.display(),
            repo.display(),
            exe,
            port,
            stop_command,
            if graceful { 5_000 } else { 500 },
            port,
            port
        ),
    )
    .unwrap();
    let manager = ServiceManager::new(load_config(&config_path).unwrap()).unwrap();
    manager.start("fixture").unwrap();
    let mut ready = false;
    for _ in 0..30 {
        if manager.service_status("fixture").unwrap().health.ok == Some(true) {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(ready, "fixture not ready");
    drop(manager);
    let manager = ServiceManager::new(load_config(&config_path).unwrap()).unwrap();
    let result = manager.stop("fixture").unwrap();
    assert_eq!(
        result.message.contains("优雅停止"),
        graceful,
        "{}",
        result.message
    );
    assert_eq!(
        manager.service_status("fixture").unwrap().state,
        ServiceState::Stopped
    );
    let logs = manager.logs("fixture", 200).unwrap();
    assert!(
        logs.contains(if graceful {
            "fixture graceful shutdown"
        } else {
            "优雅停止超时"
        }),
        "{logs}"
    );
    fs::remove_dir_all(root).unwrap();
}

struct ExternalFixture(Child);

impl Drop for ExternalFixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
