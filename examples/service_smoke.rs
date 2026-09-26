use std::{
    env,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use zi_devtools::{
    config::load_config,
    service::{ServiceManager, ServiceState},
};

struct StopGuard {
    manager: Arc<ServiceManager>,
    service_id: String,
    armed: bool,
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.manager.stop(&self.service_id);
        }
    }
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let config_path = PathBuf::from(
        args.next()
            .context("usage: service_smoke <config> <service-id> [timeout-seconds]")?,
    );
    let service_id = args.next().context("missing service id")?;
    let timeout = args
        .next()
        .map(|value| value.parse::<u64>())
        .transpose()
        .context("invalid timeout")?
        .unwrap_or(60);

    let manager = ServiceManager::new(load_config(&config_path)?)?;
    let spec = manager.service_spec(&service_id)?;
    let initial = manager.service_status(&service_id)?;
    if initial.state.is_available() {
        bail!(
            "refusing smoke test because {} is already {}",
            service_id,
            initial.state.label()
        );
    }

    println!("START service={service_id} repo={}", spec.repo.display());
    manager.start(&service_id)?;
    let mut guard = StopGuard {
        manager: Arc::clone(&manager),
        service_id: service_id.clone(),
        armed: true,
    };
    let started = Instant::now();
    let mut last_state = None;
    let healthy = loop {
        let status = manager.service_status(&service_id)?;
        if last_state != Some(status.state) {
            println!(
                "STATE service={} state={} pid={:?} health={:?}",
                service_id,
                status.state.label(),
                status.pid,
                status.health.ok
            );
            last_state = Some(status.state);
        }
        if status.state == ServiceState::Running {
            if spec.health_url.is_some() {
                if status.health.ok == Some(true) {
                    break true;
                }
            } else if started.elapsed() >= Duration::from_secs(3) {
                break true;
            }
        }
        if status.state == ServiceState::Stopped && started.elapsed() >= Duration::from_secs(2) {
            break false;
        }
        if started.elapsed() >= Duration::from_secs(timeout) {
            break false;
        }
        thread::sleep(Duration::from_millis(250));
    };

    if !healthy {
        let logs = manager.logs(&service_id, 80).unwrap_or_default();
        bail!("service did not become healthy\n--- log tail ---\n{logs}");
    }
    println!(
        "HEALTHY service={service_id} elapsed_ms={}",
        started.elapsed().as_millis()
    );
    manager.stop(&service_id)?;
    guard.armed = false;
    thread::sleep(Duration::from_millis(500));
    let final_state = manager.service_status(&service_id)?.state;
    if final_state != ServiceState::Stopped {
        bail!("service remained {} after stop", final_state.label());
    }
    println!("STOPPED service={service_id}");
    Ok(())
}
