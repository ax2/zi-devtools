use eframe::egui;
use std::{
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

pub fn discover() -> String {
    let mut lines =
        vec!["只读检查：不读取会话、文档、数据库或凭据，不启动软件、不下载模型。".to_owned()];
    let paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for (name, candidates) in [
        ("Ollama", &["ollama.exe"][..]),
        ("Codex CLI", &["codex.exe", "codex.cmd", "codex.ps1"]),
        ("Claude CLI", &["claude.exe", "claude.cmd"]),
        ("Python", &["python.exe"]),
        ("Java", &["java.exe"]),
        ("Java 编译器", &["javac.exe"]),
        ("Maven", &["mvn.cmd", "mvn.exe"]),
        ("Gradle", &["gradle.bat", "gradle.exe"]),
        ("Django CLI", &["django-admin.exe", "django-admin.cmd"]),
        ("Node.js", &["node.exe"]),
        ("uv", &["uv.exe"]),
        ("Docker", &["docker.exe"]),
        ("Git", &["git.exe"]),
    ] {
        let found = paths
            .iter()
            .any(|p| candidates.iter().any(|c| p.join(c).is_file()));
        lines.push(format!(
            "{name}：{}",
            if found {
                "PATH 中存在入口（未验证可运行或登录）"
            } else {
                "PATH 中未发现；不代表未安装"
            }
        ));
    }
    lines.push("\nJava / Django：入口存在不代表项目环境一致。Django CLI 与默认 Python 可能来自不同虚拟环境；本检查不运行命令或导入项目设置。".into());
    lines.push("\n本地端口（仅 TCP 连通，不证明接口或模型可用）：".into());
    for (name, port) in [
        ("Ollama", 11434),
        ("LM Studio / 兼容服务", 1234),
        ("AnythingLLM 常见服务", 3001),
        ("Qdrant", 6333),
    ] {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let open = TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok();
        lines.push(format!(
            "{name} · {address}：{}",
            if open { "TCP 可连接" } else { "未连接" }
        ));
    }
    lines.push("\n集成建议：Ollama / LM Studio 使用插件中心的 HTTP 连接器；AnythingLLM API、Codex CLI 任务、MCP 会话与 RAG 索引仍在规划，不会自动读取它们的数据。".into());
    lines.join("\n")
}
#[derive(Default)]
pub struct IntegrationState {
    output: String,
    receiver: Option<Receiver<String>>,
}
impl IntegrationState {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("本机集成发现");
        ui.label(
            "复用已有模型与开发环境，让核心应用保持轻量。检测仅覆盖 PATH 入口和少量本地端口。",
        );
        if ui
            .add_enabled(
                self.receiver.is_none(),
                egui::Button::new("扫描本机入口与服务端口"),
            )
            .clicked()
        {
            let (tx, rx) = mpsc::channel();
            self.receiver = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(discover());
            });
        }
        if let Some(rx) = &self.receiver {
            if let Ok(result) = rx.try_recv() {
                self.output = result;
                self.receiver = None;
            } else {
                ui.spinner();
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        }
        if ui
            .add_enabled(!self.output.is_empty(), egui::Button::new("复制检测报告"))
            .clicked()
        {
            ui.ctx().copy_text(self.output.clone());
        }
        let mut output = self.output.as_str();
        ui.add_sized(
            [ui.available_width(), 470.0],
            egui::TextEdit::multiline(&mut output).font(egui::TextStyle::Monospace),
        );
    }
}
