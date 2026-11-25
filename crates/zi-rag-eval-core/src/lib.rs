//! Shared fixture validation and retrieval evaluation metrics.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub mod review;
pub const MAX_REPORT_BYTES: usize = 64 * 1024;

fn present_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

const MAX_SUITE_BYTES: usize = 32 * 1024;
pub const MAX_CASES: usize = 10;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub question: String,
    pub terms: String,
    pub expected_source: String,
    pub expected_relative: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_answer_contains: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub version: u32,
    pub cases: Vec<Case>,
}

pub(crate) fn valid_text(value: &str, max_chars: usize, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

pub(crate) fn validate_case_content(
    id: &str,
    question: &str,
    terms: &str,
    source: &str,
    relative: &str,
    answer: Option<&str>,
) -> Result<()> {
    ensure!(
        valid_text(question, 1000, 4000),
        "{}：问题为空、过长或含控制字符",
        id
    );
    ensure!(
        valid_text(terms, 120, 512),
        "{}：检索词为空、过长或含控制字符",
        id
    );
    ensure!(
        valid_text(source, 80, 320) && valid_text(relative, 512, 2048),
        "{}：预期来源名称或相对路径无效",
        id
    );
    if let Some(expected) = answer {
        ensure!(
            valid_text(expected, 200, 800),
            "{}：预期答案片段为空、过长或含控制字符",
            id
        );
    }
    Ok(())
}

pub fn parse_suite_with(
    input: &str,
    mut verify_source: impl FnMut(&Case) -> Result<()>,
) -> Result<Suite> {
    ensure!(input.len() <= MAX_SUITE_BYTES, "评测集超过 32 KiB");
    let suite: Suite = serde_json::from_str(input).context("评测集 JSON 格式无效")?;
    ensure!(suite.version == 1, "只支持评测集版本 1");
    ensure!(
        (1..=MAX_CASES).contains(&suite.cases.len()),
        "评测集需包含 1–10 个样例"
    );
    let mut ids = HashSet::new();
    for case in &suite.cases {
        ensure!(
            valid_text(&case.id, 64, 128),
            "样例 ID 为空、过长或含控制字符"
        );
        ensure!(ids.insert(&case.id), "样例 ID 重复：{}", case.id);
        validate_case_content(
            &case.id,
            &case.question,
            &case.terms,
            &case.expected_source,
            &case.expected_relative,
            case.expected_answer_contains.as_deref(),
        )?;
        verify_source(case)?;
    }
    Ok(suite)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub id: usize,
    pub source: String,
    pub relative: String,
    pub location: String,
    pub file_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub id: String,
    pub question: String,
    pub terms: String,
    pub expected_source: String,
    pub expected_relative: String,
    #[serde(deserialize_with = "present_option")]
    pub expected_answer_contains: Option<String>,
    pub evidence_count: usize,
    pub evidence: Vec<EvidenceRecord>,
    #[serde(deserialize_with = "present_option")]
    pub expected_rank: Option<usize>,
    pub cited_ids: Vec<usize>,
    #[serde(deserialize_with = "present_option")]
    pub expected_cited: Option<bool>,
    #[serde(deserialize_with = "present_option")]
    pub answer_contains_expected: Option<bool>,
    #[serde(deserialize_with = "present_option")]
    pub answer: Option<String>,
    #[serde(deserialize_with = "present_option")]
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonCase {
    pub id: String,
    pub expected_source: String,
    pub expected_relative: String,
    #[serde(deserialize_with = "present_option")]
    pub keyword_rank: Option<usize>,
    #[serde(deserialize_with = "present_option")]
    pub hybrid_rank: Option<usize>,
    pub keyword_ms: u128,
    pub hybrid_ms: u128,
    #[serde(deserialize_with = "present_option")]
    pub keyword_error: Option<String>,
    #[serde(deserialize_with = "present_option")]
    pub hybrid_error: Option<String>,
}

pub fn first_expected_rank<'a>(
    mut evidence: impl Iterator<Item = (usize, &'a str, &'a str)>,
    expected_source: &str,
    expected_relative: &str,
) -> Option<usize> {
    evidence.find_map(|(rank, source, relative)| {
        (source == expected_source && relative == expected_relative).then_some(rank)
    })
}

#[derive(Default, Debug)]
pub struct ComparisonStats {
    pub paired: usize,
    pub keyword_hits: usize,
    pub hybrid_hits: usize,
    pub keyword_only: usize,
    pub hybrid_only: usize,
    pub both: usize,
    pub neither: usize,
    pub keyword_mrr: f64,
    pub hybrid_mrr: f64,
}

pub fn reciprocal_rank(rank: Option<usize>) -> f64 {
    rank.map_or(0.0, |position| 1.0 / position as f64)
}

pub fn comparison_stats(rows: &[ComparisonCase]) -> ComparisonStats {
    let mut stats = ComparisonStats::default();
    let mut keyword_rr = 0.0;
    let mut hybrid_rr = 0.0;
    for row in rows {
        if row.keyword_error.is_some() || row.hybrid_error.is_some() {
            continue;
        }
        stats.paired += 1;
        match (row.keyword_rank.is_some(), row.hybrid_rank.is_some()) {
            (true, true) => stats.both += 1,
            (true, false) => stats.keyword_only += 1,
            (false, true) => stats.hybrid_only += 1,
            (false, false) => stats.neither += 1,
        }
        stats.keyword_hits += usize::from(row.keyword_rank.is_some());
        stats.hybrid_hits += usize::from(row.hybrid_rank.is_some());
        keyword_rr += reciprocal_rank(row.keyword_rank);
        hybrid_rr += reciprocal_rank(row.hybrid_rank);
    }
    if stats.paired > 0 {
        stats.keyword_mrr = keyword_rr / stats.paired as f64;
        stats.hybrid_mrr = hybrid_rr / stats.paired as f64;
    }
    stats
}
