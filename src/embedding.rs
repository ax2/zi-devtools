//! Explicit local Ollama embedding comparison; text and vectors remain in memory.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_DIMENSIONS: usize = 8192;

pub fn endpoint_url(endpoint: &str) -> Result<reqwest::Url> {
    let url = crate::plugins::endpoint(endpoint)?;
    ensure!(
        url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
            && url.path() == "/api/embed",
        "Embedding 只连接本机 127.0.0.1 或 ::1 的 /api/embed"
    );
    Ok(url)
}

fn validate_text(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty()
            && text.chars().count() <= 4000
            && text.len() <= 16 * 1024
            && !text
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
        "每段文本需为 1–4000 字符、最多 16 KiB，且不含异常控制字符"
    );
    Ok(())
}

pub fn validate(endpoint: &str, model: &str, a: &str, b: &str) -> Result<reqwest::Url> {
    let url = endpoint_url(endpoint)?;
    ensure!(
        !model.trim().is_empty() && model.len() <= 256 && !model.chars().any(char::is_control),
        "请填写有效嵌入模型名称"
    );
    validate_text(a)?;
    validate_text(b)?;
    Ok(url)
}

#[derive(Clone, Debug)]
pub struct Comparison {
    pub model: String,
    pub dimensions: usize,
    pub cosine: f64,
    pub euclidean: f64,
    pub norm_a: f64,
    pub norm_b: f64,
    pub vector_a: Vec<f64>,
    pub vector_b: Vec<f64>,
}

pub fn parse_response(bytes: &[u8], model: &str) -> Result<Comparison> {
    ensure!(bytes.len() <= MAX_RESPONSE, "嵌入响应超过 4 MiB");
    let response: Value = serde_json::from_slice(bytes).context("嵌入响应不是 JSON")?;
    let embeddings = response
        .get("embeddings")
        .and_then(Value::as_array)
        .context("嵌入响应缺少 embeddings 数组")?;
    ensure!(embeddings.len() == 2, "嵌入响应必须恰好包含两条向量");
    let mut vectors = Vec::with_capacity(2);
    for item in embeddings {
        let values = item.as_array().context("嵌入向量不是数值数组")?;
        ensure!(
            !values.is_empty() && values.len() <= MAX_DIMENSIONS,
            "嵌入维度必须在 1–8192 之间"
        );
        let vector = values
            .iter()
            .map(|value| value.as_f64().context("嵌入向量包含非数值"))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            vector
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 1e6),
            "嵌入向量包含非有限或异常数值"
        );
        vectors.push(vector);
    }
    let vector_b = vectors.pop().unwrap();
    let vector_a = vectors.pop().unwrap();
    ensure!(vector_a.len() == vector_b.len(), "两条嵌入向量维度不同");
    let dot: f64 = vector_a.iter().zip(&vector_b).map(|(a, b)| a * b).sum();
    let norm_a = vector_a.iter().map(|v| v * v).sum::<f64>().sqrt();
    let norm_b = vector_b.iter().map(|v| v * v).sum::<f64>().sqrt();
    let euclidean = vector_a
        .iter()
        .zip(&vector_b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    ensure!(
        norm_a.is_finite() && norm_b.is_finite() && norm_a > 0.0 && norm_b > 0.0,
        "嵌入向量范数无效或为零"
    );
    ensure!(euclidean.is_finite(), "向量距离无效");
    let cosine = (dot / (norm_a * norm_b)).clamp(-1.0, 1.0);
    ensure!(cosine.is_finite(), "余弦相似度无效");
    Ok(Comparison {
        model: model.into(),
        dimensions: vector_a.len(),
        cosine,
        euclidean,
        norm_a,
        norm_b,
        vector_a,
        vector_b,
    })
}

async fn cancellable<T>(future: impl Future<Output = T>, cancel: &AtomicBool) -> Result<T> {
    let mut future = Box::pin(future);
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "嵌入请求已取消");
        match tokio::time::timeout(Duration::from_millis(100), &mut future).await {
            Ok(result) => return Ok(result),
            Err(_) => continue,
        }
    }
}

pub fn compare(
    endpoint: &str,
    model: &str,
    a: &str,
    b: &str,
    cancel: &AtomicBool,
) -> Result<Comparison> {
    let url = validate(endpoint, model, a, b)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法创建嵌入请求运行时")?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("无法创建本机嵌入连接")?;
        let payload = serde_json::to_vec(&json!({"model":model,"input":[a,b]}))?;
        let request = client
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(payload);
        let mut response = cancellable(request.send(), cancel)
            .await?
            .map_err(|_| anyhow::anyhow!("本机嵌入服务连接失败或超时（最长 120 秒）"))?;
        ensure!(
            response.status().is_success(),
            "本机嵌入服务返回 HTTP {}，请检查模型与接口",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = cancellable(response.chunk(), cancel)
            .await?
            .map_err(|_| anyhow::anyhow!("读取嵌入响应失败或超时"))?
        {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE,
                "嵌入响应超过 4 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        ensure!(!cancel.load(Ordering::Relaxed), "嵌入请求已取消");
        parse_response(&bytes, model)
    })
}

pub struct State {
    endpoint: String,
    model: String,
    a: String,
    b: String,
    result: Option<Comparison>,
    running: Option<Receiver<Result<Comparison, String>>>,
    model_running: Option<Receiver<Result<Vec<String>, String>>>,
    models: Vec<String>,
    cancel: Arc<AtomicBool>,
    message: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:11434/api/embed".into(),
            model: String::new(),
            a: String::new(),
            b: String::new(),
            result: None,
            running: None,
            model_running: None,
            models: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            message: String::new(),
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl State {
    #[cfg(feature = "ui-preview")]
    pub fn preview_fixture(&mut self) {
        self.model = "bge-m3:latest".into();
        self.a = "Rust 的所有权模型避免了数据竞争。".into();
        self.b = "Rust 使用借用检查器管理内存安全。".into();
        let samples_a = [0.31, -0.12, 0.27, 0.53, 0.08, -0.18, 0.41, 0.35];
        let samples_b = [0.28, -0.09, 0.32, 0.49, 0.11, -0.15, 0.38, 0.39];
        let a = samples_a.into_iter().cycle().take(1024).collect::<Vec<_>>();
        let b = samples_b.into_iter().cycle().take(1024).collect::<Vec<_>>();
        let bytes = serde_json::to_vec(&json!({"embeddings":[a,b]})).unwrap();
        self.result = Some(parse_response(&bytes, &self.model).unwrap());
        self.message = "合成界面预览 · 未调用模型".into();
    }

    fn start(&mut self) {
        if let Err(error) = validate(&self.endpoint, &self.model, &self.a, &self.b) {
            self.message = error.to_string();
            return;
        }
        let endpoint = self.endpoint.clone();
        let model = self.model.clone();
        let a = self.a.clone();
        let b = self.b.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = compare(&endpoint, &model, &a, &b, &cancel).map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        self.result = None;
        self.running = Some(receiver);
        self.message = "正在从本机模型生成两条向量…".into();
    }

    fn discover(&mut self) {
        let mut url = match endpoint_url(&self.endpoint) {
            Ok(url) => url,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        url.set_path("/api/chat");
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::model_discovery::fetch(url.as_str(), String::new())
                .map(|report| report.models)
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
        });
        self.model_running = Some(receiver);
        self.message = "正在读取本机模型列表…".into();
    }

    fn poll(&mut self, ui: &egui::Ui) {
        if let Some(receiver) = &self.running {
            match receiver.try_recv() {
                Ok(Ok(result)) => {
                    self.message = format!(
                        "已生成两条 {} 维向量；内容仅保留在当前程序内存。",
                        result.dimensions
                    );
                    self.result = Some(result);
                    self.running = None;
                }
                Ok(Err(error)) => {
                    self.message = error;
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "嵌入任务意外结束".into();
                    self.running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
            }
        }
        if let Some(receiver) = &self.model_running {
            match receiver.try_recv() {
                Ok(Ok(models)) => {
                    self.message =
                        format!("发现 {} 个本机模型；请选支持嵌入的模型。", models.len());
                    self.models = models;
                    self.model_running = None;
                }
                Ok(Err(error)) => {
                    self.message = error;
                    self.model_running = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "模型列表任务意外结束".into();
                    self.model_running = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.poll(ui);
        let busy = self.running.is_some() || self.model_running.is_some();
        ui.heading("Embedding 工作台");
        ui.label("明确调用本机 Ollama 嵌入模型，比较两段文本的向量。可复制完整向量供其他应用使用；本工具不建立知识库索引。");
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("本机模型");
            ui.label("接口");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.endpoint)
                        .desired_width(ui.available_width().min(850.0)),
                )
                .changed()
            {
                self.result = None;
                self.models.clear();
            }
            ui.horizontal(|ui| {
                ui.label("模型");
                if ui
                    .add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.model)
                            .desired_width(280.0)
                            .hint_text("例如 bge-m3:latest"),
                    )
                    .changed()
                {
                    self.result = None;
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("获取本机模型列表"))
                    .clicked()
                {
                    self.discover();
                }
                if self.model_running.is_some() {
                    ui.spinner();
                }
            });
            if !self.models.is_empty() && !busy {
                egui::ComboBox::from_id_salt("embedding-models")
                    .selected_text("从列表选择模型")
                    .show_ui(ui, |ui| {
                        for name in &self.models {
                            if ui.button(name).clicked() {
                                self.model = name.clone();
                                self.result = None;
                            }
                        }
                    });
            }
            ui.small(
                "只接受本机回环 /api/embed；列表可能包含不支持嵌入的模型。模型首次加载可能较慢。",
            );
        });
        ui.add_space(8.0);
        ui.columns(2, |columns| {
            for (index, text) in [&mut self.a, &mut self.b].into_iter().enumerate() {
                columns[index].strong(if index == 0 { "文本 A" } else { "文本 B" });
                if columns[index]
                    .add_enabled(
                        !busy,
                        egui::TextEdit::multiline(text)
                            .desired_rows(6)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.result = None;
                }
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("生成并比较向量"))
                .clicked()
            {
                self.start();
            }
            if ui
                .add_enabled(self.running.is_some(), egui::Button::new("取消请求"))
                .clicked()
            {
                self.cancel.store(true, Ordering::Relaxed);
                self.message = "正在取消本机请求…".into();
            }
            if self.running.is_some() {
                ui.spinner();
            }
        });
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(result) = &self.result {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width().min(900.0));
                ui.strong(format!("{} · {} 维", result.model, result.dimensions));
                ui.add_space(7.0);
                ui.columns(2, |columns| {
                    columns[0].small("余弦相似度 · 越接近 1 越相似");
                    columns[0].label(
                        egui::RichText::new(format!("{:.6}", result.cosine))
                            .size(25.0)
                            .strong(),
                    );
                    columns[1].small("欧氏距离 · 越接近 0 越相似");
                    columns[1].label(
                        egui::RichText::new(format!("{:.6}", result.euclidean))
                            .size(25.0)
                            .strong(),
                    );
                });
                ui.add_space(5.0);
                ui.small(format!(
                    "范数：A {:.6} · B {:.6}",
                    result.norm_a, result.norm_b
                ));
                ui.small("分数只反映当前模型的向量几何关系，不代表事实正确或来源可信。");
                ui.horizontal(|ui| {
                    if ui.button("复制完整向量 A").clicked() {
                        ui.ctx()
                            .copy_text(serde_json::to_string(&result.vector_a).unwrap_or_default());
                    }
                    if ui.button("复制完整向量 B").clicked() {
                        ui.ctx()
                            .copy_text(serde_json::to_string(&result.vector_b).unwrap_or_default());
                    }
                    if ui.button("复制指标").clicked() {
                        ui.ctx().copy_text(format!(
                            "模型：{}；维度：{}；余弦相似度：{:.6}；欧氏距离：{:.6}",
                            result.model, result.dimensions, result.cosine, result.euclidean
                        ));
                    }
                });
                egui::CollapsingHeader::new("向量预览 · 每条前 8 个元素")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (label, values) in [("A", &result.vector_a), ("B", &result.vector_b)] {
                            let preview = values
                                .iter()
                                .take(8)
                                .map(|value| format!("{value:.5}"))
                                .collect::<Vec<_>>()
                                .join(", ");
                            ui.monospace(format!(
                                "{label}: [{preview}{}]",
                                if values.len() > 8 { ", …" } else { "" }
                            ));
                        }
                    });
            });
        }
        ui.small("文本和向量仅保留在当前程序内存；请求会发送给你选择的本机 Ollama，服务自身日志和模型资源由其设置决定。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_endpoint_and_vector_shape() {
        assert!(endpoint_url("http://127.0.0.1:11434/api/embed").is_ok());
        for endpoint in [
            "https://example.com/api/embed",
            "http://localhost:11434/api/embed",
            "http://127.0.0.1:11434/api/chat",
            "http://127.0.0.1:11434/api/embed?token=x",
        ] {
            assert!(endpoint_url(endpoint).is_err(), "{endpoint}");
        }
        let result = parse_response(br#"{"embeddings":[[3,4],[4,3]]}"#, "fixture").unwrap();
        assert_eq!(result.dimensions, 2);
        assert!((result.cosine - 0.96).abs() < 1e-12);
        assert!((result.euclidean - 2_f64.sqrt()).abs() < 1e-12);
        for response in [
            br#"{"embeddings":[]}"#.as_slice(),
            br#"{"embeddings":[[0,0],[1,1]]}"#,
            br#"{"embeddings":[[1],[1,2]]}"#,
            br#"{"embeddings":[[1,"bad"],[1,2]]}"#,
        ] {
            assert!(parse_response(response, "fixture").is_err());
        }
    }
}
