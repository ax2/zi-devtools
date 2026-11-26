//! Numeric comparison shared by the original Ollama flow and the offline tool.
use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub const MAX_RESPONSE: usize = 4 * 1024 * 1024;
pub const MAX_DIMENSIONS: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub dimensions: usize,
    pub cosine: f64,
    pub euclidean: f64,
    pub norm_a: f64,
    pub norm_b: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricsError {
    Dimension,
    UnequalDimensions,
    Component,
    InvalidNorm,
    InvalidDistance,
    InvalidCosine,
}

impl std::fmt::Display for MetricsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Dimension => "嵌入维度必须在 1–8192 之间",
            Self::UnequalDimensions => "嵌入向量维度不同",
            Self::Component => "嵌入向量包含非有限或异常数值",
            Self::InvalidNorm => "嵌入向量范数无效或为零",
            Self::InvalidDistance => "向量距离无效",
            Self::InvalidCosine => "余弦相似度无效",
        })
    }
}

impl std::error::Error for MetricsError {}

fn norm(vector: &[f64]) -> f64 {
    vector.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn component_is_valid(value: f64) -> bool {
    value.is_finite() && value.abs() <= 1e6
}

pub fn compare_metrics(a: &[f64], b: &[f64]) -> std::result::Result<Metrics, MetricsError> {
    if !(1..=MAX_DIMENSIONS).contains(&a.len()) || !(1..=MAX_DIMENSIONS).contains(&b.len()) {
        return Err(MetricsError::Dimension);
    }
    if a.len() != b.len() {
        return Err(MetricsError::UnequalDimensions);
    }
    if !a.iter().chain(b).copied().all(component_is_valid) {
        return Err(MetricsError::Component);
    }
    let dot: f64 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    let norm_a = norm(a);
    let norm_b = norm(b);
    let euclidean = a
        .iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    if !(norm_a.is_finite() && norm_b.is_finite() && norm_a > 0.0 && norm_b > 0.0) {
        return Err(MetricsError::InvalidNorm);
    }
    if !euclidean.is_finite() {
        return Err(MetricsError::InvalidDistance);
    }
    let cosine = (dot / (norm_a * norm_b)).clamp(-1.0, 1.0);
    if !cosine.is_finite() {
        return Err(MetricsError::InvalidCosine);
    }
    Ok(Metrics {
        dimensions: a.len(),
        cosine,
        euclidean,
        norm_a,
        norm_b,
    })
}

pub fn parse_response(bytes: &[u8], model: &str) -> Result<Comparison> {
    let mut vectors = parse_vectors(bytes, 2)?;
    let vector_b = vectors.pop().expect("two vectors were validated");
    let vector_a = vectors.pop().expect("two vectors were validated");
    let metrics = compare_metrics(&vector_a, &vector_b)?;
    Ok(Comparison {
        model: model.into(),
        dimensions: metrics.dimensions,
        cosine: metrics.cosine,
        euclidean: metrics.euclidean,
        norm_a: metrics.norm_a,
        norm_b: metrics.norm_b,
        vector_a,
        vector_b,
    })
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

pub fn parse_vectors(bytes: &[u8], expected: usize) -> Result<Vec<Vec<f64>>> {
    ensure!(bytes.len() <= MAX_RESPONSE, "嵌入响应超过 4 MiB");
    ensure!((1..=16).contains(&expected), "每批最多 16 条文本");
    let response: Value = serde_json::from_slice(bytes).context("嵌入响应不是 JSON")?;
    let embeddings = response
        .get("embeddings")
        .and_then(Value::as_array)
        .context("嵌入响应缺少 embeddings 数组")?;
    ensure!(embeddings.len() == expected, "嵌入响应向量数量不符合请求");
    let mut vectors = Vec::with_capacity(expected);
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
    let dimensions = vectors[0].len();
    ensure!(
        vectors.iter().all(|vector| vector.len() == dimensions),
        "嵌入向量维度不同"
    );
    ensure!(
        vectors.iter().all(|vector| {
            let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
            norm.is_finite() && norm > 0.0
        }),
        "嵌入向量范数无效或为零"
    );
    Ok(vectors)
}
