use std::{
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum NetworkPane {
    #[default]
    Dns,
    Tcp,
}

pub struct NetworkState {
    pub pane: NetworkPane,
    pub host: String,
    pub port: String,
    pub output: String,
    pub busy: bool,
    pub request_id: u64,
    pub task_pane: NetworkPane,
    pub job: crate::tasks::Job,
}

impl Default for NetworkState {
    fn default() -> Self {
        Self {
            pane: NetworkPane::Dns,
            host: "localhost".into(),
            port: "8080".into(),
            output: String::new(),
            busy: false,
            request_id: 0,
            task_pane: NetworkPane::Dns,
            job: crate::tasks::Job::default(),
        }
    }
}

impl NetworkState {
    pub fn task_snapshot(&self) -> Option<crate::tasks::Row> {
        let mut row = self.job.snapshot("network-diagnostic", "网络诊断", false)?;
        row.instance = Some(self.request_id.to_string());
        row.instance_name = Some(match self.task_pane {
            NetworkPane::Dns => "DNS 解析".into(),
            NetworkPane::Tcp => "TCP 连通性".into(),
        });
        Some(row)
    }
}

fn validate_host(host: &str) -> Result<&str> {
    let host = host.trim();
    if host.is_empty()
        || host.len() > 253
        || host.chars().any(char::is_whitespace)
        || host.contains('/')
        || host.contains('@')
    {
        return Err(anyhow!(
            "请输入主机名或 IP 地址，不要输入 URL、凭据或空白字符"
        ));
    }
    Ok(host)
}

pub fn resolve_host(host: &str) -> Result<String> {
    let host = validate_host(host)?;
    let mut addresses: Vec<_> = (host, 0)
        .to_socket_addrs()
        .context("DNS 解析失败")?
        .map(|address| address.ip())
        .collect();
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(anyhow!("未解析到 IP 地址"));
    }
    Ok(format!(
        "主机：{host}\n解析结果（{}）：\n{}",
        addresses.len(),
        addresses
            .into_iter()
            .take(32)
            .map(|address| address.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

pub fn test_tcp(host: &str, port: &str) -> Result<String> {
    let host = validate_host(host)?;
    let port: u16 = port.trim().parse().context("端口必须是 1–65535 的整数")?;
    if port == 0 {
        return Err(anyhow!("端口必须是 1–65535 的整数"));
    }
    let addresses: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .context("主机解析失败")?
        .take(8)
        .collect();
    if addresses.is_empty() {
        return Err(anyhow!("未解析到可连接的地址"));
    }
    let mut failures = Vec::new();
    for address in addresses {
        let started = Instant::now();
        match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(_) => {
                return Ok(format!(
                    "TCP 连接成功\n目标：{address}\n耗时：{} ms",
                    started.elapsed().as_millis()
                ));
            }
            Err(error) => failures.push(format!("{address}：{error}")),
        }
    }
    Err(anyhow!("TCP 连接失败\n{}", failures.join("\n")))
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn validates_host_and_resolves_localhost() {
        assert!(validate_host("https://example.com").is_err());
        assert!(validate_host("name@host").is_err());
        assert!(resolve_host("localhost").unwrap().contains("解析结果"));
    }

    #[test]
    fn checks_disposable_tcp_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(
            test_tcp("127.0.0.1", &port.to_string())
                .unwrap()
                .contains("连接成功")
        );
        assert!(test_tcp("127.0.0.1", "0").is_err());
    }

    #[test]
    fn task_snapshot_exposes_safe_status_and_fixed_diagnostic_kind_only() {
        let mut state = NetworkState {
            host: "private-host.example".into(),
            port: "8443".into(),
            output: "private host output and socket error".into(),
            task_pane: NetworkPane::Tcp,
            request_id: 7,
            ..Default::default()
        };
        state.job.begin();

        let running = state.task_snapshot().unwrap();
        assert_eq!(running.key, "network-diagnostic");
        assert_eq!(running.instance.as_deref(), Some("7"));
        assert_eq!(running.instance_name.as_deref(), Some("TCP 连通性"));
        assert!(running.phase.active());
        assert!(!running.cancellable);
        let visible = format!(
            "{} {} {}",
            running.title,
            running.instance_name.as_deref().unwrap_or_default(),
            running.summary
        );
        assert!(!visible.contains("private-host.example"));
        assert!(!visible.contains("8443"));
        assert!(!visible.contains("socket error"));

        state
            .job
            .finish(crate::tasks::Phase::Failed, "检查失败 · 25 ms");
        let failed = state.task_snapshot().unwrap();
        assert_eq!(failed.phase, crate::tasks::Phase::Failed);
        assert_eq!(failed.summary, "检查失败 · 25 ms");
        assert!(!failed.summary.contains("private host"));
    }
}
