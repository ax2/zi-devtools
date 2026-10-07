use std::{
    fs,
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{Mutex, MutexGuard},
    thread,
    time::Duration,
};

use zi_devtools::{
    config::load_config,
    service::{ServiceManager, ServiceState},
};

// Each fixture reserves an ephemeral port only until its child binds it. Running
// these tests in parallel can hand the same released port to another fixture.
fn network_fixture_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[test]
fn starts_health_checks_logs_and_stops_a_windows_service_tree() {
    let _guard = network_fixture_guard();
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

    let original = manager.service_spec("fixture").unwrap();
    let mut edited = original.clone();
    edited.name = "Edited fixture".into();
    assert!(
        manager
            .save_service(Some(&original), edited.clone())
            .is_err()
    );
    assert!(manager.delete_service(&original).is_err());
    assert_eq!(manager.service_spec("fixture").unwrap(), original);

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

    manager
        .save_service(Some(&original), edited.clone())
        .unwrap();
    assert_eq!(
        load_config(&config_path).unwrap().services["fixture"].name,
        "Edited fixture"
    );
    manager.delete_service(&edited).unwrap();
    assert!(load_config(&config_path).unwrap().services.is_empty());
    assert!(state.join("logs/fixture.log").is_file());
    assert!(repo.is_dir());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn service_crud_retains_yaml_options_and_rejects_stale_or_duplicate_edits() {
    let root = std::env::temp_dir().join(format!("zi-service-crud-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("services.yml");
    fs::write(
        &path,
        format!(
            "state_dir: '{}'\ncustom_option: retained\nservices: {{}}\n",
            root.join("state").display()
        ),
    )
    .unwrap();
    let manager = ServiceManager::new(load_config(&path).unwrap()).unwrap();
    let spec: zi_devtools::config::ServiceSpec = serde_yaml_ng::from_str(&format!(
        "name: One\nrepo: '{}'\ncommand: echo fixture\nenv: {{TEST: original}}\n",
        root.display()
    ))
    .unwrap();
    let mut spec = spec;
    spec.id = "one".into();
    manager.save_service(None, spec.clone()).unwrap();
    assert!(manager.save_service(None, spec.clone()).is_err());
    let saved = manager.service_spec("one").unwrap();
    let mut changed = saved.clone();
    changed.description = "changed".into();
    manager.save_service(Some(&saved), changed.clone()).unwrap();
    assert!(manager.save_service(Some(&saved), saved.clone()).is_err());
    assert!(manager.delete_service(&saved).is_err());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("custom_option: retained"));
    // Preserve extensions attached to the service as well as top-level options.
    let mut document: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
    document["services"]["one"]["extension"] = serde_yaml_ng::Value::String("keep".into());
    fs::write(&path, serde_yaml_ng::to_string(&document).unwrap()).unwrap();
    manager.replace_config(load_config(&path).unwrap()).unwrap();
    manager
        .save_service(Some(&changed), changed.clone())
        .unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("extension: keep")
    );
    let mut external = load_config(&path).unwrap().services["one"].clone();
    external.command = "echo external".into();
    document = serde_yaml_ng::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    document["services"]["one"]["command"] = serde_yaml_ng::Value::String(external.command.clone());
    fs::write(&path, serde_yaml_ng::to_string(&document).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(
        manager
            .save_service(Some(&changed), changed.clone())
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    manager.replace_config(load_config(&path).unwrap()).unwrap();
    manager.delete_service(&external).unwrap();
    assert!(manager.service_ids().is_empty());
    assert!(load_config(&path).unwrap().services.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn first_service_creates_independent_config_and_rejects_invalid_inputs() {
    let root = std::env::temp_dir().join(format!("zi-service-first-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("new/services.yml");
    let manager = ServiceManager::new(zi_devtools::config::DashboardConfig {
        path: path.clone(),
        state_dir: root.join("state"),
        services: Default::default(),
        modified: None,
    })
    .unwrap();
    let mut spec: zi_devtools::config::ServiceSpec = serde_yaml_ng::from_str(&format!(
        "repo: '{}'\ncommand: echo fixture",
        root.display()
    ))
    .unwrap();
    spec.id = "../escape".into();
    assert!(manager.save_service(None, spec.clone()).is_err());
    assert!(!path.exists());
    spec.id = "first".into();
    spec.port = Some(0);
    assert!(manager.save_service(None, spec.clone()).is_err());
    spec.port = None;
    manager.save_service(None, spec).unwrap();
    assert!(load_config(&path).unwrap().services.contains_key("first"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn does_not_kill_external_port_owner_or_reused_pid() {
    let _guard = network_fixture_guard();
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
    let _guard = network_fixture_guard();
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
    let result = if graceful {
        manager.stop("fixture").unwrap()
    } else {
        let stopping = manager.clone();
        let task = thread::spawn(move || stopping.stop("fixture"));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !manager
            .logs("fixture", 100)
            .unwrap()
            .contains("Zi DevTools graceful stop")
        {
            assert!(
                std::time::Instant::now() < deadline,
                "stop never entered graceful wait"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let config = load_config(&config_path).unwrap();
        let began = std::time::Instant::now();
        assert!(
            manager.replace_config(config).is_err(),
            "hot reload must defer during stop"
        );
        assert!(
            began.elapsed() < Duration::from_millis(250),
            "hot reload blocked the UI during stop"
        );
        task.join().unwrap().unwrap()
    };
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
